//! Signal detection for regret events.
//!
//! This module implements detection of:
//! - Canonical revert lines (§8.1): "This reverts commit <sha>."
//! - Patch-ID revert matching (§8.2): manual reverts via patch equivalence
//! - Linked-fix trailers (§8.3): Fixes-Commit/Fixes-SHA trailers

use anyhow::Result;
use chrono::{DateTime, Utc};
use regex::bytes::Regex;
use std::sync::LazyLock;

/// Confidence value for canonical revert line detection (§8.1)
pub const CONFIDENCE_CANONICAL_REVERT: f64 = 0.95;

/// Confidence value for patch-ID equivalence detection (§8.2)
pub const CONFIDENCE_PATCH_ID: f64 = 0.90;

/// Confidence value for explicit trailer detection (§8.3)
pub const CONFIDENCE_LINKED_FIX: f64 = 0.92;

/// Default weight for revert signals
pub const WEIGHT_REVERT: i64 = 10;

/// Default weight for linked_fix signals
pub const WEIGHT_LINKED_FIX: i64 = 8;

/// Type of signal detected
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalType {
    Revert,
    LinkedFix,
}

impl SignalType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignalType::Revert => "revert",
            SignalType::LinkedFix => "linked_fix",
        }
    }
}

/// Reason for confidence assignment
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfidenceReason {
    CanonicalRevertLine,
    PatchIdEquivalence,
    ExplicitTrailer,
}

impl ConfidenceReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConfidenceReason::CanonicalRevertLine => "canonical_revert_line",
            ConfidenceReason::PatchIdEquivalence => "patch_id_equivalence",
            ConfidenceReason::ExplicitTrailer => "explicit_trailer",
        }
    }
}

/// A detected signal before storage
#[derive(Debug, Clone)]
pub struct DetectedSignal {
    pub signal_type: SignalType,
    pub culprit_sha: String,
    pub evidence_sha: String,
    pub evidence_time_utc: String,
    pub confidence: f64,
    pub confidence_reason: ConfidenceReason,
    pub weight: i64,
    pub time_to_regret_hours: f64,
}

/// Regex for canonical revert line: "This reverts commit <40-hex-sha>."
/// Matches the exact git revert format.
static CANONICAL_REVERT_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"This reverts commit ([0-9a-fA-F]{40})\.").expect("invalid regex")
});

/// Regex for linked-fix trailers: "Fixes-Commit: <sha>" or "Fixes-SHA: <sha>"
/// Accepts 7-40 hex character prefixes (will be resolved against repo).
/// The pattern matches trailers at the start of a line.
static LINKED_FIX_TRAILER_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^Fixes-(Commit|SHA):\s*([0-9a-fA-F]{7,40})\s*$").expect("invalid regex")
});

/// Detect canonical revert lines in a commit body.
///
/// Per §8.1: Pattern is "This reverts commit <40-hex-sha>."
/// - Byte scan commit body (stream-parsed, not persisted)
/// - Extract culprit SHA from pattern
/// - Validate SHA is 40 hex characters
///
/// Returns all detected culprit SHAs (handles multiple reverts in one commit).
pub fn detect_canonical_reverts(body: &[u8]) -> Vec<String> {
    let mut culprits = Vec::new();

    for cap in CANONICAL_REVERT_REGEX.captures_iter(body) {
        if let Some(sha_match) = cap.get(1) {
            // The regex ensures 40 hex chars, but let's be safe
            let sha_bytes = sha_match.as_bytes();
            if sha_bytes.len() == 40 && sha_bytes.iter().all(|b| b.is_ascii_hexdigit()) {
                // Convert to lowercase for consistency
                let sha = String::from_utf8_lossy(sha_bytes).to_lowercase();
                culprits.push(sha);
            }
        }
    }

    culprits
}

/// A linked-fix trailer found in a commit message.
/// The SHA prefix needs to be resolved against the repository.
#[derive(Debug, Clone)]
pub struct LinkedFixTrailer {
    /// The SHA prefix (7-40 hex characters, lowercase)
    pub sha_prefix: String,
}

/// Detect linked-fix trailers in a commit body.
///
/// Per §8.3: Trailers are "Fixes-Commit: <sha>" or "Fixes-SHA: <sha>"
/// - Accept 7-40 hex prefixes
/// - Must be resolved against local repo (caller responsibility)
/// - Ambiguous prefix: NO signal (caller handles)
///
/// Returns all detected trailers for resolution.
pub fn detect_linked_fix_trailers(body: &[u8]) -> Vec<LinkedFixTrailer> {
    let mut trailers = Vec::new();

    for cap in LINKED_FIX_TRAILER_REGEX.captures_iter(body) {
        if let Some(sha_match) = cap.get(2) {
            let sha_bytes = sha_match.as_bytes();
            // Validate hex characters (regex should ensure this, but be safe)
            if sha_bytes.len() >= 7
                && sha_bytes.len() <= 40
                && sha_bytes.iter().all(|b| b.is_ascii_hexdigit())
            {
                let sha = String::from_utf8_lossy(sha_bytes).to_lowercase();
                trailers.push(LinkedFixTrailer { sha_prefix: sha });
            }
        }
    }

    trailers
}

/// Create a linked-fix signal from resolved data.
pub fn create_linked_fix_signal(
    culprit_sha: String,
    culprit_time_utc: &str,
    evidence_sha: String,
    evidence_time_utc: &str,
) -> Result<DetectedSignal> {
    let time_to_regret_hours =
        calculate_time_to_regret_hours(culprit_time_utc, evidence_time_utc)?;

    Ok(DetectedSignal {
        signal_type: SignalType::LinkedFix,
        culprit_sha,
        evidence_sha,
        evidence_time_utc: evidence_time_utc.to_string(),
        confidence: CONFIDENCE_LINKED_FIX,
        confidence_reason: ConfidenceReason::ExplicitTrailer,
        weight: WEIGHT_LINKED_FIX,
        time_to_regret_hours,
    })
}

/// Calculate time-to-regret in hours between culprit and evidence commits.
pub fn calculate_time_to_regret_hours(
    culprit_time_utc: &str,
    evidence_time_utc: &str,
) -> Result<f64> {
    let culprit_dt = DateTime::parse_from_rfc3339(culprit_time_utc)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| culprit_time_utc.parse::<DateTime<Utc>>())?;

    let evidence_dt = DateTime::parse_from_rfc3339(evidence_time_utc)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| evidence_time_utc.parse::<DateTime<Utc>>())?;

    let duration = evidence_dt.signed_duration_since(culprit_dt);
    let hours = duration.num_seconds() as f64 / 3600.0;

    Ok(hours.max(0.0)) // Ensure non-negative
}

/// Create a canonical revert signal from detected data.
pub fn create_canonical_revert_signal(
    culprit_sha: String,
    culprit_time_utc: &str,
    evidence_sha: String,
    evidence_time_utc: &str,
) -> Result<DetectedSignal> {
    let time_to_regret_hours =
        calculate_time_to_regret_hours(culprit_time_utc, evidence_time_utc)?;

    Ok(DetectedSignal {
        signal_type: SignalType::Revert,
        culprit_sha,
        evidence_sha,
        evidence_time_utc: evidence_time_utc.to_string(),
        confidence: CONFIDENCE_CANONICAL_REVERT,
        confidence_reason: ConfidenceReason::CanonicalRevertLine,
        weight: WEIGHT_REVERT,
        time_to_regret_hours,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_standard_git_revert_message() {
        let body = b"Revert \"Add feature X\"\n\nThis reverts commit abc1234567890def1234567890abc123456789de.\n";
        let culprits = detect_canonical_reverts(body);
        assert_eq!(culprits.len(), 1);
        assert_eq!(
            culprits[0],
            "abc1234567890def1234567890abc123456789de"
        );
    }

    #[test]
    fn detects_uppercase_sha() {
        let body = b"This reverts commit ABC1234567890DEF1234567890ABC123456789DE.";
        let culprits = detect_canonical_reverts(body);
        assert_eq!(culprits.len(), 1);
        // Should be lowercased
        assert_eq!(
            culprits[0],
            "abc1234567890def1234567890abc123456789de"
        );
    }

    #[test]
    fn detects_multiple_reverts_in_one_commit() {
        // Two canonical revert lines in one commit message (both start with "This")
        let body = b"This reverts commit 1111111111111111111111111111111111111111.\n\
                     This reverts commit 2222222222222222222222222222222222222222.\n";
        let culprits = detect_canonical_reverts(body);
        assert_eq!(culprits.len(), 2);
        assert_eq!(culprits[0], "1111111111111111111111111111111111111111");
        assert_eq!(culprits[1], "2222222222222222222222222222222222222222");
    }

    #[test]
    fn ignores_manual_message_without_pattern() {
        let body = b"Revert the feature because it broke production";
        let culprits = detect_canonical_reverts(body);
        assert!(culprits.is_empty());
    }

    #[test]
    fn ignores_malformed_sha_too_short() {
        let body = b"This reverts commit abc123.";
        let culprits = detect_canonical_reverts(body);
        assert!(culprits.is_empty());
    }

    #[test]
    fn ignores_malformed_sha_too_long() {
        let body = b"This reverts commit abc1234567890def1234567890abc1234567890def0.";
        let culprits = detect_canonical_reverts(body);
        assert!(culprits.is_empty());
    }

    #[test]
    fn ignores_missing_period() {
        let body = b"This reverts commit abc1234567890def1234567890abc1234567890de";
        let culprits = detect_canonical_reverts(body);
        assert!(culprits.is_empty());
    }

    #[test]
    fn ignores_non_hex_characters() {
        let body = b"This reverts commit gggg111111111111111111111111111111111111.";
        let culprits = detect_canonical_reverts(body);
        assert!(culprits.is_empty());
    }

    #[test]
    fn time_to_regret_calculation() {
        let culprit = "2024-01-01T10:00:00Z";
        let evidence = "2024-01-01T12:30:00Z";
        let hours = calculate_time_to_regret_hours(culprit, evidence).unwrap();
        assert!((hours - 2.5).abs() < 0.001);
    }

    #[test]
    fn time_to_regret_negative_becomes_zero() {
        // Evidence before culprit (shouldn't happen, but handle gracefully)
        let culprit = "2024-01-01T12:00:00Z";
        let evidence = "2024-01-01T10:00:00Z";
        let hours = calculate_time_to_regret_hours(culprit, evidence).unwrap();
        assert_eq!(hours, 0.0);
    }

    // Linked-fix trailer tests

    #[test]
    fn detects_fixes_commit_trailer_full_sha() {
        let body = b"Fix the bug\n\nFixes-Commit: abc1234567890def1234567890abc123456789de\n";
        let trailers = detect_linked_fix_trailers(body);
        assert_eq!(trailers.len(), 1);
        assert_eq!(
            trailers[0].sha_prefix,
            "abc1234567890def1234567890abc123456789de"
        );
    }

    #[test]
    fn detects_fixes_sha_trailer() {
        let body = b"Fix the bug\n\nFixes-SHA: abc1234567890def1234567890abc123456789de\n";
        let trailers = detect_linked_fix_trailers(body);
        assert_eq!(trailers.len(), 1);
        assert_eq!(
            trailers[0].sha_prefix,
            "abc1234567890def1234567890abc123456789de"
        );
    }

    #[test]
    fn detects_short_sha_prefix() {
        let body = b"Fix the bug\n\nFixes-Commit: abc1234\n";
        let trailers = detect_linked_fix_trailers(body);
        assert_eq!(trailers.len(), 1);
        assert_eq!(trailers[0].sha_prefix, "abc1234");
    }

    #[test]
    fn detects_multiple_linked_fix_trailers() {
        let body = b"Fix multiple bugs\n\nFixes-Commit: 1111111\nFixes-SHA: 2222222\n";
        let trailers = detect_linked_fix_trailers(body);
        assert_eq!(trailers.len(), 2);
        assert_eq!(trailers[0].sha_prefix, "1111111");
        assert_eq!(trailers[1].sha_prefix, "2222222");
    }

    #[test]
    fn ignores_sha_prefix_too_short() {
        let body = b"Fix the bug\n\nFixes-Commit: abc123\n"; // 6 chars, need at least 7
        let trailers = detect_linked_fix_trailers(body);
        assert!(trailers.is_empty());
    }

    #[test]
    fn ignores_inline_trailer_not_at_line_start() {
        let body = b"Fix the bug with Fixes-Commit: abc1234567\n";
        let trailers = detect_linked_fix_trailers(body);
        assert!(trailers.is_empty());
    }

    #[test]
    fn handles_whitespace_after_sha() {
        let body = b"Fix the bug\n\nFixes-Commit: abc1234567   \n";
        let trailers = detect_linked_fix_trailers(body);
        assert_eq!(trailers.len(), 1);
        assert_eq!(trailers[0].sha_prefix, "abc1234567");
    }

    #[test]
    fn lowercases_sha_prefix() {
        let body = b"Fix the bug\n\nFixes-Commit: ABC1234DEF\n";
        let trailers = detect_linked_fix_trailers(body);
        assert_eq!(trailers.len(), 1);
        assert_eq!(trailers[0].sha_prefix, "abc1234def");
    }
}
