use crate::fast_path;
use crate::store::{CommitRow, Store};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use git2::{Oid, Repository, Sort};
use std::path::Path;

const SUBJECT_TRUNCATE_LEN: usize = 80;
const META_LAST_SCANNED_GRAPH_TIP: &str = "last_scanned_graph_tip";
const META_CACHE_VALID: &str = "cache_valid";
const META_COVERAGE_VALID: &str = "coverage_valid";

pub(crate) struct ScanSummary {
    pub(crate) new_commits: usize,
    #[allow(dead_code)]
    pub(crate) ref_oid: String,
}

pub(crate) fn incremental_scan(
    repo_root: &Path,
    store: &mut Store,
    ref_name: &str,
) -> Result<ScanSummary> {
    let repo = Repository::discover(repo_root).with_context(|| {
        format!(
            "error: unable to open git repository {}",
            repo_root.display()
        )
    })?;
    let current_oid = fast_path::resolve_ref_oid(&repo, ref_name)?
        .ok_or_else(|| anyhow!("error: unable to resolve ref {}", ref_name))?;

    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)?;
    revwalk.push(current_oid)?;

    if let Some(last_tip) = store.get_meta_value(META_LAST_SCANNED_GRAPH_TIP)? {
        let last_oid = Oid::from_str(&last_tip)
            .map_err(|_| anyhow!("error: invalid last_scanned_graph_tip {}", last_tip))?;
        revwalk.hide(last_oid).ok();
    }

    let mut rows: Vec<CommitRow> = Vec::new();
    for oid_result in revwalk {
        let oid = oid_result?;
        let commit = repo.find_commit(oid)?;

        let time_utc = commit_time_utc(&commit)?;
        let subject = commit.summary().map(truncate_subject);
        let pr_number = subject.as_ref().and_then(|s| parse_pr_number(s));
        let pr_source = pr_number.as_ref().map(|_| "merge_commit".to_string());

        rows.push(CommitRow {
            sha: oid.to_string(),
            time_utc,
            subject,
            pr_number,
            pr_source,
        });
    }

    store.upsert_commits(&rows)?;
    fast_path::set_last_scanned_oids(store, &current_oid.to_string(), &current_oid.to_string())?;
    store.set_meta_bool(META_CACHE_VALID, true)?;
    store.set_meta_bool(META_COVERAGE_VALID, true)?;

    Ok(ScanSummary {
        new_commits: rows.len(),
        ref_oid: current_oid.to_string(),
    })
}

fn commit_time_utc(commit: &git2::Commit<'_>) -> Result<String> {
    let time = commit.time();
    let seconds = time.seconds();
    let dt = DateTime::<Utc>::from_timestamp(seconds, 0)
        .ok_or_else(|| anyhow!("error: invalid commit timestamp {}", seconds))?;
    Ok(dt.to_rfc3339())
}

fn truncate_subject(subject: &str) -> String {
    subject.chars().take(SUBJECT_TRUNCATE_LEN).collect()
}

fn parse_pr_number(subject: &str) -> Option<i64> {
    let prefix = "Merge pull request #";
    let rest = subject.strip_prefix(prefix)?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selected_branch;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::tempdir;

    fn real_path(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

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
        run_git(path, &["commit", "-m", "init"]);
        run_git(path, &["branch", "-m", "main"]);
    }

    #[test]
    fn incremental_scan_processes_new_commits_only() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        init_repo(&base);

        let cache_dir = base.join(".regret");
        let mut store = Store::open(&cache_dir).unwrap();
        let selected = selected_branch::ensure_selected_branch(&base, &store).unwrap();

        let first = incremental_scan(&base, &mut store, &selected).unwrap();
        assert!(first.new_commits > 0);

        let second = incremental_scan(&base, &mut store, &selected).unwrap();
        assert_eq!(second.new_commits, 0);

        std::fs::write(base.join("CHANGELOG.md"), "change").unwrap();
        run_git(&base, &["add", "."]);
        run_git(&base, &["commit", "-m", "change"]);

        let third = incremental_scan(&base, &mut store, &selected).unwrap();
        assert_eq!(third.new_commits, 1);
    }
}
