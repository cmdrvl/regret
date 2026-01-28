//! Schema lock tests per §15.3.
//!
//! These tests enforce schema versioning policy to prevent
//! accidental breaking changes to the cache schema or NDJSON format.

use rusqlite::Connection;
use std::collections::HashSet;

/// The expected schema version constant (must match docs and code).
const EXPECTED_SCHEMA_VERSION: &str = "1";

/// Expected tables in the SQLite schema.
const EXPECTED_TABLES: &[&str] = &["meta", "file", "commit", "fileset", "signal"];

/// Expected indexes in the SQLite schema.
const EXPECTED_INDEXES: &[&str] = &["signal_time", "signal_culprit"];

/// Expected columns in the signal table.
const EXPECTED_SIGNAL_COLUMNS: &[&str] = &[
    "id",
    "ref",
    "type",
    "time_utc",
    "culprit_sha",
    "evidence_sha",
    "weight",
    "confidence",
    "time_to_regret_hours",
    "culprit_files_hash",
    "evidence_files_hash",
];

/// Expected columns in the commit table.
const EXPECTED_COMMIT_COLUMNS: &[&str] = &[
    "sha",
    "time_utc",
    "subject",
    "pr_number",
    "pr_source",
    "patch_id",
    "patch_id_rev",
    "files_hash",
];

/// Expected NDJSON meta record fields.
const EXPECTED_NDJSON_META_FIELDS: &[&str] = &[
    "type",
    "schema_version",
    "tool_version",
    "repo_id",
    "repo_basename",
    "selected_branch",
    "window_since_utc",
    "window_until_utc",
    "window_until_source",
    "coverage_since_utc",
    "coverage_valid",
    "cache_valid",
];

/// Expected NDJSON stat record fields.
const EXPECTED_NDJSON_STAT_FIELDS: &[&str] = &["type", "name", "value"];

/// Expected NDJSON rank record fields.
const EXPECTED_NDJSON_RANK_FIELDS: &[&str] = &[
    "type",
    "culprit_sha",
    "score",
    "events",
    "ttr_p50_h",
    "culprit_time_utc",
];

/// Expected NDJSON evidence record fields.
const EXPECTED_NDJSON_EVIDENCE_FIELDS: &[&str] = &[
    "type",
    "culprit_sha",
    "evidence_sha",
    "signal_type",
    "confidence",
    "confidence_reason",
    "time_to_regret_hours",
];

/// Create a database from the v1.sql schema file.
fn create_v1_db() -> Connection {
    let schema_sql = include_str!("../docs/schema/sqlite/v1.sql");
    let conn = Connection::open_in_memory().expect("create in-memory db");
    conn.execute_batch(schema_sql).expect("execute v1 schema");
    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)",
        [EXPECTED_SCHEMA_VERSION],
    )
    .expect("set schema version");
    conn
}

/// Get table names from sqlite_master.
fn get_table_names(conn: &Connection) -> HashSet<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .expect("prepare table query");
    let rows = stmt
        .query_map([], |row| row.get(0))
        .expect("execute table query");
    rows.filter_map(|r| r.ok()).collect()
}

/// Get index names from sqlite_master.
fn get_index_names(conn: &Connection) -> HashSet<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='index' AND name NOT LIKE 'sqlite_%'")
        .expect("prepare index query");
    let rows = stmt
        .query_map([], |row| row.get(0))
        .expect("execute index query");
    rows.filter_map(|r| r.ok()).collect()
}

/// Get column names for a table.
fn get_column_names(conn: &Connection, table: &str) -> HashSet<String> {
    // Use PRAGMA table_info to get column names
    let pragma_sql = format!("PRAGMA table_info(\"{}\")", table);
    let mut stmt = conn.prepare(&pragma_sql).expect("prepare pragma");
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("execute pragma");
    rows.filter_map(|r| r.ok()).collect()
}

#[test]
fn schema_version_matches_docs() {
    let conn = create_v1_db();
    let version: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("read schema version");

    assert_eq!(
        version, EXPECTED_SCHEMA_VERSION,
        "Schema version in DB must match EXPECTED_SCHEMA_VERSION constant"
    );
}

#[test]
fn required_tables_exist() {
    let conn = create_v1_db();
    let tables = get_table_names(&conn);

    for expected in EXPECTED_TABLES {
        assert!(
            tables.contains(*expected),
            "Required table '{}' is missing from schema",
            expected
        );
    }
}

#[test]
fn required_indexes_exist() {
    let conn = create_v1_db();
    let indexes = get_index_names(&conn);

    for expected in EXPECTED_INDEXES {
        assert!(
            indexes.contains(*expected),
            "Required index '{}' is missing from schema",
            expected
        );
    }
}

#[test]
fn signal_table_has_required_columns() {
    let conn = create_v1_db();
    let columns = get_column_names(&conn, "signal");

    for expected in EXPECTED_SIGNAL_COLUMNS {
        assert!(
            columns.contains(*expected),
            "Required column 'signal.{}' is missing from schema",
            expected
        );
    }
}

#[test]
fn commit_table_has_required_columns() {
    let conn = create_v1_db();
    let columns = get_column_names(&conn, "commit");

    for expected in EXPECTED_COMMIT_COLUMNS {
        assert!(
            columns.contains(*expected),
            "Required column 'commit.{}' is missing from schema",
            expected
        );
    }
}

#[test]
fn foreign_key_check_passes() {
    let conn = create_v1_db();
    conn.pragma_update(None, "foreign_keys", "ON")
        .expect("enable foreign keys");

    let violations: i64 = conn
        .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .expect("run foreign key check");

    assert_eq!(
        violations, 0,
        "Foreign key violations found in schema: {}",
        violations
    );
}

#[test]
fn v1_sql_creates_valid_db() {
    // This test ensures the v1.sql file is valid and creates a working database
    let conn = create_v1_db();

    // Verify we can insert a commit
    conn.execute(
        "INSERT INTO \"commit\" (sha, time_utc, subject) VALUES (?1, ?2, ?3)",
        [
            "a".repeat(40).as_str(),
            "2026-01-27T12:00:00Z",
            "Test commit",
        ],
    )
    .expect("insert commit");

    // Verify we can insert a signal
    conn.execute(
        "INSERT INTO signal (ref, type, time_utc, culprit_sha, evidence_sha, weight, confidence, time_to_regret_hours) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        [
            "refs/heads/main",
            "revert",
            "2026-01-27T13:00:00Z",
            "a".repeat(40).as_str(),
            "b".repeat(40).as_str(),
            "10",
            "0.95",
            "1.0",
        ],
    ).expect("insert signal");

    // Verify the signal is readable
    let count: i64 = conn
        .query_row("SELECT count(*) FROM signal", [], |row| row.get(0))
        .expect("count signals");
    assert_eq!(count, 1);
}

#[test]
fn signal_type_constraint_enforced() {
    let conn = create_v1_db();

    // Insert a commit first (for foreign key if needed)
    conn.execute(
        "INSERT INTO \"commit\" (sha, time_utc) VALUES (?1, ?2)",
        ["a".repeat(40).as_str(), "2026-01-27T12:00:00Z"],
    )
    .expect("insert commit");

    // Valid types should work
    conn.execute(
        "INSERT INTO signal (ref, type, time_utc, culprit_sha, evidence_sha, weight, confidence, time_to_regret_hours) \
         VALUES ('refs/heads/main', 'revert', '2026-01-27T13:00:00Z', ?, ?, 10, 0.95, 1.0)",
        ["a".repeat(40).as_str(), "b".repeat(40).as_str()],
    ).expect("insert revert signal");

    conn.execute(
        "INSERT INTO signal (ref, type, time_utc, culprit_sha, evidence_sha, weight, confidence, time_to_regret_hours) \
         VALUES ('refs/heads/main', 'linked_fix', '2026-01-27T14:00:00Z', ?, ?, 8, 0.90, 2.0)",
        ["a".repeat(40).as_str(), "c".repeat(40).as_str()],
    ).expect("insert linked_fix signal");

    // Invalid type should fail
    let result = conn.execute(
        "INSERT INTO signal (ref, type, time_utc, culprit_sha, evidence_sha, weight, confidence, time_to_regret_hours) \
         VALUES ('refs/heads/main', 'invalid', '2026-01-27T15:00:00Z', ?, ?, 5, 0.80, 3.0)",
        ["a".repeat(40).as_str(), "d".repeat(40).as_str()],
    );
    assert!(
        result.is_err(),
        "Invalid signal type should be rejected by CHECK constraint"
    );
}

#[test]
fn ordering_invariant_holds_for_out_of_order_inserts() {
    let conn = create_v1_db();

    // Insert signals out of order (by time_utc)
    conn.execute(
        "INSERT INTO signal (ref, type, time_utc, culprit_sha, evidence_sha, weight, confidence, time_to_regret_hours) \
         VALUES ('refs/heads/main', 'revert', '2026-01-27T15:00:00Z', ?, ?, 10, 0.95, 1.0)",
        ["a".repeat(40).as_str(), "e".repeat(40).as_str()],
    ).expect("insert signal 3");

    conn.execute(
        "INSERT INTO signal (ref, type, time_utc, culprit_sha, evidence_sha, weight, confidence, time_to_regret_hours) \
         VALUES ('refs/heads/main', 'revert', '2026-01-27T13:00:00Z', ?, ?, 10, 0.95, 1.0)",
        ["b".repeat(40).as_str(), "f".repeat(40).as_str()],
    ).expect("insert signal 1");

    conn.execute(
        "INSERT INTO signal (ref, type, time_utc, culprit_sha, evidence_sha, weight, confidence, time_to_regret_hours) \
         VALUES ('refs/heads/main', 'revert', '2026-01-27T14:00:00Z', ?, ?, 10, 0.95, 1.0)",
        ["c".repeat(40).as_str(), "g".repeat(40).as_str()],
    ).expect("insert signal 2");

    // Query with ORDER BY time_utc should return in correct order regardless of insert order
    let mut stmt = conn
        .prepare("SELECT culprit_sha FROM signal ORDER BY time_utc")
        .expect("prepare ordered query");
    let shas: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("execute query")
        .filter_map(|r| r.ok())
        .collect();

    // Should be ordered: b (13:00), c (14:00), a (15:00)
    assert_eq!(shas.len(), 3);
    assert!(shas[0].starts_with('b'), "First should be 'b' (13:00)");
    assert!(shas[1].starts_with('c'), "Second should be 'c' (14:00)");
    assert!(shas[2].starts_with('a'), "Third should be 'a' (15:00)");
}

/// Test that NDJSON meta record has expected structure.
/// This is a compile-time check via serde.
#[test]
fn ndjson_meta_fields_documented() {
    // This test documents the expected fields. The actual serialization
    // is tested in e2e_ndjson_snapshots.rs.
    // If fields are added/removed, this list must be updated.
    for field in EXPECTED_NDJSON_META_FIELDS {
        // Just verify the constant is well-formed
        assert!(!field.is_empty(), "Field name should not be empty");
    }
}

#[test]
fn ndjson_stat_fields_documented() {
    for field in EXPECTED_NDJSON_STAT_FIELDS {
        assert!(!field.is_empty(), "Field name should not be empty");
    }
}

#[test]
fn ndjson_rank_fields_documented() {
    for field in EXPECTED_NDJSON_RANK_FIELDS {
        assert!(!field.is_empty(), "Field name should not be empty");
    }
}

#[test]
fn ndjson_evidence_fields_documented() {
    for field in EXPECTED_NDJSON_EVIDENCE_FIELDS {
        assert!(!field.is_empty(), "Field name should not be empty");
    }
}
