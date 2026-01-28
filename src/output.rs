//! Output formatting for regret CLI.
//!
//! This module implements human-readable output per §10.1 and NDJSON output per §10.2:
//! - Header line (§10.1.1)
//! - TOP culprits table (§10.1.2)
//! - NDJSON meta and stat records (§10.2)

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

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
    if len.is_multiple_of(2) {
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

// =============================================================================
// NDJSON Output (§10.2)
// =============================================================================

/// NDJSON meta record per §10.2.
#[derive(Debug, Clone, Serialize)]
pub struct NdjsonMeta {
    #[serde(rename = "type")]
    pub record_type: &'static str,
    pub schema_version: u32,
    pub tool_version: String,
    pub repo_id: String,
    pub repo_basename: String,
    pub selected_branch: String,
    pub window_since_utc: Option<String>,
    pub window_until_utc: String,
    pub window_until_source: String,
    pub coverage_since_utc: Option<String>,
    pub coverage_valid: bool,
    pub cache_valid: bool,
}

/// NDJSON stat record per §10.2.
#[derive(Debug, Clone, Serialize)]
pub struct NdjsonStat {
    #[serde(rename = "type")]
    pub record_type: &'static str,
    pub name: String,
    pub value: serde_json::Value,
}

impl NdjsonStat {
    pub fn new<V: Into<serde_json::Value>>(name: &str, value: V) -> Self {
        Self {
            record_type: "stat",
            name: name.to_string(),
            value: value.into(),
        }
    }
}

/// NDJSON rank record per §10.2.
#[derive(Debug, Clone, Serialize)]
pub struct NdjsonRank {
    #[serde(rename = "type")]
    pub record_type: &'static str,
    pub culprit_sha: String,
    pub score: i64,
    pub events: usize,
    pub ttr_p50_h: f64,
    pub culprit_time_utc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub surface_coverage: Option<f64>,
}

/// NDJSON evidence record per §10.2.
#[derive(Debug, Clone, Serialize)]
pub struct NdjsonEvidence {
    #[serde(rename = "type")]
    pub record_type: &'static str,
    pub culprit_sha: String,
    pub evidence_sha: String,
    pub signal_type: String,
    pub confidence: f64,
    pub confidence_reason: String,
    pub time_to_regret_hours: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bead_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_ref: Option<String>,
}

/// Info for NDJSON output.
pub struct NdjsonInfo<'a> {
    pub tool_version: String,
    pub repo_path: std::path::PathBuf,
    pub repo_basename: String,
    pub selected_branch: String,
    pub window_since_utc: Option<String>,
    pub window_until_utc: String,
    pub window_until_source: String,
    pub coverage_since_utc: Option<String>,
    pub coverage_valid: bool,
    pub cache_valid: bool,
    pub events: usize,
    pub max_score: i64,
    pub commits_in_window: usize,
    pub coverage_days: Option<u64>,
    pub ranked_culprits: &'a [RankedCulprit],
    pub signals: &'a [SignalRow],
}

/// Compute a stable repo_id from the repo path (SHA256 hash, first 16 hex chars).
pub fn compute_repo_id(repo_path: &std::path::Path) -> String {
    let path_str = repo_path.to_string_lossy();
    let mut hasher = Sha256::new();
    hasher.update(path_str.as_bytes());
    let result = hasher.finalize();
    hex::encode(&result[..8]) // 16 hex chars
}

/// Print NDJSON output (meta + stat + rank + evidence records).
///
/// Output goes to stdout as newline-delimited JSON.
/// Order: meta (one), stat records, rank records, evidence records
pub fn print_ndjson(info: &NdjsonInfo) {
    let repo_id = compute_repo_id(&info.repo_path);

    // 1. Meta record (exactly one, first)
    let meta = NdjsonMeta {
        record_type: "meta",
        schema_version: 1,
        tool_version: info.tool_version.clone(),
        repo_id,
        repo_basename: info.repo_basename.clone(),
        selected_branch: info.selected_branch.clone(),
        window_since_utc: info.window_since_utc.clone(),
        window_until_utc: info.window_until_utc.clone(),
        window_until_source: info.window_until_source.clone(),
        coverage_since_utc: info.coverage_since_utc.clone(),
        coverage_valid: info.coverage_valid,
        cache_valid: info.cache_valid,
    };
    println!("{}", serde_json::to_string(&meta).expect("serialize meta"));

    // 2. Stat records (fixed order by name)
    let rate = if info.commits_in_window > 0 {
        (info.events as f64 / info.commits_in_window as f64) * 100.0
    } else {
        0.0
    };

    let stats = [
        NdjsonStat::new("regret_events", info.events as i64),
        NdjsonStat::new("max_score", info.max_score),
        NdjsonStat::new("events_per_100_commits", rate),
        NdjsonStat::new("commits_in_window", info.commits_in_window as i64),
        NdjsonStat::new(
            "coverage_days",
            info.coverage_days.map(|d| d as i64).unwrap_or(-1),
        ),
    ];

    for stat in stats {
        println!("{}", serde_json::to_string(&stat).expect("serialize stat"));
    }

    // 3. Rank records (sorted by score desc, then SHA)
    for culprit in info.ranked_culprits {
        let rank = NdjsonRank {
            record_type: "rank",
            culprit_sha: culprit.culprit_sha.clone(),
            score: culprit.score,
            events: culprit.events,
            ttr_p50_h: culprit.ttr_p50_hours,
            culprit_time_utc: culprit.culprit_date_utc.clone(),
            surface_coverage: None, // TODO: implement surfaces
        };
        println!("{}", serde_json::to_string(&rank).expect("serialize rank"));
    }

    // 4. Evidence records (grouped by culprit, then evidence_time, then SHA)
    // Build a map of signals grouped by culprit
    let mut by_culprit: std::collections::HashMap<&str, Vec<&SignalRow>> =
        std::collections::HashMap::new();
    for signal in info.signals {
        by_culprit
            .entry(&signal.culprit_sha)
            .or_default()
            .push(signal);
    }

    // Output evidence in culprit order (matching rank order)
    for culprit in info.ranked_culprits {
        if let Some(signals) = by_culprit.get(culprit.culprit_sha.as_str()) {
            // Sort by evidence time, then SHA
            let mut sorted_signals: Vec<_> = signals.iter().collect();
            sorted_signals.sort_by(|a, b| {
                a.culprit_time_utc
                    .cmp(&b.culprit_time_utc)
                    .then_with(|| a.evidence_sha.cmp(&b.evidence_sha))
            });

            for signal in sorted_signals {
                let evidence = NdjsonEvidence {
                    record_type: "evidence",
                    culprit_sha: signal.culprit_sha.clone(),
                    evidence_sha: signal.evidence_sha.clone(),
                    signal_type: signal_type_from_weight(signal.weight),
                    confidence: signal.confidence,
                    confidence_reason: confidence_reason_from_weight_and_confidence(
                        signal.weight,
                        signal.confidence,
                    ),
                    time_to_regret_hours: signal.time_to_regret_hours,
                    work_ref: None,    // TODO: parse from commit
                    bead_ref: None,    // TODO: parse from commit
                    session_ref: None, // TODO: parse from commit
                };
                println!(
                    "{}",
                    serde_json::to_string(&evidence).expect("serialize evidence")
                );
            }
        }
    }
}

/// Derive signal type from weight.
fn signal_type_from_weight(weight: i64) -> String {
    match weight {
        10 => "revert".to_string(),
        8 => "linked_fix".to_string(),
        _ => "unknown".to_string(),
    }
}

/// Derive confidence reason from weight and confidence.
fn confidence_reason_from_weight_and_confidence(weight: i64, confidence: f64) -> String {
    match (weight, confidence) {
        (10, c) if c >= 0.95 => "canonical_revert_line".to_string(),
        (10, _) => "patch_id_equivalence".to_string(),
        (8, _) => "explicit_trailer".to_string(),
        _ => "unknown".to_string(),
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
pub fn aggregate_culprits(signals: &[SignalRow], min_confidence: f64) -> Vec<RankedCulprit> {
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

// =============================================================================
// Explain Mode Output (§10.3)
// =============================================================================

/// Info for explain mode output.
pub struct ExplainInfo<'a> {
    pub culprit_sha: String,
    pub signals: &'a [SignalRow],
    /// For NDJSON mode
    pub tool_version: String,
    pub repo_path: std::path::PathBuf,
    pub repo_basename: String,
    pub selected_branch: String,
    pub window_until_utc: String,
    pub window_until_source: String,
    pub coverage_since_utc: Option<String>,
    pub coverage_valid: bool,
    pub cache_valid: bool,
}

/// Print NO_SIGNALS message for non-culprit SHA.
pub fn print_no_signals(sha: &str) {
    println!("NO_SIGNALS culprit=sha:{}", sha);
}

/// Print NO_SIGNALS in NDJSON format.
pub fn print_no_signals_ndjson(info: &ExplainInfo) {
    let repo_id = compute_repo_id(&info.repo_path);

    // Just meta record, no rank/evidence
    let meta = NdjsonMeta {
        record_type: "meta",
        schema_version: 1,
        tool_version: info.tool_version.clone(),
        repo_id,
        repo_basename: info.repo_basename.clone(),
        selected_branch: info.selected_branch.clone(),
        window_since_utc: None,
        window_until_utc: info.window_until_utc.clone(),
        window_until_source: info.window_until_source.clone(),
        coverage_since_utc: info.coverage_since_utc.clone(),
        coverage_valid: info.coverage_valid,
        cache_valid: info.cache_valid,
    };
    println!("{}", serde_json::to_string(&meta).expect("serialize meta"));
}

/// Print human-readable explain output for a culprit.
pub fn print_explain_human(info: &ExplainInfo) {
    if info.signals.is_empty() {
        print_no_signals(&info.culprit_sha);
        return;
    }

    // Aggregate for the culprit
    let ranked = aggregate_culprits(info.signals, 0.0);
    let culprit = &ranked[0];

    // Short SHA (7 chars)
    let short_sha = if info.culprit_sha.len() >= 7 {
        &info.culprit_sha[..7]
    } else {
        &info.culprit_sha
    };

    // CULPRIT line
    println!(
        "CULPRIT sha:{} score={} events={} ttr_p50_h={:.1} culprit_date_utc={}",
        short_sha, culprit.score, culprit.events, culprit.ttr_p50_hours, culprit.culprit_date_utc
    );

    // Subject (truncated to 120 chars)
    let subject = info.signals[0]
        .culprit_subject
        .clone()
        .unwrap_or_else(|| "(no subject)".to_string());
    let subject_trunc = if subject.len() > 120 {
        format!("{}...", &subject[..117])
    } else {
        subject
    };
    println!("  subject: {}", subject_trunc);

    // EVIDENCE section
    println!();
    println!("EVIDENCE (sorted by evidence_time desc):");
    println!(
        "  {:<12} {:<12} {:>20} {:>8} {:>6} confidence_reason",
        "type", "evidence_sha", "evidence_date_utc", "ttr_h", "conf"
    );

    // Sort signals by evidence time desc (we need to get evidence time from store)
    // For now, just print in the order we have (which is by evidence_time from the query)
    for signal in info.signals {
        let signal_type = signal_type_from_weight(signal.weight);
        let short_evidence = if signal.evidence_sha.len() >= 7 {
            &signal.evidence_sha[..7]
        } else {
            &signal.evidence_sha
        };
        let reason = confidence_reason_from_weight_and_confidence(signal.weight, signal.confidence);

        // We don't have evidence_date_utc in SignalRow, use culprit_time + ttr for now
        // TODO: Add evidence_time_utc to SignalRow
        println!(
            "  {:<12} {:<12} {:>20} {:>8.1} {:>6.2} {}",
            signal_type,
            short_evidence,
            "-",
            signal.time_to_regret_hours,
            signal.confidence,
            reason
        );
    }

    // SURFACES section - placeholder for now (after E5)
    // println!();
    // println!("SURFACES (top 5 by score):");
    // println!("  (not yet implemented)");
}

/// Print NDJSON explain output for a culprit.
pub fn print_explain_ndjson(info: &ExplainInfo) {
    if info.signals.is_empty() {
        print_no_signals_ndjson(info);
        return;
    }

    let repo_id = compute_repo_id(&info.repo_path);

    // 1. Meta record
    let meta = NdjsonMeta {
        record_type: "meta",
        schema_version: 1,
        tool_version: info.tool_version.clone(),
        repo_id,
        repo_basename: info.repo_basename.clone(),
        selected_branch: info.selected_branch.clone(),
        window_since_utc: None,
        window_until_utc: info.window_until_utc.clone(),
        window_until_source: info.window_until_source.clone(),
        coverage_since_utc: info.coverage_since_utc.clone(),
        coverage_valid: info.coverage_valid,
        cache_valid: info.cache_valid,
    };
    println!("{}", serde_json::to_string(&meta).expect("serialize meta"));

    // 2. One rank record (NO stat records per spec)
    let ranked = aggregate_culprits(info.signals, 0.0);
    if let Some(culprit) = ranked.first() {
        let rank = NdjsonRank {
            record_type: "rank",
            culprit_sha: culprit.culprit_sha.clone(),
            score: culprit.score,
            events: culprit.events,
            ttr_p50_h: culprit.ttr_p50_hours,
            culprit_time_utc: culprit.culprit_date_utc.clone(),
            surface_coverage: None,
        };
        println!("{}", serde_json::to_string(&rank).expect("serialize rank"));
    }

    // 3. All evidence records for this culprit
    for signal in info.signals {
        let evidence = NdjsonEvidence {
            record_type: "evidence",
            culprit_sha: signal.culprit_sha.clone(),
            evidence_sha: signal.evidence_sha.clone(),
            signal_type: signal_type_from_weight(signal.weight),
            confidence: signal.confidence,
            confidence_reason: confidence_reason_from_weight_and_confidence(
                signal.weight,
                signal.confidence,
            ),
            time_to_regret_hours: signal.time_to_regret_hours,
            work_ref: None,
            bead_ref: None,
            session_ref: None,
        };
        println!(
            "{}",
            serde_json::to_string(&evidence).expect("serialize evidence")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_subject() {
        assert_eq!(truncate_subject("short", 80), "short");
        assert_eq!(
            truncate_subject("a".repeat(80).as_str(), 80),
            "a".repeat(80)
        );

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
        let signals = vec![SignalRow {
            culprit_sha: "aaaa".to_string(),
            evidence_sha: "bbbb".to_string(),
            weight: 10,
            confidence: 0.85,
            time_to_regret_hours: 24.0,
            culprit_time_utc: "2026-01-27T10:00:00Z".to_string(),
            culprit_subject: Some("commit A".to_string()),
        }];

        // With min_confidence=0.90, this signal should be filtered out
        let ranked = aggregate_culprits(&signals, 0.90);
        assert!(ranked.is_empty());

        // With min_confidence=0.80, it should be included
        let ranked = aggregate_culprits(&signals, 0.80);
        assert_eq!(ranked.len(), 1);
    }
}
