#[path = "fixtures/mod.rs"]
mod fixtures;
mod snapshot_utils;

use fixtures::build_fixture_repo;
use snapshot_utils::{assert_snapshot, run_regret};

#[test]
fn table_output_snapshot() {
    let repo = build_fixture_repo().expect("build fixture repo");
    let output = run_regret(
        repo.path.as_path(),
        &["--since", "30d", "--until", "2024-01-02", "--table"],
    );
    assert_snapshot("table", &output);
}
