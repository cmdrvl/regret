//! Output formatting for regret CLI.
//!
//! This module implements human-readable output per §10.1:
//! - Header line (§10.1.1)
//! - TOP culprits table (§10.1.2)

use chrono::{DateTime, Utc};

/// Summary information for the header line.
pub struct HeaderInfo {
    pub tool_version: String,
    pub repo_basename: String,
    pub selected_branch: String,
    pub scan_status: ScanStatus,
    pub new_commits: usize,
    pub scanned_commits: usize,
    pub coverage_days: Option<u64>,
}

/// Whether scanning was skipped or ran.
#[derive(Debug, Clone, Copy)]
pub enum ScanStatus {
    Skipped,
    Ran,
}

impl ScanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScanStatus::Skipped => "skipped",
            ScanStatus::Ran => "ran",
        }
    }
}

/// A ranked culprit for the TOP table.
#[derive(Debug, Clone)]
pub struct RankedCulprit {
    /// Unique identifier (row number)
    pub id: usize,
    /// Aggregated score = sum of weights for signals in window
    pub score: i64,
    /// Number of evidence events
    pub events: usize,
    /// Median time-to-regret in hours
    pub ttr_p50_hours: f64,
    /// Culprit commit SHA (full 40 chars)
    pub culprit_sha: String,
    /// Culprit commit time
    pub culprit_date_utc: String,
    /// Human-readable age (e.g., "3d", "2w")
    pub culprit_age: String,
    /// Subject line, truncated
    pub subject_trunc: String,
    /// Minimum confidence of signals
    pub min_confidence: f64,
}

/// Print the header line per §10.1.1.
///
/// Format: regret <version> repo=<name> branch=<branch> scan=<status> new_commits=<n> scanned_commits=<n> coverage_days=<n>
pub fn print_header(info: &HeaderInfo) {
    let coverage_str = match info.coverage_days {
        Some(days) => days.to_string(),
        None => "?".to_string(),
    };

    println!(
        "regret {} repo={} branch={} scan={} new_commits={} scanned_commits={} coverage_days={}",
        info.tool_version,
        info.repo_basename,
        info.selected_branch,
        info.scan_status.as_str(),
        info.new_commits,
        info.scanned_commits,
        coverage_str,
    );
}

/// Print the TOP culprits table per §10.1.2.
///
/// Columns: id, score, events, ttr_p50_h, culprit_date_utc, culprit_age, subject_trunc, conf
pub fn print_top_table(culprits: &[RankedCulprit], limit: usize) {
    if culprits.is_empty() {
        return;
    }

    // Print header
    println!();
    println!(
        "{:>3} {:>6} {:>6} {:>10} {:>20} {:>8} {:<80} {:>5}",
        "id", "score", "events", "ttr_p50_h", "culprit_date_utc", "age", "subject", "conf"
    );
    println!("{}", "-".repeat(145));

    // Print rows (up to limit)
    for culprit in culprits.iter().take(limit) {
        let short_sha = if culprit.culprit_sha.len() >= 7 {
            &culprit.culprit_sha[..7]
        } else {
            &culprit.culprit_sha
        };

        // Truncate subject at 80 chars
        let subject = truncate_subject(&culprit.subject_trunc, 80);

        println!(
            "{:>3} {:>6} {:>6} {:>10.1} {:>20} {:>8} {:<80} {:>5.2}",
            culprit.id,
            culprit.score,
            culprit.events,
            culprit.ttr_p50_hours,
            format!("{} {}", &culprit.culprit_date_utc[..10], short_sha),
            culprit.culprit_age,
            subject,
            culprit.min_confidence,
        );
    }
}

/// Truncate a subject line to the specified length.
fn truncate_subject(subject: &str, max_len: usize) -> String {
    if subject.len() <= max_len {
        subject.to_string()
    } else {
        format!("{}...", &subject[..max_len - 3])
    }
}

/// Compute human-readable age from a UTC timestamp.
pub fn compute_age(commit_time_utc: &str) -> String {
    let commit_time = match DateTime::parse_from_rfc3339(commit_time_utc) {
        Ok(dt) => dt.with_timezone(&Utc),
        Err(_) => return "?".to_string(),
    };

    let now = Utc::now();
    let duration = now.signed_duration_since(commit_time);

    let hours = duration.num_hours();
    let days = duration.num_days();
    let weeks = days / 7;

    if weeks > 0 {
        format!("{}w", weeks)
    } else if days > 0 {
        format!("{}d", days)
    } else if hours > 0 {
        format!("{}h", hours)
    } else {
        "<1h".to_string()
    }
}

/// Calculate median of a slice of f64 values.
pub fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let len = values.len();
    if len % 2 == 0 {
        (values[len / 2 - 1] + values[len / 2]) / 2.0
    } else {
        values[len / 2]
    }
}

/// Info for RATE line per §10.1.4.
pub struct RateInfo {
    pub events: usize,
    pub commits: usize,
}

/// Print the RATE line per §10.1.4.
///
/// Format: RATE events_per_100_commits=<x> events=<e> commits=<c>
pub fn print_rate(info: &RateInfo) {
    let rate = if info.commits > 0 {
        (info.events as f64 / info.commits as f64) * 100.0
    } else {
        0.0
    };

    println!(
        "RATE events_per_100_commits={:.2} events={} commits={}",
        rate, info.events, info.commits
    );
}

/// Coverage status per §10.1.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageStatus {
    Complete,
    Incomplete,
    Invalid,
}

impl CoverageStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CoverageStatus::Complete => "complete",
            CoverageStatus::Incomplete => "incomplete",
            CoverageStatus::Invalid => "invalid",
        }
    }
}

/// Info for COVERAGE line per §10.1.5.
pub struct CoverageInfo {
    pub status: CoverageStatus,
    pub coverage_since_utc: String,
    pub coverage_valid: bool,
}

/// Print the COVERAGE line per §10.1.5 (only when incomplete or invalid).
///
/// Format: COVERAGE status=<status> coverage_since_utc=<ts> coverage_valid=<0|1>
/// NEXT: regret --scan --since <window> (or --all for invalid)
pub fn print_coverage(info: &CoverageInfo, window_days: Option<u64>) {
    if info.status == CoverageStatus::Complete {
        return;
    }

    println!(
        "COVERAGE status={} coverage_since_utc={} coverage_valid={}",
        info.status.as_str(),
        info.coverage_since_utc,
        if info.coverage_valid { "1" } else { "0" }
    );

    match info.status {
        CoverageStatus::Incomplete => {
            let window = window_days.unwrap_or(30);
            println!("NEXT: regret --scan --since {}d", window);
        }
        CoverageStatus::Invalid => {
            println!("NEXT: regret --scan --all");
        }
        CoverageStatus::Complete => {}
    }
}

/// Reason for NO_EVENTS per §10.1.6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoEventsReason {
    CoverageIncomplete,
    NoSignalsDetected,
    SignalsOutsideWindow,
}

impl NoEventsReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            NoEventsReason::CoverageIncomplete => "coverage_incomplete",
            NoEventsReason::NoSignalsDetected => "no_signals_detected",
            NoEventsReason::SignalsOutsideWindow => "signals_outside_window",
        }
    }
}

/// Info for NO_EVENTS block per §10.1.6.
pub struct NoEventsInfo {
    pub reverts_detected: usize,
    pub linked_fix_trailers: usize,
    pub coverage_days: Option<u64>,
    pub reason: NoEventsReason,
}

/// Print the NO_EVENTS activation block per §10.1.6.
///
/// Format:
/// NO_EVENTS reverts_detected=<n> linked_fix_trailers=<n> coverage_days=<n>
/// REASON: <reason>
/// NEXT: <command>
pub fn print_no_events(info: &NoEventsInfo) {
    let coverage_str = match info.coverage_days {
        Some(days) => days.to_string(),
        None => "?".to_string(),
    };

    println!(
        "NO_EVENTS reverts_detected={} linked_fix_trailers={} coverage_days={}",
        info.reverts_detected, info.linked_fix_trailers, coverage_str
    );
    println!("REASON: {}", info.reason.as_str());

    match info.reason {
        NoEventsReason::CoverageIncomplete => {
            println!("NEXT: regret --scan --all");
        }
        NoEventsReason::NoSignalsDetected => {
            println!("NEXT: git log --oneline | head -20 # verify recent commit activity");
        }
        NoEventsReason::SignalsOutsideWindow => {
            println!("NEXT: regret --since 90d # widen ranking window");
        }
    }
}

/// Aggregate signal row from database.
#[derive(Debug, Clone)]
pub struct SignalRow {
    pub culprit_sha: String,
    pub evidence_sha: String,
    pub weight: i64,
    pub confidence: f64,
    pub time_to_regret_hours: f64,
    pub culprit_time_utc: String,
    pub culprit_subject: Option<String>,
}

/// Aggregate signals into ranked culprits.
///
/// Scoring per §9.1: score = Σ(weight) for signals in window with confidence >= min_confidence
/// Sorting: score desc, then culprit SHA (lexical)
pub fn aggregate_culprits(
    signals: &[SignalRow],
    min_confidence: f64,
) -> Vec<RankedCulprit> {
    use std::collections::HashMap;

    // Group signals by culprit_sha
    let mut by_culprit: HashMap<String, Vec<&SignalRow>> = HashMap::new();
    for signal in signals {
        if signal.confidence >= min_confidence {
            by_culprit
                .entry(signal.culprit_sha.clone())
                .or_default()
                .push(signal);
        }
    }

    // Aggregate each culprit
    let mut culprits: Vec<RankedCulprit> = by_culprit
        .into_iter()
        .map(|(sha, signals)| {
            let score: i64 = signals.iter().map(|s| s.weight).sum();
            let events = signals.len();

            let mut ttr_values: Vec<f64> = signals.iter().map(|s| s.time_to_regret_hours).collect();
            let ttr_p50 = median(&mut ttr_values);

            let min_conf = signals
                .iter()
                .map(|s| s.confidence)
                .fold(f64::MAX, f64::min);

            // Take info from first signal
            let first = signals[0];
            let age = compute_age(&first.culprit_time_utc);
            let subject = first
                .culprit_subject
                .clone()
                .unwrap_or_else(|| "(no subject)".to_string());

            RankedCulprit {
                id: 0, // Will be assigned after sorting
                score,
                events,
                ttr_p50_hours: ttr_p50,
                culprit_sha: sha,
                culprit_date_utc: first.culprit_time_utc.clone(),
                culprit_age: age,
                subject_trunc: subject,
                min_confidence: min_conf,
            }
        })
        .collect();

    // Sort by score desc, then SHA (lexical)
    culprits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.culprit_sha.cmp(&b.culprit_sha))
    });

    // Assign IDs
    for (i, culprit) in culprits.iter_mut().enumerate() {
        culprit.id = i + 1;
    }

    culprits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_subject() {
        assert_eq!(truncate_subject("short", 80), "short");
        assert_eq!(truncate_subject("a".repeat(80).as_str(), 80), "a".repeat(80));

        let long = "a".repeat(100);
        let truncated = truncate_subject(&long, 80);
        assert_eq!(truncated.len(), 80);
        assert!(truncated.ends_with("..."));
    }

    #[test]
    fn test_median_odd() {
        let mut values = vec![1.0, 3.0, 2.0];
        assert_eq!(median(&mut values), 2.0);
    }

    #[test]
    fn test_median_even() {
        let mut values = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(median(&mut values), 2.5);
    }

    #[test]
    fn test_median_empty() {
        let mut values: Vec<f64> = vec![];
        assert_eq!(median(&mut values), 0.0);
    }

    #[test]
    fn test_aggregate_culprits_sorting() {
        let signals = vec![
            SignalRow {
                culprit_sha: "aaaa".to_string(),
                evidence_sha: "bbbb".to_string(),
                weight: 10,
                confidence: 0.95,
                time_to_regret_hours: 24.0,
                culprit_time_utc: "2026-01-27T10:00:00Z".to_string(),
                culprit_subject: Some("commit A".to_string()),
            },
            SignalRow {
                culprit_sha: "cccc".to_string(),
                evidence_sha: "dddd".to_string(),
                weight: 20,
                confidence: 0.90,
                time_to_regret_hours: 48.0,
                culprit_time_utc: "2026-01-26T10:00:00Z".to_string(),
                culprit_subject: Some("commit C".to_string()),
            },
        ];

        let ranked = aggregate_culprits(&signals, 0.0);

        // cccc should be first (higher score)
        assert_eq!(ranked[0].culprit_sha, "cccc");
        assert_eq!(ranked[0].score, 20);
        assert_eq!(ranked[0].id, 1);

        assert_eq!(ranked[1].culprit_sha, "aaaa");
        assert_eq!(ranked[1].score, 10);
        assert_eq!(ranked[1].id, 2);
    }

    #[test]
    fn test_aggregate_filters_by_confidence() {
        let signals = vec![
            SignalRow {
                culprit_sha: "aaaa".to_string(),
                evidence_sha: "bbbb".to_string(),
                weight: 10,
                confidence: 0.85,
                time_to_regret_hours: 24.0,
                culprit_time_utc: "2026-01-27T10:00:00Z".to_string(),
                culprit_subject: Some("commit A".to_string()),
            },
        ];

        // With min_confidence=0.90, this signal should be filtered out
        let ranked = aggregate_culprits(&signals, 0.90);
        assert!(ranked.is_empty());

        // With min_confidence=0.80, it should be included
        let ranked = aggregate_culprits(&signals, 0.80);
        assert_eq!(ranked.len(), 1);
    }
}
