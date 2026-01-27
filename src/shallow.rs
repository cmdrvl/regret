use anyhow::{Context, Result};
use git2::Repository;
use std::path::Path;

pub(crate) fn is_shallow_repo(repo_root: &Path) -> Result<bool> {
    let repo = Repository::discover(repo_root).with_context(|| {
        format!(
            "error: unable to open git repository {}",
            repo_root.display()
        )
    })?;

    if repo.is_shallow() {
        return Ok(true);
    }

    let shallow_path = repo.path().join("shallow");
    if shallow_path.exists() {
        return Ok(true);
    }

    Ok(false)
}
