#![allow(dead_code)]

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

const AUTHOR_NAME: &str = "Regret Fixture";
const AUTHOR_EMAIL: &str = "fixture@regret.local";
const BASE_TIME: &str = "2024-01-01T00:00:00Z";
const AMBIGUOUS_PREFIX_LEN: usize = 2;

#[derive(Debug)]
pub struct FixtureMeta {
    pub canonical_culprit: String,
    pub canonical_evidence: String,
    pub manual_culprit: String,
    pub manual_evidence: String,
    pub linked_fix_culprit: String,
    pub linked_fix_evidence: String,
    pub ambiguous_prefix: String,
    pub ambiguous_culprit_a: String,
    pub ambiguous_culprit_b: String,
    pub ambiguous_evidence: String,
    pub crlf_commit: String,
    pub rewrite_base: String,
    pub rewrite_head: String,
    pub main_head: String,
}

#[derive(Debug)]
pub struct FixtureRepo {
    #[allow(dead_code)]
    pub dir: TempDir,
    pub path: PathBuf,
    pub meta: FixtureMeta,
    base_env: Vec<(String, String)>,
}

impl FixtureRepo {
    pub fn git(&self, args: &[&str]) -> Result<String> {
        run_git(&self.path, &self.base_env, args)
    }
}

pub fn build_fixture_repo() -> Result<FixtureRepo> {
    let dir = TempDir::new().context("create temp dir")?;
    let repo_path = dir.path().join("repo");
    fs::create_dir_all(&repo_path).context("create repo dir")?;

    let empty_gitconfig = dir.path().join("empty_gitconfig");
    fs::write(&empty_gitconfig, "").context("create empty gitconfig")?;

    let base_env = base_env(&empty_gitconfig);

    run_git(&repo_path, &base_env, &["init", "-b", "main"])?;
    run_git(&repo_path, &base_env, &["config", "user.name", AUTHOR_NAME])?;
    run_git(
        &repo_path,
        &base_env,
        &["config", "user.email", AUTHOR_EMAIL],
    )?;
    run_git(&repo_path, &base_env, &["config", "core.autocrlf", "false"])?;
    run_git(
        &repo_path,
        &base_env,
        &["config", "commit.gpgsign", "false"],
    )?;

    let base_time = DateTime::parse_from_rfc3339(BASE_TIME)
        .context("parse base time")?
        .with_timezone(&Utc);
    let mut minutes = 0i64;

    let _sha_a = commit_file(
        &repo_path,
        &base_env,
        "file.txt",
        b"one\n",
        "Add base\n",
        next_time(base_time, &mut minutes),
        false,
    )?;

    let sha_b = commit_file(
        &repo_path,
        &base_env,
        "file.txt",
        b"two\n",
        "Change feature B\n",
        next_time(base_time, &mut minutes),
        false,
    )?;

    let canonical_message = format!(
        "Revert \"Change feature B\"\n\nThis reverts commit {}.\n",
        sha_b
    );
    let sha_c = commit_file(
        &repo_path,
        &base_env,
        "file.txt",
        b"one\n",
        &canonical_message,
        next_time(base_time, &mut minutes),
        false,
    )?;

    let sha_d = commit_file(
        &repo_path,
        &base_env,
        "file.txt",
        b"three\n",
        "Introduce bug\n",
        next_time(base_time, &mut minutes),
        false,
    )?;

    let sha_e = commit_file(
        &repo_path,
        &base_env,
        "file.txt",
        b"one\n",
        "manual revert: fix three\n",
        next_time(base_time, &mut minutes),
        false,
    )?;

    let sha_f = commit_file(
        &repo_path,
        &base_env,
        "file.txt",
        b"bad\n",
        "Bad change\n",
        next_time(base_time, &mut minutes),
        false,
    )?;

    let linked_fix_message = format!("Fix bug\n\nFixes-Commit: {}\n", sha_f);
    let sha_g = commit_file(
        &repo_path,
        &base_env,
        "file.txt",
        b"good\n",
        &linked_fix_message,
        next_time(base_time, &mut minutes),
        false,
    )?;

    let (ambiguous_prefix, ambiguous_a, ambiguous_b) =
        find_ambiguous_prefix(&repo_path, &base_env, base_time, &mut minutes)?;

    let ambiguous_message = format!(
        "Ambiguous fix trailer\n\nFixes-Commit: {}\n",
        ambiguous_prefix
    );
    let ambiguous_evidence = commit_allow_empty(
        &repo_path,
        &base_env,
        &ambiguous_message,
        next_time(base_time, &mut minutes),
    )?;

    let crlf_commit = commit_file(
        &repo_path,
        &base_env,
        "crlf.txt",
        b"line1\r\nline2\r\n",
        "Add CRLF file\n",
        next_time(base_time, &mut minutes),
        false,
    )?;

    let rewrite_base = sha_d.clone();
    run_git(
        &repo_path,
        &base_env,
        &["checkout", "-b", "rewrite-branch", &rewrite_base],
    )?;

    let rewrite_head = commit_allow_empty(
        &repo_path,
        &base_env,
        "Rewrite history\n",
        next_time(base_time, &mut minutes),
    )?;

    run_git(&repo_path, &base_env, &["checkout", "main"])?;
    let main_head = run_git(&repo_path, &base_env, &["rev-parse", "HEAD"])?;

    let meta = FixtureMeta {
        canonical_culprit: sha_b,
        canonical_evidence: sha_c,
        manual_culprit: sha_d,
        manual_evidence: sha_e,
        linked_fix_culprit: sha_f,
        linked_fix_evidence: sha_g,
        ambiguous_prefix,
        ambiguous_culprit_a: ambiguous_a,
        ambiguous_culprit_b: ambiguous_b,
        ambiguous_evidence,
        crlf_commit,
        rewrite_base,
        rewrite_head,
        main_head,
    };

    Ok(FixtureRepo {
        dir,
        path: repo_path,
        meta,
        base_env,
    })
}

fn base_env(empty_gitconfig: &Path) -> Vec<(String, String)> {
    vec![
        ("TZ".to_string(), "UTC".to_string()),
        ("LC_ALL".to_string(), "C".to_string()),
        ("LANG".to_string(), "C".to_string()),
        ("NO_COLOR".to_string(), "1".to_string()),
        ("GIT_CONFIG_NOSYSTEM".to_string(), "1".to_string()),
        (
            "GIT_CONFIG_GLOBAL".to_string(),
            empty_gitconfig.to_string_lossy().to_string(),
        ),
    ]
}

fn next_time(base: DateTime<Utc>, minutes: &mut i64) -> String {
    let stamp = base + ChronoDuration::minutes(*minutes);
    *minutes += 5;
    stamp.to_rfc3339()
}

fn find_ambiguous_prefix(
    repo_path: &Path,
    base_env: &[(String, String)],
    base_time: DateTime<Utc>,
    minutes: &mut i64,
) -> Result<(String, String, String)> {
    let mut prefixes: HashMap<String, String> = HashMap::new();

    for i in 0..512 {
        let message = format!("Ambiguous seed {}\n", i);
        let sha = commit_allow_empty(repo_path, base_env, &message, next_time(base_time, minutes))?;
        let prefix = sha[..AMBIGUOUS_PREFIX_LEN].to_string();
        if let Some(existing) = prefixes.get(&prefix) {
            return Ok((prefix, existing.clone(), sha));
        }
        prefixes.insert(prefix, sha);
    }

    bail!("unable to generate ambiguous SHA prefix")
}

fn commit_file(
    repo_path: &Path,
    base_env: &[(String, String)],
    rel_path: &str,
    contents: &[u8],
    message: &str,
    timestamp: String,
    allow_empty: bool,
) -> Result<String> {
    let path = repo_path.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create parent directory")?;
    }
    fs::write(&path, contents).context("write file contents")?;

    run_git(repo_path, base_env, &["add", rel_path])?;
    commit(repo_path, base_env, message, timestamp, allow_empty)
}

fn commit_allow_empty(
    repo_path: &Path,
    base_env: &[(String, String)],
    message: &str,
    timestamp: String,
) -> Result<String> {
    commit(repo_path, base_env, message, timestamp, true)
}

fn commit(
    repo_path: &Path,
    base_env: &[(String, String)],
    message: &str,
    timestamp: String,
    allow_empty: bool,
) -> Result<String> {
    let message_path = repo_path.join(".git").join("fixture-message.txt");
    fs::write(&message_path, message).context("write commit message")?;

    let mut command = Command::new("git");
    command
        .current_dir(repo_path)
        .envs(base_env.iter().map(|(k, v)| (k, v)))
        .env("GIT_AUTHOR_DATE", &timestamp)
        .env("GIT_COMMITTER_DATE", &timestamp)
        .args(["commit", "-F"])
        .arg(&message_path);

    if allow_empty {
        command.arg("--allow-empty");
    }

    let output = command.output().context("run git commit")?;
    if !output.status.success() {
        bail!(
            "git commit failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    run_git(repo_path, base_env, &["rev-parse", "HEAD"])
}

fn run_git(repo_path: &Path, base_env: &[(String, String)], args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(repo_path)
        .envs(base_env.iter().map(|(k, v)| (k, v)))
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;

    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
