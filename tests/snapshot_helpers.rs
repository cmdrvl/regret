#![allow(dead_code)]

use anyhow::{Context, Result};
use assert_cmd::Command;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const BASE_ENV: &[(&str, &str)] = &[
    ("TZ", "UTC"),
    ("LC_ALL", "C"),
    ("LANG", "C"),
    ("NO_COLOR", "1"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

pub struct SnapshotRepo {
    _dir: TempDir,
    pub path: PathBuf,
    pub empty_gitconfig: PathBuf,
}

impl SnapshotRepo {
    pub fn new() -> Result<Self> {
        let dir = TempDir::new().context("create temp dir")?;
        let repo_path = dir.path().join("repo");
        fs::create_dir_all(&repo_path).context("create repo dir")?;

        let empty_gitconfig = dir.path().join("empty_gitconfig");
        fs::write(&empty_gitconfig, "").context("create empty gitconfig")?;

        init_repo(&repo_path, &empty_gitconfig)?;

        Ok(Self {
            _dir: dir,
            path: repo_path,
            empty_gitconfig,
        })
    }

    pub fn cache_dir(&self) -> PathBuf {
        self.path.join(".regret")
    }
}

pub fn base_env(empty_gitconfig: &Path) -> Vec<(String, String)> {
    let mut env = BASE_ENV
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<Vec<_>>();
    env.push((
        "GIT_CONFIG_GLOBAL".to_string(),
        empty_gitconfig.to_string_lossy().to_string(),
    ));
    env
}

pub fn run_regret(repo: &SnapshotRepo, args: &[&str]) -> Result<String> {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("regret"));
    cmd.current_dir(&repo.path);

    for (key, value) in base_env(&repo.empty_gitconfig) {
        cmd.env(key, value);
    }

    cmd.args(args);
    let output = cmd.output().context("run regret")?;
    if !output.status.success() {
        anyhow::bail!("regret failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(stdout)
}

pub fn normalize_output(output: &str, repo: &SnapshotRepo) -> String {
    let mut normalized = output.to_string();
    let repo_path = repo.path.to_string_lossy();
    let cache_dir = repo.cache_dir();
    let cache_path = cache_dir.to_string_lossy();
    normalized = normalized.replace(repo_path.as_ref(), "<REPO>");
    normalized = normalized.replace(cache_path.as_ref(), "<CACHE>");
    normalized = normalized.replace(env!("CARGO_PKG_VERSION"), "<VERSION>");
    normalized
}

fn init_repo(repo_path: &Path, empty_gitconfig: &Path) -> Result<()> {
    let env = base_env(empty_gitconfig);
    run_git(repo_path, &env, &["init", "-b", "main"]).context("git init")?;
    run_git(repo_path, &env, &["config", "user.name", "Regret Snapshot"])?;
    run_git(
        repo_path,
        &env,
        &["config", "user.email", "snapshot@regret.local"],
    )?;
    run_git(repo_path, &env, &["config", "commit.gpgsign", "false"])?;
    run_git(repo_path, &env, &["config", "core.autocrlf", "false"])?;
    Ok(())
}

fn run_git(repo_path: &Path, env: &[(String, String)], args: &[&str]) -> Result<String> {
    let output = std::process::Command::new("git")
        .current_dir(repo_path)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;

    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
