#![allow(dead_code)]

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const BASE_ENV: &[(&str, &str)] = &[
    ("TZ", "UTC"),
    ("LC_ALL", "C"),
    ("LANG", "C"),
    ("NO_COLOR", "1"),
    ("GIT_CONFIG_NOSYSTEM", "1"),
];

pub struct FixtureRepo {
    dir: TempDir,
}

impl FixtureRepo {
    pub fn new() -> Result<Self> {
        let dir = tempfile::tempdir().context("create temp dir")?;
        let repo = Self { dir };
        repo.git_init()?;
        repo.git(&["config", "user.name", "Fixture User"], &[])?;
        repo.git(&["config", "user.email", "fixture@example.com"], &[])?;
        repo.git(&["config", "commit.gpgsign", "false"], &[])?;
        repo.git(&["config", "core.autocrlf", "false"], &[])?;
        repo.git(&["config", "core.eol", "lf"], &[])?;
        Ok(repo)
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn commit_file(
        &self,
        relative_path: &str,
        contents: &[u8],
        subject: &str,
        body: Option<&str>,
        timestamp: &str,
    ) -> Result<String> {
        let path = self.path().join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("create parent dirs")?;
        }
        fs::write(&path, contents).context("write fixture file")?;
        self.git(&["add", relative_path], &[])?;
        self.commit_message(subject, body, timestamp)?;
        self.git(&["rev-parse", "HEAD"], &[])
    }

    pub fn commit_message(&self, subject: &str, body: Option<&str>, timestamp: &str) -> Result<()> {
        let mut args = vec!["commit", "-m", subject];
        if let Some(body) = body {
            args.push("-m");
            args.push(body);
        }

        self.git(
            &args,
            &[
                ("GIT_AUTHOR_DATE", timestamp),
                ("GIT_COMMITTER_DATE", timestamp),
            ],
        )?;
        Ok(())
    }

    pub fn rev_parse(&self, rev: &str) -> Result<String> {
        self.git(&["rev-parse", rev], &[])
    }

    pub fn revert_commit(&self, sha: &str, timestamp: &str) -> Result<String> {
        self.git(
            &["revert", "--no-edit", sha],
            &[
                ("GIT_AUTHOR_DATE", timestamp),
                ("GIT_COMMITTER_DATE", timestamp),
            ],
        )?;
        self.rev_parse("HEAD")
    }

    pub fn update_ref(&self, reference: &str, sha: &str) -> Result<()> {
        self.git(&["update-ref", reference, sha], &[])?;
        Ok(())
    }

    pub fn checkout_force(&self, reference: &str) -> Result<()> {
        self.git(&["checkout", "-f", reference], &[])?;
        Ok(())
    }

    fn git(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Result<String> {
        let output = self
            .git_command(args, extra_env)?
            .output()
            .with_context(|| format!("run git {}", args.join(" ")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git {} failed: {}", args.join(" "), stderr.trim());
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn git_command(&self, args: &[&str], extra_env: &[(&str, &str)]) -> Result<Command> {
        let mut cmd = Command::new("git");
        cmd.current_dir(self.path());
        cmd.args(args);

        for (key, value) in BASE_ENV.iter() {
            cmd.env(key, value);
        }

        for (key, value) in extra_env.iter() {
            cmd.env(key, value);
        }

        Ok(cmd)
    }

    fn git_init(&self) -> Result<()> {
        if self.try_git_init_with_branch("main").is_ok() {
            return Ok(());
        }

        self.git(&["init"], &[])?;
        self.git(&["checkout", "-b", "main"], &[])?;
        Ok(())
    }

    fn try_git_init_with_branch(&self, branch: &str) -> Result<()> {
        let output = self
            .git_command(&["init", "-b", branch], &[])?
            .output()
            .context("run git init -b")?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("git init -b failed: {}", stderr.trim());
        }
    }
}

pub struct Scenario {
    pub repo: FixtureRepo,
    pub culprit_sha: String,
    pub evidence_sha: String,
}

pub struct RewriteScenario {
    pub repo: FixtureRepo,
    pub original_head: String,
    pub rewritten_head: String,
    pub orphaned_sha: String,
}

pub struct CrlfScenario {
    pub repo: FixtureRepo,
    pub commit_sha: String,
}

pub fn canonical_revert() -> Result<Scenario> {
    let repo = FixtureRepo::new()?;
    let culprit = repo.commit_file(
        "src/canonical.txt",
        b"canonical\n",
        "Add canonical file",
        None,
        "2024-01-01T00:00:00Z",
    )?;
    let evidence = repo.revert_commit(&culprit, "2024-01-01T01:00:00Z")?;

    Ok(Scenario {
        repo,
        culprit_sha: culprit,
        evidence_sha: evidence,
    })
}

pub fn manual_revert() -> Result<Scenario> {
    let repo = FixtureRepo::new()?;
    let culprit = repo.commit_file(
        "src/manual.txt",
        b"manual\n",
        "Add manual file",
        None,
        "2024-02-01T00:00:00Z",
    )?;
    let evidence = repo.commit_file(
        "src/manual.txt",
        b"",
        "Manual rollback of manual file",
        None,
        "2024-02-01T01:00:00Z",
    )?;

    Ok(Scenario {
        repo,
        culprit_sha: culprit,
        evidence_sha: evidence,
    })
}

pub fn linked_fix_unique() -> Result<Scenario> {
    let repo = FixtureRepo::new()?;
    let culprit = repo.commit_file(
        "src/linked.txt",
        b"linked\n",
        "Introduce linked fix target",
        None,
        "2024-03-01T00:00:00Z",
    )?;
    let body = format!("Fixes-Commit: {}", culprit);
    let evidence = repo.commit_file(
        "src/linked.txt",
        b"linked-fixed\n",
        "Fix linked issue",
        Some(&body),
        "2024-03-01T02:00:00Z",
    )?;

    Ok(Scenario {
        repo,
        culprit_sha: culprit,
        evidence_sha: evidence,
    })
}

pub fn linked_fix_ambiguous_short() -> Result<Scenario> {
    let repo = FixtureRepo::new()?;
    let culprit = repo.commit_file(
        "src/ambiguous.txt",
        b"ambiguous\n",
        "Introduce ambiguous change",
        None,
        "2024-04-01T00:00:00Z",
    )?;
    let short_prefix = &culprit[..8];
    let body = format!("Fixes-Commit: {}", short_prefix);
    let evidence = repo.commit_file(
        "src/ambiguous.txt",
        b"ambiguous-fixed\n",
        "Fix ambiguous change (short prefix)",
        Some(&body),
        "2024-04-01T01:00:00Z",
    )?;

    Ok(Scenario {
        repo,
        culprit_sha: culprit,
        evidence_sha: evidence,
    })
}

pub fn rewritten_history() -> Result<RewriteScenario> {
    let repo = FixtureRepo::new()?;
    let first = repo.commit_file(
        "src/rewrite.txt",
        b"v1\n",
        "Rewrite baseline",
        None,
        "2024-05-01T00:00:00Z",
    )?;
    let second = repo.commit_file(
        "src/rewrite.txt",
        b"v2\n",
        "Rewrite change",
        None,
        "2024-05-01T01:00:00Z",
    )?;
    let original_head = repo.rev_parse("HEAD")?;

    repo.update_ref("refs/heads/main", &first)?;
    repo.checkout_force("main")?;
    let rewritten_head = repo.commit_file(
        "src/rewrite.txt",
        b"v3\n",
        "Rewrite after reset",
        None,
        "2024-05-01T02:00:00Z",
    )?;

    Ok(RewriteScenario {
        repo,
        original_head,
        rewritten_head,
        orphaned_sha: second,
    })
}

pub fn crlf_edge_case() -> Result<CrlfScenario> {
    let repo = FixtureRepo::new()?;
    let commit_sha = repo.commit_file(
        "src/crlf.txt",
        b"line1\r\nline2\r\n",
        "Add CRLF file",
        None,
        "2024-06-01T00:00:00Z",
    )?;

    Ok(CrlfScenario { repo, commit_sha })
}

pub fn is_full_sha(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}

pub fn is_short_sha(value: &str) -> bool {
    value.len() < 40 && value.chars().all(|c| c.is_ascii_hexdigit())
}
