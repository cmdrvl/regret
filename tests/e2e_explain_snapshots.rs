#[path = "fixtures/mod.rs"]
mod fixtures;
mod snapshot_utils;

use fixtures::build_fixture_repo;
use snapshot_utils::{assert_snapshot, run_regret};

#[test]
fn explain_output_snapshot() {
    let repo = build_fixture_repo().expect("build fixture repo");
    let sha_arg = format!("sha:{}", repo.meta.canonical_culprit);
    let _ = run_regret(repo.path.as_path(), &["--scan", "--since", "45d"]);
    let output = run_regret(
        repo.path.as_path(),
        &[
            "--since",
            "30d",
            "--until",
            "2024-01-02T00:00:00Z",
            sha_arg.as_str(),
        ],
    );
    assert_snapshot("explain", &output);
}
