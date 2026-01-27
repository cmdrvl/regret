#![cfg_attr(not(test), allow(dead_code))]

use anyhow::{bail, Context, Result};
use std::fs::{self, DirBuilder, OpenOptions};
use std::path::{Path, PathBuf};

const SYMLINK_ERROR: &str = "error: .regret/ contains a symlink component; refusing to write";

pub(crate) fn validate_cache_path(path: &Path) -> Result<()> {
    let mut cursor = PathBuf::new();

    for component in path.components() {
        cursor.push(component);
        if let Ok(metadata) = fs::symlink_metadata(&cursor) {
            if metadata.file_type().is_symlink() {
                bail!(SYMLINK_ERROR);
            }
        }
    }

    Ok(())
}

pub(crate) fn ensure_cache_dir(path: &Path) -> Result<()> {
    validate_cache_path(path)?;
    create_dir_secure(path)?;
    Ok(())
}

pub(crate) fn create_file_secure(path: &Path) -> Result<fs::File> {
    if let Some(parent) = path.parent() {
        ensure_cache_dir(parent)?;
    } else {
        validate_cache_path(path)?;
    }

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }

    options
        .open(path)
        .with_context(|| format!("error: unable to create file {}", path.display()))
}

fn create_dir_secure(path: &Path) -> Result<()> {
    let mut cursor = PathBuf::new();

    for component in path.components() {
        cursor.push(component);
        if cursor.exists() {
            continue;
        }

        let mut builder = DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }

        builder
            .create(&cursor)
            .with_context(|| format!("error: unable to create directory {}", cursor.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn validate_cache_path_allows_regular_path() {
        let temp = tempdir().unwrap();
        let cache_dir = temp.path().join(".regret");
        assert!(validate_cache_path(&cache_dir).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn validate_cache_path_rejects_symlink_component() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let target = temp.path().join("target");
        fs::create_dir_all(&target).unwrap();
        let link = temp.path().join(".regret");
        symlink(&target, &link).unwrap();

        let err = validate_cache_path(&link).unwrap_err();
        assert!(err.to_string().contains(SYMLINK_ERROR));
    }

    #[cfg(unix)]
    #[test]
    fn validate_cache_path_rejects_symlink_parent() {
        use std::os::unix::fs::symlink;

        let temp = tempdir().unwrap();
        let real = temp.path().join("real");
        fs::create_dir_all(&real).unwrap();
        let link = temp.path().join("link");
        symlink(&real, &link).unwrap();
        let cache_dir = link.join(".regret");

        let err = validate_cache_path(&cache_dir).unwrap_err();
        assert!(err.to_string().contains(SYMLINK_ERROR));
    }

    #[cfg(unix)]
    #[test]
    fn ensure_cache_dir_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempdir().unwrap();
        let cache_dir = temp.path().join(".regret");
        ensure_cache_dir(&cache_dir).unwrap();

        let dir_mode = fs::metadata(&cache_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);

        let file_path = cache_dir.join("cache.db");
        create_file_secure(&file_path).unwrap();
        let file_mode = fs::metadata(&file_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(file_mode, 0o600);
    }
}
