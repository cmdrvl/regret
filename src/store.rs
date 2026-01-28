use crate::cache_path;
use anyhow::{anyhow, bail, Context, Result};
use rusqlite::params;
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const CACHE_DB_FILENAME: &str = "cache.db";
const SCHEMA_VERSION: i64 = 1;
const SCHEMA_V1_SQL: &str = include_str!("../docs/schema/sqlite/v1.sql");

#[derive(Debug, Default)]
pub(crate) struct TableInfo {
    pub(crate) exists: bool,
    pub(crate) row_count: Option<i64>,
}

#[derive(Debug, Default)]
pub(crate) struct DiagnosticResults {
    pub(crate) quick_check: String,
    pub(crate) schema_version: Option<String>,
    pub(crate) tables: HashMap<String, TableInfo>,
    pub(crate) indexes: HashMap<String, bool>,
    pub(crate) last_scanned_ref_oid: Option<String>,
    pub(crate) coverage_since_utc: Option<String>,
    pub(crate) coverage_since_oid: Option<String>,
    pub(crate) coverage_valid: Option<bool>,
    pub(crate) is_shallow: Option<bool>,
    pub(crate) integrity_check: Option<String>,
}

pub(crate) struct Store {
    #[allow(dead_code)]
    conn: Connection,
    #[allow(dead_code)]
    path: PathBuf,
}

pub(crate) struct CommitRow {
    pub(crate) sha: String,
    pub(crate) time_utc: String,
    pub(crate) subject: Option<String>,
    pub(crate) pr_number: Option<i64>,
    pub(crate) pr_source: Option<String>,
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

    pub(crate) fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let value = self
            .conn
            .query_row("SELECT value FROM meta WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(value)
    }

    pub(crate) fn get_meta_value(&self, key: &str) -> Result<Option<String>> {
        self.get_meta(key)
    }

    pub(crate) fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
                [key, value],
            )
            .context("error: unable to set meta value")?;
        Ok(())
    }

    pub(crate) fn set_meta_value(&self, key: &str, value: &str) -> Result<()> {
        self.set_meta(key, value)
    }

    pub(crate) fn get_meta_bool(&self, key: &str) -> Result<Option<bool>> {
        let value = self.get_meta(key)?;
        Ok(value.map(|raw| raw == "true"))
    }

    pub(crate) fn set_meta_bool(&self, key: &str, value: bool) -> Result<()> {
        self.set_meta(key, if value { "true" } else { "false" })
    }

    pub(crate) fn upsert_commits(&mut self, commits: &[CommitRow]) -> Result<()> {
        if commits.is_empty() {
            return Ok(());
        }

        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO \"commit\" \
                (sha, time_utc, subject, pr_number, pr_source, patch_id, patch_id_rev, files_hash) \
                VALUES (?1, ?2, ?3, ?4, ?5, NULL, NULL, NULL)",
            )?;

            for commit in commits {
                stmt.execute(params![
                    commit.sha,
                    commit.time_utc,
                    commit.subject,
                    commit.pr_number,
                    commit.pr_source
                ])?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub(crate) fn get_oldest_commit(&self) -> Result<Option<(String, String)>> {
        self.conn
            .query_row(
                "SELECT sha, time_utc FROM \"commit\" ORDER BY time_utc ASC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .context("error: unable to read oldest commit")
    }

    /// Look up a commit by SHA and return its time_utc if found.
    pub(crate) fn get_commit_time(&self, sha: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT time_utc FROM \"commit\" WHERE sha=?1",
                [sha],
                |row| row.get(0),
            )
            .optional()
            .context("error: unable to read commit time")
    }

    /// Insert detected signals into the database.
    pub(crate) fn insert_signals(
        &mut self,
        ref_name: &str,
        signals: &[crate::signals::DetectedSignal],
    ) -> Result<usize> {
        if signals.is_empty() {
            return Ok(0);
        }

        let tx = self.conn.transaction()?;
        let mut inserted = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO signal \
                (ref, type, time_utc, culprit_sha, evidence_sha, weight, confidence, \
                 time_to_regret_hours, culprit_files_hash, evidence_files_hash) \
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL)",
            )?;

            for signal in signals {
                stmt.execute(params![
                    ref_name,
                    signal.signal_type.as_str(),
                    signal.evidence_time_utc,
                    signal.culprit_sha,
                    signal.evidence_sha,
                    signal.weight,
                    signal.confidence,
                    signal.time_to_regret_hours,
                ])?;
                inserted += 1;
            }
        }

        tx.commit()?;
        Ok(inserted)
    }

    /// Check if a signal already exists for the given culprit and evidence SHAs.
    pub(crate) fn signal_exists(&self, culprit_sha: &str, evidence_sha: &str) -> Result<bool> {
        let exists: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM signal WHERE culprit_sha=?1 AND evidence_sha=?2",
                [culprit_sha, evidence_sha],
                |_| Ok(true),
            )
            .optional()?
            .unwrap_or(false);
        Ok(exists)
    }

    /// Get the patch_id for a commit (20 bytes, stored as BLOB).
    pub(crate) fn get_patch_id(&self, sha: &str) -> Result<Option<[u8; 20]>> {
        let result: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT patch_id FROM \"commit\" WHERE sha=?1",
                [sha],
                |row| row.get(0),
            )
            .optional()?;

        match result {
            Some(bytes) if bytes.len() == 20 => {
                let mut arr = [0u8; 20];
                arr.copy_from_slice(&bytes);
                Ok(Some(arr))
            }
            Some(_) => Ok(None), // Invalid length, treat as missing
            None => Ok(None),
        }
    }

    /// Set the patch_id for a commit (20 bytes, stored as BLOB).
    pub(crate) fn set_patch_id(&self, sha: &str, patch_id: &[u8; 20]) -> Result<()> {
        self.conn.execute(
            "UPDATE \"commit\" SET patch_id=?1 WHERE sha=?2",
            params![patch_id.as_slice(), sha],
        )?;
        Ok(())
    }

    /// Get the patch_id_rev (reverse patch-id) for a commit.
    pub(crate) fn get_patch_id_rev(&self, sha: &str) -> Result<Option<[u8; 20]>> {
        let result: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT patch_id_rev FROM \"commit\" WHERE sha=?1",
                [sha],
                |row| row.get(0),
            )
            .optional()?;

        match result {
            Some(bytes) if bytes.len() == 20 => {
                let mut arr = [0u8; 20];
                arr.copy_from_slice(&bytes);
                Ok(Some(arr))
            }
            Some(_) => Ok(None), // Invalid length, treat as missing
            None => Ok(None),
        }
    }

    /// Set the patch_id_rev (reverse patch-id) for a commit.
    pub(crate) fn set_patch_id_rev(&self, sha: &str, patch_id_rev: &[u8; 20]) -> Result<()> {
        self.conn.execute(
            "UPDATE \"commit\" SET patch_id_rev=?1 WHERE sha=?2",
            params![patch_id_rev.as_slice(), sha],
        )?;
        Ok(())
    }

    /// Run database diagnostics and return formatted results
    pub(crate) fn run_diagnostics(&self, deep: bool) -> Result<DiagnosticResults> {
        use rusqlite::OptionalExtension;

        let mut results = DiagnosticResults::default();

        // Quick check (PRAGMA quick_check)
        match self
            .conn
            .query_row("PRAGMA quick_check", [], |row: &rusqlite::Row| {
                let result: String = row.get(0)?;
                Ok(result)
            }) {
            Ok(result) => {
                results.quick_check = if result == "ok" {
                    "ok".to_string()
                } else {
                    format!("FAILED: {}", result)
                };
            }
            Err(e) => {
                results.quick_check = format!("ERROR: {}", e);
            }
        }

        // Schema version check
        match self.conn.query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |row: &rusqlite::Row| {
                let version: String = row.get(0)?;
                Ok(version)
            },
        ) {
            Ok(version) => {
                results.schema_version = Some(version);
            }
            Err(_) => {
                results.schema_version = None;
            }
        }

        // Check for core tables and get row counts
        let tables_to_check = vec!["meta", "file", "commit", "fileset", "signal"];
        for table in &tables_to_check {
            let exists = self
                .conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?",
                    [table],
                    |_row: &rusqlite::Row| Ok(true),
                )
                .optional()?
                .is_some();

            let row_count = if exists {
                let table_sql = format!("\"{}\"", table);
                self.conn
                    .query_row(
                        &format!("SELECT COUNT(*) FROM {}", table_sql),
                        [],
                        |row: &rusqlite::Row| {
                            let count: i64 = row.get(0)?;
                            Ok(count)
                        },
                    )
                    .ok()
            } else {
                None
            };

            results
                .tables
                .insert(table.to_string(), TableInfo { exists, row_count });
        }

        // Check indexes
        let indexes_to_check = vec!["signal_time", "signal_culprit"];
        for index in &indexes_to_check {
            let exists = self
                .conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='index' AND name=?",
                    [index],
                    |_row: &rusqlite::Row| Ok(true),
                )
                .optional()?
                .is_some();

            results.indexes.insert(index.to_string(), exists);
        }

        // Coverage and scanning status
        results.last_scanned_ref_oid = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key='last_scanned_ref_oid'",
                [],
                |row: &rusqlite::Row| {
                    let oid: String = row.get(0)?;
                    Ok(oid)
                },
            )
            .optional()?;

        results.coverage_since_utc = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key='coverage_since_utc'",
                [],
                |row: &rusqlite::Row| {
                    let coverage: String = row.get(0)?;
                    Ok(coverage)
                },
            )
            .optional()?;

        results.coverage_since_oid = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key='coverage_since_oid'",
                [],
                |row: &rusqlite::Row| {
                    let coverage: String = row.get(0)?;
                    Ok(coverage)
                },
            )
            .optional()?;

        results.coverage_valid = self.get_meta_bool("coverage_valid")?;
        results.is_shallow = self.get_meta_bool("is_shallow")?;

        // Deep integrity check if requested
        if deep {
            match self
                .conn
                .query_row("PRAGMA integrity_check", [], |row: &rusqlite::Row| {
                    let result: String = row.get(0)?;
                    Ok(result)
                }) {
                Ok(result) => {
                    results.integrity_check = Some(if result == "ok" {
                        "ok".to_string()
                    } else {
                        format!("FAILED: {}", result)
                    });
                }
                Err(e) => {
                    results.integrity_check = Some(format!("ERROR: {}", e));
                }
            }
        }

        Ok(results)
    }
}

fn ensure_db_file(path: &Path) -> Result<()> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("error: unable to stat cache db {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("error: cache db is a symlink; refusing to write");
        }
        if !metadata.is_file() {
            bail!(
                "error: cache db path is not a regular file: {}",
                path.display()
            );
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
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn real_path(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    #[test]
    fn store_creates_db_and_schema() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        let cache_dir = base.join(".regret");
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
        let base = real_path(temp.path());
        let cache_dir = base.join(".regret");
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
