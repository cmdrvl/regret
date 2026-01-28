use crate::fast_path;
use crate::patch_id::{self, PatchId};
use crate::shallow;
use crate::signals::{self, DetectedSignal};
use crate::store::{CommitRow, Store};
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use git2::{Oid, Repository, Sort};
use std::path::Path;

const SUBJECT_TRUNCATE_LEN: usize = 80;
const META_LAST_SCANNED_GRAPH_TIP: &str = "last_scanned_graph_tip";
const META_LAST_SCANNED_REF_OID: &str = "last_scanned_ref_oid";
const META_CACHE_VALID: &str = "cache_valid";
const META_COVERAGE_VALID: &str = "coverage_valid";
pub(crate) const META_COVERAGE_SINCE_UTC: &str = "coverage_since_utc";
const META_COVERAGE_SINCE_OID: &str = "coverage_since_oid";
const META_IS_SHALLOW: &str = "is_shallow";

/// Result of resolving a SHA prefix against the repository.
enum ShaResolution {
    /// Prefix resolved to a unique full SHA
    Unique(String),
    /// Prefix was ambiguous (multiple matches)
    Ambiguous,
    /// Prefix did not match any commit
    NotFound,
}

/// Resolve a SHA prefix (7-40 chars) to a full SHA.
/// Returns Unique if exactly one commit matches, Ambiguous if multiple, NotFound if none.
fn resolve_sha_prefix(repo: &Repository, prefix: &str) -> ShaResolution {
    // Try to parse as a full or partial OID
    let oid = match Oid::from_str(prefix) {
        Ok(oid) => oid,
        Err(_) => return ShaResolution::NotFound,
    };

    // For full 40-char SHAs, just check if the commit exists
    if prefix.len() == 40 {
        return match repo.find_commit(oid) {
            Ok(_) => ShaResolution::Unique(prefix.to_lowercase()),
            Err(_) => ShaResolution::NotFound,
        };
    }

    // For prefixes, use revparse_single which handles ambiguity
    match repo.revparse_single(prefix) {
        Ok(obj) => {
            if obj.kind() == Some(git2::ObjectType::Commit) {
                ShaResolution::Unique(obj.id().to_string())
            } else {
                ShaResolution::NotFound
            }
        }
        Err(e) if e.code() == git2::ErrorCode::Ambiguous => ShaResolution::Ambiguous,
        Err(_) => ShaResolution::NotFound,
    }
}

pub(crate) struct ScanSummary {
    pub(crate) new_commits: usize,
    #[allow(dead_code)]
    pub(crate) ref_oid: String,
    #[allow(dead_code)]
    pub(crate) rewrite_detected: bool,
}

pub(crate) fn incremental_scan(
    repo_root: &Path,
    store: &mut Store,
    ref_name: &str,
) -> Result<ScanSummary> {
    let repo = Repository::discover(repo_root).with_context(|| {
        format!(
            "error: unable to open git repository {}",
            repo_root.display()
        )
    })?;
    let current_oid = fast_path::resolve_ref_oid(&repo, ref_name)?
        .ok_or_else(|| anyhow!("error: unable to resolve ref {}", ref_name))?;

    if detect_rewrite(&repo, store, current_oid)? {
        store.set_meta_bool(META_COVERAGE_VALID, false)?;
        store.set_meta_bool(META_CACHE_VALID, false)?;
        store.set_meta_value(META_LAST_SCANNED_GRAPH_TIP, "")?;
        store.set_meta_value(META_LAST_SCANNED_REF_OID, "")?;
        return Ok(ScanSummary {
            new_commits: 0,
            ref_oid: current_oid.to_string(),
            rewrite_detected: true,
        });
    }

    let mut revwalk = repo.revwalk()?;
    revwalk.set_sorting(Sort::TOPOLOGICAL)?;
    revwalk.push(current_oid)?;

    let last_tip = store.get_meta_value(META_LAST_SCANNED_GRAPH_TIP)?;
    if let Some(last_tip) = last_tip.as_deref().filter(|value| !value.is_empty()) {
        let last_oid = Oid::from_str(last_tip)
            .map_err(|_| anyhow!("error: invalid last_scanned_graph_tip {}", last_tip))?;
        revwalk.hide(last_oid).ok();
    }

    let mut rows: Vec<CommitRow> = Vec::new();
    // Pending canonical revert signals: (evidence_sha, evidence_time, culprit_shas)
    let mut pending_revert_signals: Vec<(String, String, Vec<String>)> = Vec::new();
    // Pending linked-fix signals: (evidence_sha, evidence_time, sha_prefixes)
    let mut pending_linked_fix_signals: Vec<(String, String, Vec<String>)> = Vec::new();

    for oid_result in revwalk {
        let oid = oid_result?;
        let commit = repo.find_commit(oid)?;

        let time_utc = commit_time_utc(&commit)?;
        let subject = commit.summary().map(truncate_subject);
        let pr_number = subject.as_ref().and_then(|s| parse_pr_number(s));
        let pr_source = pr_number.as_ref().map(|_| "merge_commit".to_string());

        // Detect signals in commit body
        if let Some(body) = commit.body_bytes() {
            // Canonical revert lines (§8.1)
            let culprit_shas = signals::detect_canonical_reverts(body);
            if !culprit_shas.is_empty() {
                pending_revert_signals.push((oid.to_string(), time_utc.clone(), culprit_shas));
            }

            // Linked-fix trailers (§8.3)
            let trailers = signals::detect_linked_fix_trailers(body);
            if !trailers.is_empty() {
                let sha_prefixes: Vec<String> =
                    trailers.into_iter().map(|t| t.sha_prefix).collect();
                pending_linked_fix_signals.push((oid.to_string(), time_utc.clone(), sha_prefixes));
            }
        }

        rows.push(CommitRow {
            sha: oid.to_string(),
            time_utc,
            subject,
            pr_number,
            pr_source,
        });
    }

    store.upsert_commits(&rows)?;

    // Process pending signals now that commits are in the store
    let mut detected_signals: Vec<DetectedSignal> = Vec::new();

    // Process canonical revert signals (§8.1)
    for (evidence_sha, evidence_time_utc, culprit_shas) in pending_revert_signals {
        for culprit_sha in culprit_shas {
            // Skip if signal already exists
            if store.signal_exists(&culprit_sha, &evidence_sha)? {
                continue;
            }

            // Look up culprit time from store
            if let Some(culprit_time_utc) = store.get_commit_time(&culprit_sha)? {
                if let Ok(signal) = signals::create_canonical_revert_signal(
                    culprit_sha,
                    &culprit_time_utc,
                    evidence_sha.clone(),
                    &evidence_time_utc,
                ) {
                    detected_signals.push(signal);
                }
            }
            // If culprit not in store, we skip (it may be outside our scan window)
        }
    }

    // Process linked-fix signals (§8.3)
    for (evidence_sha, evidence_time_utc, sha_prefixes) in pending_linked_fix_signals {
        for sha_prefix in sha_prefixes {
            // Resolve SHA prefix against repository
            let culprit_sha = match resolve_sha_prefix(&repo, &sha_prefix) {
                ShaResolution::Unique(sha) => sha,
                ShaResolution::Ambiguous => {
                    // Per spec: ambiguous prefix = NO signal, debug diagnostic only
                    // TODO: Add --debug diagnostic output
                    continue;
                }
                ShaResolution::NotFound => {
                    // Prefix doesn't match any commit in repo
                    continue;
                }
            };

            // Skip if signal already exists
            if store.signal_exists(&culprit_sha, &evidence_sha)? {
                continue;
            }

            // Look up culprit time from store
            if let Some(culprit_time_utc) = store.get_commit_time(&culprit_sha)? {
                if let Ok(signal) = signals::create_linked_fix_signal(
                    culprit_sha,
                    &culprit_time_utc,
                    evidence_sha.clone(),
                    &evidence_time_utc,
                ) {
                    detected_signals.push(signal);
                }
            }
            // If culprit not in store, we skip (it may be outside our scan window)
        }
    }

    // Insert detected signals from canonical reverts and linked-fix trailers
    if !detected_signals.is_empty() {
        store.insert_signals(ref_name, &detected_signals)?;
    }

    // Run patch-ID matching for manual reverts (§8.2)
    let patch_id_signals = run_patch_id_matching(&repo, store, ref_name)?;
    if !patch_id_signals.is_empty() {
        store.insert_signals(ref_name, &patch_id_signals)?;
    }

    fast_path::set_last_scanned_oids(store, &current_oid.to_string(), &current_oid.to_string())?;
    store.set_meta_bool(META_CACHE_VALID, true)?;
    store.set_meta_bool(META_COVERAGE_VALID, true)?;

    let shallow_oids = shallow::shallow_boundary_oids_from_repo(&repo)?;
    let is_shallow = repo.is_shallow() || !shallow_oids.is_empty();
    store.set_meta_bool(META_IS_SHALLOW, is_shallow)?;

    if is_shallow {
        if let Some((sha, time_utc)) = earliest_boundary(&repo, &shallow_oids)? {
            store.set_meta_value(META_COVERAGE_SINCE_OID, &sha)?;
            store.set_meta_value(META_COVERAGE_SINCE_UTC, &time_utc)?;
        }
    } else if store.get_meta_value(META_COVERAGE_SINCE_UTC)?.is_none() || last_tip.is_none() {
        if let Some((sha, time_utc)) = store.get_oldest_commit()? {
            store.set_meta_value(META_COVERAGE_SINCE_OID, &sha)?;
            store.set_meta_value(META_COVERAGE_SINCE_UTC, &time_utc)?;
        }
    }

    Ok(ScanSummary {
        new_commits: rows.len(),
        ref_oid: current_oid.to_string(),
        rewrite_detected: false,
    })
}

/// Run patch-ID matching algorithm (§8.2) to find manual reverts.
///
/// Algorithm:
/// A) Evidence candidates: commits with "revert" or "rollback" in subject
/// C) Match when patch_id(E) == patch_id_rev(C)
/// D) Search bounds: C.time_utc <= E.time_utc && C.time_utc >= coverage_since_utc
/// E) Collision resolution: choose max(C.time_utc), tie-break lexical SHA
fn run_patch_id_matching(
    repo: &Repository,
    store: &mut Store,
    _ref_name: &str,
) -> Result<Vec<DetectedSignal>> {
    let mut detected_signals = Vec::new();

    // Get coverage start time
    let coverage_since_utc = match store.get_meta_value(META_COVERAGE_SINCE_UTC)? {
        Some(time) if !time.is_empty() => time,
        _ => return Ok(detected_signals), // No coverage, skip
    };

    // Get evidence candidates (commits with revert/rollback in subject)
    let evidence_candidates = store.get_evidence_candidates()?;

    for (evidence_sha, evidence_time_utc, _subject) in evidence_candidates {
        // Skip if we already have a signal for this evidence (from canonical revert line)
        // We check by looking for any signal with this evidence_sha
        // Note: This is a simplified check - in production might want a dedicated method
        if has_revert_signal_for_evidence(store, &evidence_sha)? {
            continue;
        }

        // Compute patch_id for evidence
        let evidence_patch_id = match compute_and_cache_patch_id(repo, store, &evidence_sha)? {
            Some(pid) => pid,
            None => continue, // Empty diff, skip
        };

        // Get potential culprits (commits before evidence, after coverage start)
        let culprits = store.get_potential_culprits(&evidence_time_utc, &coverage_since_utc)?;

        // Find matching culprit (first match is best due to ordering: time desc, sha asc)
        let mut best_match: Option<(String, String)> = None;

        for (culprit_sha, culprit_time_utc) in culprits {
            // Skip the evidence commit itself
            if culprit_sha == evidence_sha {
                continue;
            }

            // Skip if signal already exists
            if store.signal_exists(&culprit_sha, &evidence_sha)? {
                continue;
            }

            // Compute patch_id_rev for culprit
            let culprit_patch_id_rev =
                match compute_and_cache_patch_id_rev(repo, store, &culprit_sha)? {
                    Some(pid) => pid,
                    None => continue, // Empty diff, skip
                };

            // Check for match
            if evidence_patch_id == culprit_patch_id_rev {
                best_match = Some((culprit_sha, culprit_time_utc));
                break; // First match is best due to ordering
            }
        }

        // Emit signal for best match
        if let Some((culprit_sha, culprit_time_utc)) = best_match {
            if let Ok(signal) = signals::create_patch_id_revert_signal(
                culprit_sha,
                &culprit_time_utc,
                evidence_sha,
                &evidence_time_utc,
            ) {
                detected_signals.push(signal);
            }
        }
    }

    Ok(detected_signals)
}

/// Check if there's already a revert signal for this evidence commit.
fn has_revert_signal_for_evidence(store: &Store, evidence_sha: &str) -> Result<bool> {
    store.has_revert_signal_for_evidence(evidence_sha)
}

/// Compute and cache patch_id for a commit.
fn compute_and_cache_patch_id(
    repo: &Repository,
    store: &mut Store,
    sha: &str,
) -> Result<Option<PatchId>> {
    // Check cache first
    if let Some(bytes) = store.get_patch_id(sha)? {
        return Ok(Some(PatchId(bytes)));
    }

    // Compute
    let patch_id = patch_id::compute_patch_id(repo, sha)?;

    // Cache if computed
    if let Some(ref pid) = patch_id {
        store.set_patch_id(sha, pid.as_bytes())?;
    }

    Ok(patch_id)
}

/// Compute and cache patch_id_rev for a commit.
fn compute_and_cache_patch_id_rev(
    repo: &Repository,
    store: &mut Store,
    sha: &str,
) -> Result<Option<PatchId>> {
    // Check cache first
    if let Some(bytes) = store.get_patch_id_rev(sha)? {
        return Ok(Some(PatchId(bytes)));
    }

    // Compute
    let patch_id_rev = patch_id::compute_patch_id_rev(repo, sha)?;

    // Cache if computed
    if let Some(ref pid) = patch_id_rev {
        store.set_patch_id_rev(sha, pid.as_bytes())?;
    }

    Ok(patch_id_rev)
}

fn commit_time_utc(commit: &git2::Commit<'_>) -> Result<String> {
    let time = commit.time();
    let seconds = time.seconds();
    let dt = DateTime::<Utc>::from_timestamp(seconds, 0)
        .ok_or_else(|| anyhow!("error: invalid commit timestamp {}", seconds))?;
    Ok(dt.to_rfc3339())
}

fn earliest_boundary(repo: &Repository, oids: &[Oid]) -> Result<Option<(String, String)>> {
    let mut earliest: Option<(i64, String, String)> = None;
    for oid in oids {
        let commit = repo.find_commit(*oid)?;
        let time_utc = commit_time_utc(&commit)?;
        let seconds = commit.time().seconds();
        match earliest {
            Some((best, _, _)) if seconds >= best => {}
            _ => {
                earliest = Some((seconds, oid.to_string(), time_utc));
            }
        }
    }

    Ok(earliest.map(|(_, oid, time)| (oid, time)))
}

fn truncate_subject(subject: &str) -> String {
    subject.chars().take(SUBJECT_TRUNCATE_LEN).collect()
}

fn parse_pr_number(subject: &str) -> Option<i64> {
    let prefix = "Merge pull request #";
    let rest = subject.strip_prefix(prefix)?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

fn detect_rewrite(repo: &Repository, store: &Store, current_oid: Oid) -> Result<bool> {
    let last_oid = match store.get_meta_value(META_LAST_SCANNED_REF_OID)? {
        Some(value) if !value.is_empty() => value,
        _ => return Ok(false),
    };
    let last_oid = Oid::from_str(&last_oid)
        .map_err(|_| anyhow!("error: invalid last_scanned_ref_oid {}", last_oid))?;

    if last_oid == current_oid {
        return Ok(false);
    }

    let base = match repo.merge_base(current_oid, last_oid) {
        Ok(base) => base,
        Err(_) => return Ok(true),
    };

    Ok(base != last_oid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selected_branch;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::tempdir;

    fn real_path(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    fn run_git(repo: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("git command failed");
        assert!(output.status.success(), "git {:?} failed", args);
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn init_repo(path: &Path) {
        run_git(path, &["init"]);
        run_git(path, &["config", "user.email", "test@example.com"]);
        run_git(path, &["config", "user.name", "Test User"]);
        std::fs::write(path.join("README.md"), "hello").unwrap();
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", "init"]);
        run_git(path, &["branch", "-m", "main"]);
    }

    #[test]
    fn incremental_scan_processes_new_commits_only() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        init_repo(&base);
        let root_oid = run_git(&base, &["rev-list", "--max-parents=0", "HEAD"]);

        let cache_dir = base.join(".regret");
        let mut store = Store::open(&cache_dir).unwrap();
        let selected = selected_branch::ensure_selected_branch(&base, &store).unwrap();

        let first = incremental_scan(&base, &mut store, &selected).unwrap();
        assert!(first.new_commits > 0);
        let coverage_oid = store.get_meta_value("coverage_since_oid").unwrap();
        assert_eq!(coverage_oid.as_deref(), Some(root_oid.as_str()));
        let coverage_utc = store.get_meta_value("coverage_since_utc").unwrap();
        assert!(coverage_utc.is_some());

        let second = incremental_scan(&base, &mut store, &selected).unwrap();
        assert_eq!(second.new_commits, 0);
        let coverage_oid_again = store.get_meta_value("coverage_since_oid").unwrap();
        assert_eq!(coverage_oid_again.as_deref(), Some(root_oid.as_str()));

        std::fs::write(base.join("CHANGELOG.md"), "change").unwrap();
        run_git(&base, &["add", "."]);
        run_git(&base, &["commit", "-m", "change"]);

        let third = incremental_scan(&base, &mut store, &selected).unwrap();
        assert_eq!(third.new_commits, 1);
        let coverage_oid_after = store.get_meta_value("coverage_since_oid").unwrap();
        assert_eq!(coverage_oid_after.as_deref(), Some(root_oid.as_str()));
    }

    #[test]
    fn incremental_scan_bounds_coverage_for_shallow_repo() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        init_repo(&base);
        let root_oid = run_git(&base, &["rev-list", "--max-parents=0", "HEAD"]);

        let repo = Repository::discover(&base).unwrap();
        let shallow_path = repo.path().join("shallow");
        std::fs::write(&shallow_path, format!("{}\n", root_oid)).unwrap();

        let cache_dir = base.join(".regret");
        let mut store = Store::open(&cache_dir).unwrap();
        let selected = selected_branch::ensure_selected_branch(&base, &store).unwrap();

        incremental_scan(&base, &mut store, &selected).unwrap();

        let is_shallow = store.get_meta_bool("is_shallow").unwrap().unwrap_or(false);
        assert!(is_shallow);
        let coverage_oid = store.get_meta_value("coverage_since_oid").unwrap();
        assert_eq!(coverage_oid.as_deref(), Some(root_oid.as_str()));
    }

    #[test]
    fn detects_rewrite_when_head_not_descendant() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        init_repo(&base);

        std::fs::write(base.join("CHANGELOG.md"), "change").unwrap();
        run_git(&base, &["add", "."]);
        run_git(&base, &["commit", "-m", "change"]);

        let cache_dir = base.join(".regret");
        let mut store = Store::open(&cache_dir).unwrap();
        let selected = selected_branch::ensure_selected_branch(&base, &store).unwrap();

        let first = incremental_scan(&base, &mut store, &selected).unwrap();
        assert!(!first.rewrite_detected);
        let first_head = run_git(&base, &["rev-parse", "HEAD"]);

        // Create new history by resetting to root and recommitting
        let root = run_git(&base, &["rev-list", "--max-parents=0", "HEAD"]);
        run_git(&base, &["reset", "--hard", root.as_str()]);
        std::fs::write(base.join("NEWFILE.md"), "rewrite").unwrap();
        run_git(&base, &["add", "."]);
        run_git(&base, &["commit", "-m", "rewrite"]);

        let second = incremental_scan(&base, &mut store, &selected).unwrap();
        assert!(second.rewrite_detected);

        let head_after = run_git(&base, &["rev-parse", "HEAD"]);
        assert_ne!(first_head, head_after);
    }
}
