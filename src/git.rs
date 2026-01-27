use crate::store::Store;
use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::Command;

const META_SELECTED_BRANCH: &str = "selected_branch";

pub(crate) fn resolve_selected_branch(store: &Store, repo_root: &Path) -> Result<String> {
    if let Some(value) = store.get_meta(META_SELECTED_BRANCH)? {
        return Ok(value);
    }

    let detected = detect_default_branch(repo_root)?;
    store.set_meta(META_SELECTED_BRANCH, &detected)?;
    Ok(detected)
}

pub(crate) fn detect_default_branch(repo_root: &Path) -> Result<String> {
    if let Some(origin_head) = symbolic_ref(repo_root, "refs/remotes/origin/HEAD")? {
        return Ok(origin_head);
    }

    for candidate in ["refs/heads/main", "refs/heads/master"] {
        if ref_exists(repo_root, candidate)? {
            return Ok(candidate.to_string());
        }
    }

    if let Some(head_ref) = head_ref(repo_root)? {
        return Ok(head_ref);
    }

    bail!("error: unable to resolve selected branch");
}

fn symbolic_ref(repo_root: &Path, reference: &str) -> Result<Option<String>> {
    let output = git(repo_root, &["symbolic-ref", "-q", reference])?;
    Ok(output)
}

fn ref_exists(repo_root: &Path, reference: &str) -> Result<bool> {
    let status = Command::new("git")
        .current_dir(repo_root)
        .args(["show-ref", "--verify", "--quiet", reference])
        .status()
        .with_context(|| format!("run git show-ref {}", reference))?;
    Ok(status.success())
}

fn head_ref(repo_root: &Path) -> Result<Option<String>> {
    if let Some(reference) = symbolic_ref(repo_root, "HEAD")? {
        return Ok(Some(reference));
    }

    let output = git(repo_root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if let Some(name) = output {
        if name != "HEAD" {
            return Ok(Some(format!("refs/heads/{}", name)));
        }
    }

    Ok(None)
}

fn git(repo_root: &Path, args: &[&str]) -> Result<Option<String>> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .args(args)
        .output()
        .with_context(|| format!("run git {}", args.join(" ")))?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        Ok(None)
    } else {
        Ok(Some(stdout))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::fs;
    use tempfile::tempdir;

    fn init_repo(path: &Path, branch: &str) -> Result<()> {
        let status = Command::new("git")
            .current_dir(path)
            .args(["init", "-b", branch])
            .status();

        if status.is_err() || !status.unwrap().success() {
            Command::new("git")
                .current_dir(path)
                .args(["-c", &format!("init.defaultBranch={}", branch), "init"])
                .status()
                .context("git init")?;
        }

        Command::new("git")
            .current_dir(path)
            .args(["config", "user.name", "Fixture User"])
            .status()
            .ok();
        Command::new("git")
            .current_dir(path)
            .args(["config", "user.email", "fixture@example.com"])
            .status()
            .ok();

        fs::write(path.join("README.md"), "fixture").context("write file")?;
        Command::new("git")
            .current_dir(path)
            .args(["add", "README.md"])
            .status()
            .context("git add")?;
        Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "init"])
            .env("GIT_AUTHOR_DATE", "2024-01-01T00:00:00Z")
            .env("GIT_COMMITTER_DATE", "2024-01-01T00:00:00Z")
            .status()
            .context("git commit")?;
        Ok(())
    }

    #[test]
    fn detects_origin_head_first() {
        let temp = tempdir().unwrap();
        init_repo(temp.path(), "main").unwrap();

        let head = git(temp.path(), &["rev-parse", "HEAD"])
            .unwrap()
            .unwrap();
        Command::new("git")
            .current_dir(temp.path())
            .args(["update-ref", "refs/remotes/origin/main", &head])
            .status()
            .unwrap();
        Command::new("git")
            .current_dir(temp.path())
            .args(["symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main"])
            .status()
            .unwrap();

        let detected = detect_default_branch(temp.path()).unwrap();
        assert_eq!(detected, "refs/remotes/origin/main");
    }

    #[test]
    fn detects_main_when_origin_missing() {
        let temp = tempdir().unwrap();
        init_repo(temp.path(), "main").unwrap();
        let detected = detect_default_branch(temp.path()).unwrap();
        assert_eq!(detected, "refs/heads/main");
    }

    #[test]
    fn detects_master_when_main_missing() {
        let temp = tempdir().unwrap();
        init_repo(temp.path(), "master").unwrap();

        let detected = detect_default_branch(temp.path()).unwrap();
        assert_eq!(detected, "refs/heads/master");
    }

    #[test]
    fn resolve_selected_branch_uses_persisted_value() {
        let temp = tempdir().unwrap();
        init_repo(temp.path(), "main").unwrap();
        let cache_dir = temp.path().join(".regret");
        let store = Store::open(&cache_dir).unwrap();

        store
            .set_meta(META_SELECTED_BRANCH, "refs/heads/persisted")
            .unwrap();

        let resolved = resolve_selected_branch(&store, temp.path()).unwrap();
        assert_eq!(resolved, "refs/heads/persisted");
    }
}
