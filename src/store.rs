use crate::cache_path;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const CACHE_DB_FILENAME: &str = "cache.db";
const SCHEMA_VERSION: i64 = 1;
const SCHEMA_V1_SQL: &str = include_str!("../docs/schema/sqlite/v1.sql");

pub(crate) struct Store {
    #[allow(dead_code)]
    conn: Connection,
    #[allow(dead_code)]
    path: PathBuf,
}

impl Store {
    pub(crate) fn open(cache_root: &Path) -> Result<Self> {
        cache_path::ensure_cache_dir(cache_root)?;
        let db_path = cache_root.join(CACHE_DB_FILENAME);
        ensure_db_file(&db_path)?;

        let conn = Connection::open(&db_path)
            .with_context(|| format!("error: unable to open cache db {}", db_path.display()))?;

        apply_pragmas(&conn)?;
        ensure_schema(&conn)?;

        Ok(Self {
            conn,
            path: db_path,
        })
    }

    #[cfg(test)]
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    #[allow(dead_code)]
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

fn ensure_db_file(path: &Path) -> Result<()> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("error: unable to stat cache db {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("error: cache db is a symlink; refusing to write");
        }
        return Ok(());
    }

    let _file = cache_path::create_file_secure(path)?;
    Ok(())
}

fn apply_pragmas(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(Duration::from_millis(5000))?;
    conn.pragma_update(None, "cache_size", -64000i64)?;
    conn.pragma_update(None, "mmap_size", 268_435_456i64)?;
    Ok(())
}

fn ensure_schema(conn: &Connection) -> Result<()> {
    let has_meta: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='meta'",
            [],
            |_| Ok(true),
        )
        .optional()?
        .unwrap_or(false);

    if !has_meta {
        conn.execute_batch(SCHEMA_V1_SQL)
            .context("error: unable to create cache schema")?;
        set_schema_version(conn, SCHEMA_VERSION)?;
        return Ok(());
    }

    let version_str: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;

    match version_str {
        None => {
            set_schema_version(conn, SCHEMA_VERSION)?;
        }
        Some(value) => {
            let version: i64 = value
                .parse()
                .map_err(|_| anyhow!("error: invalid schema_version value '{}'", value))?;
            if version == SCHEMA_VERSION {
                return Ok(());
            }
            if version > SCHEMA_VERSION {
                bail!(
                    "error: cache schema version {} is newer than supported {}",
                    version,
                    SCHEMA_VERSION
                );
            }
            bail!(
                "error: cache schema version {} is older than supported {}",
                version,
                SCHEMA_VERSION
            );
        }
    }

    Ok(())
}

fn set_schema_version(conn: &Connection, version: i64) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?1)",
        [version.to_string()],
    )
    .context("error: unable to set schema_version")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn store_creates_db_and_schema() {
        let temp = tempdir().unwrap();
        let cache_dir = temp.path().join(".regret");
        let store = Store::open(&cache_dir).unwrap();

        assert!(store.path().exists());

        let version: String = store
            .conn()
            .query_row(
                "SELECT value FROM meta WHERE key='schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, "1");
    }

    #[test]
    fn store_applies_pragmas() {
        let temp = tempdir().unwrap();
        let cache_dir = temp.path().join(".regret");
        let store = Store::open(&cache_dir).unwrap();

        let journal_mode: String = store
            .conn()
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let foreign_keys: i64 = store
            .conn()
            .query_row("PRAGMA foreign_keys;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1);

        let synchronous: i64 = store
            .conn()
            .query_row("PRAGMA synchronous;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(synchronous, 1);

        let cache_size: i64 = store
            .conn()
            .query_row("PRAGMA cache_size;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(cache_size, -64000);

        let mmap_size: i64 = store
            .conn()
            .query_row("PRAGMA mmap_size;", [], |row| row.get(0))
            .unwrap();
        assert_eq!(mmap_size, 268_435_456);
    }
}
