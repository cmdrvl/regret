use crate::store::Store;
use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::process::Command;

const SELECTED_BRANCH_KEY: &str = "selected_branch";

pub(crate) fn ensure_selected_branch(repo_root: &Path, store: &Store) -> Result<String> {
    if let Some(value) = store.get_meta_value(SELECTED_BRANCH_KEY)? {
        return Ok(value);
    }

    let detected = detect_default_branch(repo_root)?;
    store.set_meta_value(SELECTED_BRANCH_KEY, &detected)?;
    Ok(detected)
}

pub(crate) fn detect_default_branch(repo_root: &Path) -> Result<String> {
    if ref_exists(repo_root, "refs/remotes/origin/HEAD")? {
        if let Some(resolved) = symbolic_ref(repo_root, "refs/remotes/origin/HEAD")? {
            return Ok(resolved);
        }
        return Ok("refs/remotes/origin/HEAD".to_string());
    }

    if ref_exists(repo_root, "refs/heads/main")? {
        return Ok("refs/heads/main".to_string());
    }

    if ref_exists(repo_root, "refs/heads/master")? {
        return Ok("refs/heads/master".to_string());
    }

    if let Some(head) = symbolic_ref(repo_root, "HEAD")? {
        return Ok(head);
    }

    Ok("HEAD".to_string())
}

fn ref_exists(repo_root: &Path, reference: &str) -> Result<bool> {
    let output = Command::new("git")
        .args(["show-ref", "--verify", "--quiet", reference])
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("error: unable to run git show-ref {}", reference))?;

    Ok(output.status.success())
}

fn symbolic_ref(repo_root: &Path, reference: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["symbolic-ref", "-q", reference])
        .current_dir(repo_root)
        .output()
        .with_context(|| format!("error: unable to resolve symbolic ref {}", reference))?;

    if !output.status.success() {
        return Ok(None);
    }

    let value = String::from_utf8(output.stdout)
        .map_err(|_| anyhow!("error: invalid utf8 in symbolic-ref output"))?
        .trim()
        .to_string();

    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
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
    }

    #[test]
    fn detect_prefers_origin_head() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        init_repo(&base);

        let sha = run_git(&base, &["rev-parse", "HEAD"]);
        run_git(&base, &["update-ref", "refs/remotes/origin/main", &sha]);
        run_git(
            &base,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );

        let branch = detect_default_branch(&base).unwrap();
        assert_eq!(branch, "refs/remotes/origin/main");
    }

    #[test]
    fn detect_falls_back_to_main() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        init_repo(&base);
        run_git(&base, &["branch", "-m", "main"]);

        let branch = detect_default_branch(&base).unwrap();
        assert_eq!(branch, "refs/heads/main");
    }

    #[test]
    fn detect_falls_back_to_master() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        init_repo(&base);
        run_git(&base, &["branch", "-m", "master"]);

        let branch = detect_default_branch(&base).unwrap();
        assert_eq!(branch, "refs/heads/master");
    }

    #[test]
    fn detect_uses_current_head_branch() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        init_repo(&base);
        run_git(&base, &["branch", "-m", "dev"]);

        let branch = detect_default_branch(&base).unwrap();
        assert_eq!(branch, "refs/heads/dev");
    }

    #[test]
    fn ensure_selected_branch_persists() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        init_repo(&base);
        run_git(&base, &["branch", "-m", "main"]);

        let cache_dir = base.join(".regret");
        let store = Store::open(&cache_dir).unwrap();

        let first = ensure_selected_branch(&base, &store).unwrap();
        assert_eq!(first, "refs/heads/main");

        let sha = run_git(&base, &["rev-parse", "HEAD"]);
        run_git(&base, &["update-ref", "refs/remotes/origin/main", &sha]);
        run_git(
            &base,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );

        let second = ensure_selected_branch(&base, &store).unwrap();
        assert_eq!(second, "refs/heads/main");
    }
}
