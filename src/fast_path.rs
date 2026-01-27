use crate::store::Store;
use anyhow::{anyhow, Context, Result};
use git2::{Oid, Repository};
use std::path::Path;

const META_REF_NAME: &str = "ref_name";
const META_LAST_SCANNED_REF_OID: &str = "last_scanned_ref_oid";
const META_LAST_SCANNED_GRAPH_TIP: &str = "last_scanned_graph_tip";
const META_CACHE_VALID: &str = "cache_valid";
const META_COVERAGE_VALID: &str = "coverage_valid";

pub(crate) fn ensure_meta_defaults(store: &Store, ref_name: &str) -> Result<()> {
    if store.get_meta_value(META_REF_NAME)?.is_none() {
        store.set_meta_value(META_REF_NAME, ref_name)?;
    }

    if store.get_meta_bool(META_CACHE_VALID)?.is_none() {
        store.set_meta_bool(META_CACHE_VALID, false)?;
    }

    if store.get_meta_bool(META_COVERAGE_VALID)?.is_none() {
        store.set_meta_bool(META_COVERAGE_VALID, false)?;
    }

    Ok(())
}

pub(crate) fn should_skip_scan(repo_root: &Path, store: &Store, ref_name: &str) -> Result<bool> {
    let cache_valid = store.get_meta_bool(META_CACHE_VALID)?.unwrap_or(false);
    let coverage_valid = store.get_meta_bool(META_COVERAGE_VALID)?.unwrap_or(false);

    if !cache_valid || !coverage_valid {
        return Ok(false);
    }

    let last_scanned_ref_oid = match store.get_meta_value(META_LAST_SCANNED_REF_OID)? {
        Some(value) => value,
        None => return Ok(false),
    };

    let repo = Repository::discover(repo_root).with_context(|| {
        format!(
            "error: unable to open git repository {}",
            repo_root.display()
        )
    })?;
    let current_oid = resolve_ref_oid(&repo, ref_name)?;

    Ok(current_oid
        .map(|oid| oid.to_string() == last_scanned_ref_oid)
        .unwrap_or(false))
}

pub(crate) fn resolve_ref_oid(repo: &Repository, ref_name: &str) -> Result<Option<Oid>> {
    if ref_name == "HEAD" {
        let head = repo.head().context("error: unable to resolve HEAD")?;
        return Ok(head.target());
    }

    let reference = match repo.find_reference(ref_name) {
        Ok(reference) => reference,
        Err(_) => return Ok(None),
    };

    let resolved = reference
        .resolve()
        .map_err(|e| anyhow!("error: unable to resolve ref {}: {}", ref_name, e))?;

    Ok(resolved.target())
}

pub(crate) fn set_last_scanned_oids(store: &Store, ref_oid: &str, graph_tip: &str) -> Result<()> {
    store.set_meta_value(META_LAST_SCANNED_REF_OID, ref_oid)?;
    store.set_meta_value(META_LAST_SCANNED_GRAPH_TIP, graph_tip)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selected_branch;
    use crate::store::Store;
    use std::process::Command;
    use tempfile::tempdir;

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
    fn fast_path_skips_when_head_unchanged() {
        let temp = tempdir().unwrap();
        init_repo(temp.path());

        let cache_dir = temp.path().join(".regret");
        let store = Store::open(&cache_dir).unwrap();
        let selected = selected_branch::ensure_selected_branch(temp.path(), &store).unwrap();
        ensure_meta_defaults(&store, &selected).unwrap();

        let repo = Repository::discover(temp.path()).unwrap();
        let current = resolve_ref_oid(&repo, &selected).unwrap().unwrap();
        store
            .set_meta_value(META_LAST_SCANNED_REF_OID, &current.to_string())
            .unwrap();
        store.set_meta_bool(META_CACHE_VALID, true).unwrap();
        store.set_meta_bool(META_COVERAGE_VALID, true).unwrap();

        let skip = should_skip_scan(temp.path(), &store, &selected).unwrap();
        assert!(skip);
    }

    #[test]
    fn fast_path_runs_when_head_changes() {
        let temp = tempdir().unwrap();
        init_repo(temp.path());

        let cache_dir = temp.path().join(".regret");
        let store = Store::open(&cache_dir).unwrap();
        let selected = selected_branch::ensure_selected_branch(temp.path(), &store).unwrap();
        ensure_meta_defaults(&store, &selected).unwrap();

        let repo = Repository::discover(temp.path()).unwrap();
        let current = resolve_ref_oid(&repo, &selected).unwrap().unwrap();
        store
            .set_meta_value(META_LAST_SCANNED_REF_OID, &current.to_string())
            .unwrap();
        store.set_meta_bool(META_CACHE_VALID, true).unwrap();
        store.set_meta_bool(META_COVERAGE_VALID, true).unwrap();

        std::fs::write(temp.path().join("CHANGELOG.md"), "change").unwrap();
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["commit", "-m", "change"]);

        let skip = should_skip_scan(temp.path(), &store, &selected).unwrap();
        assert!(!skip);
    }

    #[test]
    fn fast_path_requires_valid_flags() {
        let temp = tempdir().unwrap();
        init_repo(temp.path());

        let cache_dir = temp.path().join(".regret");
        let store = Store::open(&cache_dir).unwrap();
        let selected = selected_branch::ensure_selected_branch(temp.path(), &store).unwrap();
        ensure_meta_defaults(&store, &selected).unwrap();

        let repo = Repository::discover(temp.path()).unwrap();
        let current = resolve_ref_oid(&repo, &selected).unwrap().unwrap();
        store
            .set_meta_value(META_LAST_SCANNED_REF_OID, &current.to_string())
            .unwrap();
        store.set_meta_bool(META_CACHE_VALID, false).unwrap();
        store.set_meta_bool(META_COVERAGE_VALID, true).unwrap();

        let skip = should_skip_scan(temp.path(), &store, &selected).unwrap();
        assert!(!skip);
    }
}
