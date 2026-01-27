use anyhow::{anyhow, Context, Result};
use git2::{Oid, Repository};
use std::path::Path;

#[allow(dead_code)]
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

#[allow(dead_code)]
pub(crate) fn shallow_boundary_oids(repo_root: &Path) -> Result<Vec<Oid>> {
    let repo = Repository::discover(repo_root).with_context(|| {
        format!(
            "error: unable to open git repository {}",
            repo_root.display()
        )
    })?;
    shallow_boundary_oids_from_repo(&repo)
}

#[allow(dead_code)]
pub(crate) fn shallow_boundary_oids_from_repo(repo: &Repository) -> Result<Vec<Oid>> {
    let shallow_path = repo.path().join("shallow");
    if !shallow_path.exists() {
        return Ok(Vec::new());
    }

    let contents = std::fs::read_to_string(&shallow_path).with_context(|| {
        format!(
            "error: unable to read shallow file {}",
            shallow_path.display()
        )
    })?;

    let mut oids = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        let value = line.trim();
        if value.is_empty() {
            continue;
        }
        let oid = Oid::from_str(value).map_err(|_| {
            anyhow!(
                "error: invalid shallow oid '{}' at line {}",
                value,
                index + 1
            )
        })?;
        oids.push(oid);
    }

    Ok(oids)
}
