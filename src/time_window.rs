use crate::fast_path;
use crate::DateSpec;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use git2::Repository;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UntilSource {
    Flag,
    GithubActionsHeadCommitterTime,
    WallClock,
}

impl UntilSource {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            UntilSource::Flag => "flag",
            UntilSource::GithubActionsHeadCommitterTime => "github_actions_head_committer_time",
            UntilSource::WallClock => "wall_clock",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct UntilInfo {
    pub(crate) until: DateTime<Utc>,
    pub(crate) source: UntilSource,
}

pub(crate) fn compute_until(
    until: &Option<DateSpec>,
    repo_root: &Path,
    selected_ref: &str,
) -> Result<UntilInfo> {
    if let Some(spec) = until {
        return Ok(UntilInfo {
            until: date_spec_to_utc(spec)?,
            source: UntilSource::Flag,
        });
    }

    if std::env::var("GITHUB_ACTIONS")
        .map(|value| value == "true")
        .unwrap_or(false)
    {
        let repo = Repository::discover(repo_root).with_context(|| {
            format!(
                "error: unable to open git repository {}",
                repo_root.display()
            )
        })?;
        let oid = fast_path::resolve_ref_oid(&repo, selected_ref)?
            .ok_or_else(|| anyhow!("error: unable to resolve ref {}", selected_ref))?;
        let commit = repo.find_commit(oid)?;
        let seconds = commit.time().seconds();
        let until = DateTime::<Utc>::from_timestamp(seconds, 0)
            .ok_or_else(|| anyhow!("error: invalid committer time {}", seconds))?;
        return Ok(UntilInfo {
            until,
            source: UntilSource::GithubActionsHeadCommitterTime,
        });
    }

    Ok(UntilInfo {
        until: Utc::now(),
        source: UntilSource::WallClock,
    })
}

fn date_spec_to_utc(spec: &DateSpec) -> Result<DateTime<Utc>> {
    match spec {
        DateSpec::Rfc3339(dt) => Ok(*dt),
        DateSpec::Iso8601(value) => {
            let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")?;
            let dt = date
                .and_hms_opt(0, 0, 0)
                .ok_or_else(|| anyhow!("error: invalid date {}", value))?;
            Ok(DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn run_git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git command failed");
        assert!(output.status.success(), "git {:?} failed", args);
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_repo(path: &Path) {
        run_git(path, &["init"]);
        run_git(path, &["config", "user.email", "test@example.com"]);
        run_git(path, &["config", "user.name", "Test User"]);
        std::fs::write(path.join("README.md"), "hello").unwrap();
        run_git(path, &["add", "."]);
        run_git(
            path,
            &[
                "-c",
                "user.name=Test User",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-m",
                "init",
                "--date",
                "2020-01-02T03:04:05Z",
            ],
        );
        run_git(path, &["branch", "-m", "main"]);
    }

    #[test]
    fn compute_until_prefers_flag() {
        let temp = tempdir().unwrap();
        init_repo(temp.path());
        let spec = DateSpec::Iso8601("2024-01-15".to_string());
        let info = compute_until(&Some(spec), temp.path(), "refs/heads/main").unwrap();
        assert_eq!(info.source, UntilSource::Flag);
        assert_eq!(
            info.until.date_naive(),
            NaiveDate::from_ymd_opt(2024, 1, 15).unwrap()
        );
    }

    #[test]
    fn compute_until_uses_github_actions_head_time() {
        let _guard = ENV_LOCK.lock().unwrap();
        let prev = std::env::var("GITHUB_ACTIONS").ok();
        std::env::set_var("GITHUB_ACTIONS", "true");

        let temp = tempdir().unwrap();
        init_repo(temp.path());

        let repo = Repository::discover(temp.path()).unwrap();
        let oid = fast_path::resolve_ref_oid(&repo, "refs/heads/main")
            .unwrap()
            .unwrap();
        let commit = repo.find_commit(oid).unwrap();
        let expected = DateTime::<Utc>::from_timestamp(commit.time().seconds(), 0).unwrap();

        let info = compute_until(&None, temp.path(), "refs/heads/main").unwrap();
        assert_eq!(info.source, UntilSource::GithubActionsHeadCommitterTime);
        assert_eq!(info.until, expected);

        if let Some(value) = prev {
            std::env::set_var("GITHUB_ACTIONS", value);
        } else {
            std::env::remove_var("GITHUB_ACTIONS");
        }
    }
}
