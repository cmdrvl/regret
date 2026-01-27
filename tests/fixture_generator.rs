#[path = "fixtures/mod.rs"]
mod fixtures;

use fixtures::build_fixture_repo;

#[test]
fn fixture_repo_is_valid_git() {
    let repo = build_fixture_repo().expect("build fixture repo");
    let inside = repo
        .git(&["rev-parse", "--is-inside-work-tree"])
        .expect("git rev-parse");
    assert_eq!(inside, "true");
}

#[test]
fn fixture_canonical_revert_line_present() {
    let repo = build_fixture_repo().expect("build fixture repo");
    let message = repo
        .git(&["show", "-s", "--format=%B", &repo.meta.canonical_evidence])
        .expect("read commit message");
    let expected = format!("This reverts commit {}.", repo.meta.canonical_culprit);
    assert!(message.contains(&expected));
}

#[test]
fn fixture_is_deterministic() {
    let repo_a = build_fixture_repo().expect("build fixture repo");
    let repo_b = build_fixture_repo().expect("build fixture repo");

    assert_eq!(repo_a.meta.canonical_culprit, repo_b.meta.canonical_culprit);
    assert_eq!(
        repo_a.meta.canonical_evidence,
        repo_b.meta.canonical_evidence
    );
    assert_eq!(repo_a.meta.manual_culprit, repo_b.meta.manual_culprit);
    assert_eq!(repo_a.meta.manual_evidence, repo_b.meta.manual_evidence);
    assert_eq!(
        repo_a.meta.linked_fix_culprit,
        repo_b.meta.linked_fix_culprit
    );
    assert_eq!(
        repo_a.meta.linked_fix_evidence,
        repo_b.meta.linked_fix_evidence
    );
    assert_eq!(repo_a.meta.ambiguous_prefix, repo_b.meta.ambiguous_prefix);
    assert_eq!(
        repo_a.meta.ambiguous_culprit_a,
        repo_b.meta.ambiguous_culprit_a
    );
    assert_eq!(
        repo_a.meta.ambiguous_culprit_b,
        repo_b.meta.ambiguous_culprit_b
    );
    assert_eq!(
        repo_a.meta.ambiguous_evidence,
        repo_b.meta.ambiguous_evidence
    );
    assert_eq!(repo_a.meta.crlf_commit, repo_b.meta.crlf_commit);
    assert_eq!(repo_a.meta.rewrite_base, repo_b.meta.rewrite_base);
    assert_eq!(repo_a.meta.rewrite_head, repo_b.meta.rewrite_head);
    assert_eq!(repo_a.meta.main_head, repo_b.meta.main_head);
}
