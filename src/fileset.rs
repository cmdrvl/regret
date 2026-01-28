//! Lazy file set computation for surface/hotspot analysis.
//!
//! Per §9.2: Computes file sets ONLY for commits involved in signals.
//! - files_hash: blake3 over sorted included file IDs
//! - file_count_total: before filtering
//! - file_count_included: after filtering

#![allow(dead_code)]

use anyhow::{Context, Result};
use git2::{DiffOptions, Repository};
use globset::{Glob, GlobSet, GlobSetBuilder};
use rusqlite::params;

use crate::config_file::SurfacesConfig;
use crate::store::Store;

/// Default ignore patterns applied unless overridden.
const DEFAULT_IGNORE_PATTERNS: &[&str] = &[
    "**/node_modules/**",
    "**/vendor/**",
    "**/target/**",
    "**/dist/**",
    "**/build/**",
    "**/*.lock",
    "**/package-lock.json",
    "**/yarn.lock",
    "**/pnpm-lock.yaml",
    "**/*.min.js",
    "**/*.min.css",
    "**/*.generated.*",
    "**/*_generated.*",
    "**/.git/**",
];

/// Compiled ignore patterns for efficient matching.
#[derive(Debug)]
pub struct IgnorePatterns {
    globset: GlobSet,
}

impl IgnorePatterns {
    /// Build ignore patterns from defaults only.
    pub fn defaults() -> Result<Self> {
        Self::from_patterns(DEFAULT_IGNORE_PATTERNS.iter().map(|s| s.to_string()))
    }

    /// Build ignore patterns from config.
    ///
    /// If config has `ignore_patterns`, those replace defaults.
    /// If config has `additional_ignore`, those extend defaults.
    pub fn from_config(config: &SurfacesConfig) -> Result<Self> {
        let patterns: Vec<String> = if !config.ignore_patterns.is_empty() {
            // Replace defaults with custom patterns
            config.ignore_patterns.clone()
        } else {
            // Use defaults, possibly extended
            let mut patterns: Vec<String> = DEFAULT_IGNORE_PATTERNS
                .iter()
                .map(|s| s.to_string())
                .collect();
            patterns.extend(config.additional_ignore.iter().cloned());
            patterns
        };

        Self::from_patterns(patterns.into_iter())
    }

    /// Build from an iterator of pattern strings.
    fn from_patterns<I: Iterator<Item = String>>(patterns: I) -> Result<Self> {
        let mut builder = GlobSetBuilder::new();

        for pattern in patterns {
            let glob = Glob::new(&pattern)
                .with_context(|| format!("Invalid ignore pattern: {}", pattern))?;
            builder.add(glob);
        }

        let globset = builder.build().context("Failed to build ignore pattern set")?;
        Ok(Self { globset })
    }

    /// Check if a path should be ignored.
    pub fn is_ignored(&self, path: &str) -> bool {
        self.globset.is_match(path)
    }
}

/// A computed file set for a commit.
#[derive(Debug, Clone)]
pub struct FileSet {
    /// Blake3 hash of sorted file IDs (32 bytes)
    pub files_hash: [u8; 32],
    /// Total files changed before filtering
    pub file_count_total: usize,
    /// Files after ignore pattern filtering
    pub file_count_included: usize,
    /// Sorted file IDs (varint encoded in blob)
    pub file_ids: Vec<i64>,
}

/// Compute file set for a commit, caching the result.
///
/// If `ignore` is provided, files matching patterns are excluded from `file_ids`
/// and `file_count_included`, but `file_count_total` reflects all changed files.
///
/// Returns None if the commit has no changed files after filtering.
pub fn compute_file_set_for_commit(
    store: &mut Store,
    repo: &Repository,
    sha: &str,
    ignore: Option<&IgnorePatterns>,
) -> Result<Option<FileSet>> {
    // Check if already computed
    if let Some(existing_hash) = store.get_commit_files_hash(sha)? {
        // Already computed - look up in fileset table
        if let Some(fileset) = store.get_fileset(&existing_hash)? {
            return Ok(Some(fileset));
        }
    }

    // Get the commit
    let oid = git2::Oid::from_str(sha).context("Invalid SHA")?;
    let commit = repo.find_commit(oid).context("Commit not found")?;

    // Get changed files by diffing against parent(s)
    let changed_files = get_changed_files(repo, &commit)?;

    if changed_files.is_empty() {
        return Ok(None);
    }

    let file_count_total = changed_files.len();

    // Filter by ignore patterns if provided
    let included_files: Vec<&str> = match ignore {
        Some(patterns) => changed_files
            .iter()
            .filter(|path| !patterns.is_ignored(path))
            .map(|s| s.as_str())
            .collect(),
        None => changed_files.iter().map(|s| s.as_str()).collect(),
    };

    if included_files.is_empty() {
        return Ok(None);
    }

    // Get or create file IDs for included files only
    let mut file_ids: Vec<i64> = Vec::with_capacity(included_files.len());
    for path in &included_files {
        let id = store.get_or_create_file_id(path)?;
        file_ids.push(id);
    }

    // Sort for deterministic hashing
    file_ids.sort_unstable();

    // Compute blake3 hash of sorted IDs
    let files_hash = compute_files_hash(&file_ids);

    let fileset = FileSet {
        files_hash,
        file_count_total,
        file_count_included: file_ids.len(),
        file_ids: file_ids.clone(),
    };

    // Cache in database
    store.insert_fileset(&fileset)?;
    store.set_commit_files_hash(sha, &files_hash)?;

    Ok(Some(fileset))
}

/// Get changed files for a commit by diffing against parent(s).
fn get_changed_files(repo: &Repository, commit: &git2::Commit) -> Result<Vec<String>> {
    let tree = commit.tree().context("Failed to get commit tree")?;

    let mut changed_files = Vec::new();
    let mut diff_opts = DiffOptions::new();

    // For merge commits, diff against first parent only (standard behavior)
    let parent_tree = if commit.parent_count() > 0 {
        Some(commit.parent(0)?.tree()?)
    } else {
        None // Initial commit - all files are "added"
    };

    let diff = repo.diff_tree_to_tree(
        parent_tree.as_ref(),
        Some(&tree),
        Some(&mut diff_opts),
    )?;

    diff.foreach(
        &mut |delta, _| {
            // Get the new path (or old path for deletions)
            if let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) {
                if let Some(path_str) = path.to_str() {
                    changed_files.push(path_str.to_string());
                }
            }
            true
        },
        None,
        None,
        None,
    )?;

    Ok(changed_files)
}

/// Compute blake3 hash of sorted file IDs.
fn compute_files_hash(file_ids: &[i64]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();

    // Hash each ID as little-endian bytes
    for id in file_ids {
        hasher.update(&id.to_le_bytes());
    }

    *hasher.finalize().as_bytes()
}

/// Encode file IDs as varint blob for storage.
#[allow(dead_code)]
fn encode_file_ids(file_ids: &[i64]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(file_ids.len() * 5); // Rough estimate

    for id in file_ids {
        encode_varint(*id as u64, &mut blob);
    }

    blob
}

/// Decode varint blob to file IDs.
#[allow(dead_code)]
fn decode_file_ids(blob: &[u8]) -> Vec<i64> {
    let mut ids = Vec::new();
    let mut offset = 0;

    while offset < blob.len() {
        let (value, bytes_read) = decode_varint(&blob[offset..]);
        ids.push(value as i64);
        offset += bytes_read;
    }

    ids
}

/// Encode a u64 as varint.
fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// Decode a varint from bytes, returning (value, bytes_read).
fn decode_varint(bytes: &[u8]) -> (u64, usize) {
    let mut value: u64 = 0;
    let mut shift = 0;

    for (i, &byte) in bytes.iter().enumerate() {
        value |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return (value, i + 1);
        }
        shift += 7;
        if shift >= 64 {
            break;
        }
    }

    (value, bytes.len())
}

// Store methods for file set operations
impl Store {
    /// Get or create a file ID for a path.
    pub(crate) fn get_or_create_file_id(&mut self, path: &str) -> Result<i64> {
        // Try to get existing
        if let Some(id) = self.get_file_id(path)? {
            return Ok(id);
        }

        // Insert new
        self.conn.execute(
            "INSERT INTO file (path) VALUES (?1)",
            [path],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Get file ID for a path if it exists.
    fn get_file_id(&self, path: &str) -> Result<Option<i64>> {
        use rusqlite::OptionalExtension;
        self.conn
            .query_row("SELECT id FROM file WHERE path = ?1", [path], |row| row.get(0))
            .optional()
            .context("Failed to query file ID")
    }

    /// Get files_hash for a commit.
    pub(crate) fn get_commit_files_hash(&self, sha: &str) -> Result<Option<[u8; 32]>> {
        use rusqlite::OptionalExtension;
        let result: Option<Vec<u8>> = self
            .conn
            .query_row(
                "SELECT files_hash FROM \"commit\" WHERE sha = ?1",
                [sha],
                |row| row.get(0),
            )
            .optional()?;

        match result {
            Some(bytes) if bytes.len() == 32 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes);
                Ok(Some(arr))
            }
            _ => Ok(None),
        }
    }

    /// Set files_hash for a commit.
    pub(crate) fn set_commit_files_hash(&self, sha: &str, files_hash: &[u8; 32]) -> Result<()> {
        self.conn.execute(
            "UPDATE \"commit\" SET files_hash = ?1 WHERE sha = ?2",
            params![files_hash.as_slice(), sha],
        )?;
        Ok(())
    }

    /// Get a fileset by hash.
    pub(crate) fn get_fileset(&self, files_hash: &[u8; 32]) -> Result<Option<FileSet>> {
        use rusqlite::OptionalExtension;
        let result: Option<(i64, i64, Vec<u8>)> = self
            .conn
            .query_row(
                "SELECT file_count_total, file_count_included, blob FROM fileset WHERE files_hash = ?1",
                [files_hash.as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;

        match result {
            Some((total, included, blob)) => {
                let file_ids = decode_file_ids(&blob);
                Ok(Some(FileSet {
                    files_hash: *files_hash,
                    file_count_total: total as usize,
                    file_count_included: included as usize,
                    file_ids,
                }))
            }
            None => Ok(None),
        }
    }

    /// Insert a fileset.
    pub(crate) fn insert_fileset(&self, fileset: &FileSet) -> Result<()> {
        let blob = encode_file_ids(&fileset.file_ids);
        self.conn.execute(
            "INSERT OR IGNORE INTO fileset (files_hash, file_count_total, file_count_included, blob) \
             VALUES (?1, ?2, ?3, ?4)",
            params![
                fileset.files_hash.as_slice(),
                fileset.file_count_total as i64,
                fileset.file_count_included as i64,
                blob,
            ],
        )?;
        Ok(())
    }

    /// Get unique culprit/evidence SHAs from signals that need file set computation.
    pub(crate) fn get_signal_shas_needing_filesets(&self, ref_name: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT sha FROM (
                SELECT culprit_sha AS sha FROM signal WHERE ref = ?1
                UNION
                SELECT evidence_sha AS sha FROM signal WHERE ref = ?1
            ) s
            JOIN \"commit\" c ON c.sha = s.sha
            WHERE c.files_hash IS NULL",
        )?;

        let rows = stmt
            .query_map([ref_name], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint_roundtrip() {
        let values = [0u64, 1, 127, 128, 255, 256, 16383, 16384, u64::MAX];

        for &value in &values {
            let mut encoded = Vec::new();
            encode_varint(value, &mut encoded);
            let (decoded, _) = decode_varint(&encoded);
            assert_eq!(value, decoded, "Varint roundtrip failed for {}", value);
        }
    }

    #[test]
    fn test_file_ids_roundtrip() {
        let ids = vec![1i64, 42, 100, 1000, 10000];
        let encoded = encode_file_ids(&ids);
        let decoded = decode_file_ids(&encoded);
        assert_eq!(ids, decoded);
    }

    #[test]
    fn test_files_hash_deterministic() {
        let ids1 = vec![1i64, 2, 3];
        let ids2 = vec![1i64, 2, 3];
        let ids3 = vec![3i64, 2, 1]; // Different order

        let hash1 = compute_files_hash(&ids1);
        let hash2 = compute_files_hash(&ids2);
        let hash3 = compute_files_hash(&ids3);

        assert_eq!(hash1, hash2, "Same IDs should produce same hash");
        assert_ne!(hash1, hash3, "Different order should produce different hash");
    }

    #[test]
    fn test_empty_file_ids() {
        let ids: Vec<i64> = vec![];
        let encoded = encode_file_ids(&ids);
        let decoded = decode_file_ids(&encoded);
        assert!(decoded.is_empty());
    }

    #[test]
    fn test_default_ignore_patterns() {
        let patterns = IgnorePatterns::defaults().unwrap();

        // Should ignore node_modules
        assert!(patterns.is_ignored("node_modules/lodash/index.js"));
        assert!(patterns.is_ignored("src/node_modules/pkg/file.js"));

        // Should ignore vendor
        assert!(patterns.is_ignored("vendor/github.com/pkg/file.go"));

        // Should ignore target (Rust build dir)
        assert!(patterns.is_ignored("target/debug/main"));

        // Should ignore dist/build
        assert!(patterns.is_ignored("dist/bundle.js"));
        assert!(patterns.is_ignored("build/output.js"));

        // Should ignore lock files
        assert!(patterns.is_ignored("package-lock.json"));
        assert!(patterns.is_ignored("yarn.lock"));
        assert!(patterns.is_ignored("pnpm-lock.yaml"));
        assert!(patterns.is_ignored("Cargo.lock"));

        // Should ignore minified files
        assert!(patterns.is_ignored("app.min.js"));
        assert!(patterns.is_ignored("styles.min.css"));

        // Should ignore generated files
        assert!(patterns.is_ignored("schema.generated.ts"));
        assert!(patterns.is_ignored("api_generated.py"));

        // Should ignore .git
        assert!(patterns.is_ignored(".git/objects/abc123"));

        // Should NOT ignore regular source files
        assert!(!patterns.is_ignored("src/main.rs"));
        assert!(!patterns.is_ignored("app/index.js"));
        assert!(!patterns.is_ignored("lib/utils.py"));
    }

    #[test]
    fn test_config_replaces_defaults() {
        let config = SurfacesConfig {
            ignore_patterns: vec!["**/*.test.js".to_string()],
            additional_ignore: vec![],
        };

        let patterns = IgnorePatterns::from_config(&config).unwrap();

        // Should ignore test files (from config)
        assert!(patterns.is_ignored("src/utils.test.js"));

        // Should NOT ignore node_modules (defaults replaced)
        assert!(!patterns.is_ignored("node_modules/lodash/index.js"));
    }

    #[test]
    fn test_config_extends_defaults() {
        let config = SurfacesConfig {
            ignore_patterns: vec![],
            additional_ignore: vec!["**/*.test.js".to_string(), "**/__tests__/**".to_string()],
        };

        let patterns = IgnorePatterns::from_config(&config).unwrap();

        // Should still ignore node_modules (from defaults)
        assert!(patterns.is_ignored("node_modules/lodash/index.js"));

        // Should also ignore test files (from additional)
        assert!(patterns.is_ignored("src/utils.test.js"));
        assert!(patterns.is_ignored("src/__tests__/foo.js"));
    }

    #[test]
    fn test_empty_config_uses_defaults() {
        let config = SurfacesConfig::default();
        let patterns = IgnorePatterns::from_config(&config).unwrap();

        // Should behave same as defaults
        assert!(patterns.is_ignored("node_modules/pkg/file.js"));
        assert!(!patterns.is_ignored("src/main.rs"));
    }
}
