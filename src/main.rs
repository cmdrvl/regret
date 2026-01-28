use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use clap::{value_parser, Arg, ArgMatches, Command};
use regex::Regex;
use std::process;
use std::time::Duration as StdDuration;

mod cache_path;
mod config_file;
mod fast_path;
mod fileset;
mod hotspot;
mod output;
mod patch_id;
mod path_validation;
mod policy;
mod scan;
mod scan_lock;
mod selected_branch;
mod shallow;
mod signals;
mod store;
mod time_window;

/// Custom duration type for parsing time spans
#[derive(Debug, Clone, PartialEq)]
pub enum Duration {
    Hours(u64),
    Days(u64),
    Weeks(u64),
}

impl Duration {
    /// Convert to standard duration for internal use
    pub fn to_std_duration(&self) -> StdDuration {
        match self {
            Duration::Hours(h) => StdDuration::from_secs(h * 3600),
            Duration::Days(d) => StdDuration::from_secs(d * 24 * 3600),
            Duration::Weeks(w) => StdDuration::from_secs(w * 7 * 24 * 3600),
        }
    }

    /// Convert to chrono duration for date arithmetic.
    pub fn to_chrono_duration(&self) -> ChronoDuration {
        match self {
            Duration::Hours(h) => ChronoDuration::hours(*h as i64),
            Duration::Days(d) => ChronoDuration::days(*d as i64),
            Duration::Weeks(w) => ChronoDuration::weeks(*w as i64),
        }
    }
}

/// Custom date type for parsing dates
#[derive(Debug, Clone, PartialEq)]
pub enum DateSpec {
    Iso8601(String),
    Rfc3339(DateTime<Utc>),
}

/// Execution mode with precedence order
#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    /// --init: install templates/snippets (highest precedence)
    Init,
    /// --doctor: read-only diagnostics
    Doctor,
    /// --scan: scan-only, no rankings
    Scan,
    /// sha:<sha>: explain mode for specific culprit
    Explain(String),
    /// default: incremental scan + ranking (lowest precedence)
    Default,
}

/// Parse duration strings like "30d", "2w", "12h"
fn parse_duration(s: &str) -> Result<Duration> {
    let re = Regex::new(r"^(\d+)([hdw])$")?;

    let caps = re.captures(s).ok_or_else(|| {
        anyhow!(
            "Invalid duration format: '{}'. Expected format like '30d', '2w', '12h'",
            s
        )
    })?;

    let number: u64 = caps[1].parse().context("Invalid number in duration")?;

    match &caps[2] {
        "h" => Ok(Duration::Hours(number)),
        "d" => Ok(Duration::Days(number)),
        "w" => Ok(Duration::Weeks(number)),
        _ => Err(anyhow!("Invalid duration unit: expected 'h', 'd', or 'w'")),
    }
}

/// Parse date strings in YYYY-MM-DD or RFC3339 format
fn parse_date(s: &str) -> Result<DateSpec> {
    // Try RFC3339 first
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(DateSpec::Rfc3339(dt.with_timezone(&Utc)));
    }

    // Try YYYY-MM-DD format
    if NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() {
        return Ok(DateSpec::Iso8601(s.to_string()));
    }

    Err(anyhow!(
        "Invalid date format: '{}'. Expected YYYY-MM-DD or RFC3339 format",
        s
    ))
}

/// Parse SHA format (sha:<sha>)
fn parse_sha_arg(s: &str) -> Result<String> {
    if let Some(sha) = s.strip_prefix("sha:") {
        if sha.len() >= 4 && sha.len() <= 40 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(sha.to_string())
        } else {
            Err(anyhow!(
                "Invalid SHA format: '{}'. Expected 4-40 hex characters after 'sha:'",
                s
            ))
        }
    } else {
        Err(anyhow!(
            "Invalid SHA argument: '{}'. Expected format 'sha:<sha>'",
            s
        ))
    }
}

/// Resolve execution mode from config with precedence order
fn resolve_mode(config: &Config) -> Mode {
    // Order of precedence (highest to lowest):
    // 1. --init: install templates/snippets; ignore id and ranking flags
    if config.init {
        return Mode::Init;
    }

    // 2. --doctor: read-only diagnostics; ignore id and ranking flags
    if config.doctor {
        return Mode::Doctor;
    }

    // 3. --scan: scan-only; ignore id; do not print rankings
    if config.scan {
        return Mode::Scan;
    }

    // 4. sha:<sha> present: explain mode for culprit
    if let Some(sha) = &config.sha {
        return Mode::Explain(sha.clone());
    }

    // 5. default: incremental scan + ranking (lowest precedence)
    Mode::Default
}

/// Main CLI configuration
#[derive(Debug, Default)]
pub struct Config {
    // Core modes
    pub init: bool,
    pub force: bool,
    pub scan: bool,
    pub all: bool,
    pub doctor: bool,
    pub deep: bool,
    pub no_scan: bool,

    // Time filtering
    pub since: Option<Duration>,
    pub until: Option<DateSpec>,

    // Output filtering
    pub limit: Option<u32>,
    pub min_confidence: Option<f64>,

    // Output format
    pub table: bool,
    pub ndjson: bool,
    pub debug: bool,

    // Policy gates
    pub fail_if: Option<String>,

    // Positional
    pub sha: Option<String>,
}

#[derive(Debug, Clone)]
struct WindowInfo {
    since: Option<DateTime<Utc>>,
    until: DateTime<Utc>,
}

fn parse_duration_config(value: &str, field: &str) -> Result<Duration> {
    parse_duration(value).with_context(|| format!("Invalid config duration for {}", field))
}

fn compute_since(until: DateTime<Utc>, duration: &Duration) -> DateTime<Utc> {
    until - duration.to_chrono_duration()
}

fn compute_window_days(since: Option<DateTime<Utc>>, until: DateTime<Utc>) -> Option<u64> {
    since.map(|start| {
        let duration = until.signed_duration_since(start);
        duration.num_days().max(0) as u64
    })
}

fn compute_window_hint(since: Option<DateTime<Utc>>, until: DateTime<Utc>) -> Option<String> {
    let start = since?;
    let duration = until.signed_duration_since(start);
    let hours = duration.num_hours();
    if hours <= 0 {
        return None;
    }

    let hours_per_day = 24;
    let hours_per_week = hours_per_day * 7;
    if hours % hours_per_week == 0 {
        Some(format!("{}w", hours / hours_per_week))
    } else if hours % hours_per_day == 0 {
        Some(format!("{}d", hours / hours_per_day))
    } else {
        Some(format!("{}h", hours))
    }
}

fn format_error_message(err: &anyhow::Error) -> String {
    let message = err.to_string();
    if message.starts_with("error:") {
        message
    } else {
        format!("error: {}", message)
    }
}

fn build_cli() -> Command {
    Command::new("regret")
        .version(env!("CARGO_PKG_VERSION"))
        .author("CMD+RVL <engineering@cmdrvl.com>")
        .about("Single-verb, local-first, deterministic CLI that mines high-precision regret signals from git history")
        .arg(Arg::new("sha")
            .help("Explain mode for a specific commit (format: sha:<sha>)")
            .value_parser(parse_sha_arg)
            .index(1))
        .arg(Arg::new("init")
            .long("init")
            .help("Install commit templates and configuration")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("force")
            .long("force")
            .help("Overwrite existing files when running --init")
            .requires("init")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("scan")
            .long("scan")
            .help("Scan-only mode (no ranking output)")
            .conflicts_with("ndjson")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("all")
            .long("all")
            .help("Scan full history and rank over all evidence (<= --until)")
            .conflicts_with("no-scan")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("doctor")
            .long("doctor")
            .help("Run diagnostics and health checks")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("deep")
            .long("deep")
            .help("Enable expensive signal detection (patch-id equivalence)")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("no-scan")
            .long("no-scan")
            .help("Skip scanning; use existing cache only")
            .conflicts_with("scan")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("since")
            .long("since")
            .help("Rank evidence since this duration ago (e.g., 30d, 2w, 12h)")
            .value_name("DURATION")
            .value_parser(parse_duration))
        .arg(Arg::new("until")
            .long("until")
            .help("Set ranking window end (YYYY-MM-DD or RFC3339)")
            .value_name("DATE")
            .value_parser(parse_date))
        .arg(Arg::new("limit")
            .long("limit")
            .help("Maximum number of results to show")
            .value_name("N")
            .value_parser(value_parser!(u32)))
        .arg(Arg::new("min-confidence")
            .long("min-confidence")
            .help("Minimum confidence threshold (0.0-1.0)")
            .value_name("CONFIDENCE")
            .value_parser(value_parser!(f64)))
        .arg(Arg::new("table")
            .long("table")
            .help("Force table output format (default)")
            .conflicts_with("ndjson")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("ndjson")
            .long("ndjson")
            .help("Output as newline-delimited JSON (robot mode)")
            .conflicts_with("table")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("debug")
            .long("debug")
            .help("Enable debug output to stderr")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("fail-if")
            .long("fail-if")
            .help("Exit with code 3 if condition is met (policy gates)")
            .value_name("EXPR")
            .value_parser(value_parser!(String)))
}

fn config_from_matches(matches: &ArgMatches) -> Result<Config> {
    let config = Config {
        init: matches.get_flag("init"),
        force: matches.get_flag("force"),
        scan: matches.get_flag("scan"),
        all: matches.get_flag("all"),
        doctor: matches.get_flag("doctor"),
        deep: matches.get_flag("deep"),
        no_scan: matches.get_flag("no-scan"),
        since: matches.get_one::<Duration>("since").cloned(),
        until: matches.get_one::<DateSpec>("until").cloned(),
        limit: matches.get_one::<u32>("limit").copied(),
        min_confidence: matches.get_one::<f64>("min-confidence").copied(),
        table: matches.get_flag("table"),
        ndjson: matches.get_flag("ndjson"),
        debug: matches.get_flag("debug"),
        fail_if: matches.get_one::<String>("fail-if").cloned(),
        sha: matches.get_one::<String>("sha").cloned(),
    };

    if let Some(min_confidence) = config.min_confidence {
        if !(0.0..=1.0).contains(&min_confidence) {
            bail!("--min-confidence must be between 0.0 and 1.0");
        }
    }

    Ok(config)
}

/// Parse command line arguments into Config
fn parse_args() -> Result<Config> {
    let app = build_cli();

    let matches = match app.try_get_matches() {
        Ok(matches) => matches,
        Err(e) => e.exit(), // Exits with 0 for --help/--version, non-zero for errors
    };
    config_from_matches(&matches)
}

#[cfg(test)]
fn parse_args_from(args: &[&str]) -> Result<Config> {
    let app = build_cli();
    let matches = app.try_get_matches_from(args)?;
    config_from_matches(&matches)
}

/// Implement --doctor command: read-only cache diagnostics
fn run_doctor_command(config: &Config) -> Result<()> {
    use std::fs;
    use std::path::Path;

    let cache_dir = Path::new(".regret");
    let cache_db_path = cache_dir.join("cache.db");
    let cache_wal_path = cache_dir.join("cache.db-wal");

    println!("regret doctor");
    println!();

    // Check if cache directory exists
    if !cache_dir.exists() {
        println!("Cache: not found");
        println!("  No .regret/ directory found");
        return Ok(());
    }

    // Check if database file exists
    if !cache_db_path.exists() {
        println!("Cache: not found");
        println!("  .regret/ directory exists but no cache.db found");
        return Ok(());
    }

    // Report cache file sizes
    match fs::metadata(&cache_db_path) {
        Ok(metadata) => {
            let db_size = metadata.len();
            println!("Cache files:");
            println!("  cache.db: {} bytes", db_size);

            if cache_wal_path.exists() {
                match fs::metadata(&cache_wal_path) {
                    Ok(wal_metadata) => {
                        let wal_size = wal_metadata.len();
                        println!("  cache.db-wal: {} bytes", wal_size);
                    }
                    Err(_) => {
                        println!("  cache.db-wal: exists but unable to read size");
                    }
                }
            } else {
                println!("  cache.db-wal: not present");
            }
        }
        Err(_) => {
            println!("Cache files: unable to read size information");
        }
    }
    println!();

    // Try to open database and run diagnostics
    match store::Store::open(cache_dir) {
        Ok(store) => match store.run_diagnostics(config.deep) {
            Ok(results) => {
                print_diagnostic_results(&results, config);
            }
            Err(e) => {
                println!("Database diagnostics: error");
                println!("  {}", e);
            }
        },
        Err(e) => {
            println!("Database: error opening cache");
            println!("  {}", e);
        }
    }

    Ok(())
}

/// Print formatted database diagnostic results
fn print_diagnostic_results(results: &store::DiagnosticResults, config: &Config) {
    // Database integrity
    println!("Database integrity:");
    println!("  PRAGMA quick_check: {}", results.quick_check);

    // Schema version check
    println!();
    println!("Schema:");
    match &results.schema_version {
        Some(version) => {
            println!("  schema_version: {}", version);
            if version != "1" {
                println!("  warning: expected schema_version=1, found {}", version);
            }
        }
        None => {
            println!("  schema_version: not found or unreadable");
        }
    }

    // Tables
    println!();
    println!("Tables:");
    let table_order = vec!["meta", "file", "commit", "fileset", "signal"];
    for table in &table_order {
        if let Some(info) = results.tables.get(*table) {
            if info.exists {
                match info.row_count {
                    Some(count) => println!("  {}: exists ({} rows)", table, count),
                    None => println!("  {}: exists (unable to count rows)", table),
                }
            } else {
                println!("  {}: missing", table);
            }
        }
    }

    // Indexes
    println!();
    println!("Indexes:");
    let index_order = vec!["signal_time", "signal_culprit"];
    for index in &index_order {
        if let Some(&exists) = results.indexes.get(*index) {
            if exists {
                println!("  {}: exists", index);
            } else {
                println!("  {}: missing", index);
            }
        }
    }

    // Coverage and scanning status
    println!();
    println!("Coverage:");
    match &results.last_scanned_ref_oid {
        Some(oid) => {
            let short_oid = if oid.len() >= 12 { &oid[..12] } else { oid };
            println!("  last_scanned_ref_oid: {}", short_oid);
        }
        None => println!("  last_scanned_ref_oid: not set (no scans yet)"),
    }

    match &results.coverage_since_utc {
        Some(coverage) => println!("  coverage_since_utc: {}", coverage),
        None => println!("  coverage_since_utc: not set"),
    }
    match results.coverage_valid {
        Some(valid) => println!("  coverage_valid: {}", valid),
        None => println!("  coverage_valid: not set"),
    }
    match results.is_shallow {
        Some(true) => println!("  is_shallow: true (coverage bounded)"),
        Some(false) => println!("  is_shallow: false"),
        None => println!("  is_shallow: not set"),
    }

    match &results.coverage_since_oid {
        Some(coverage) => println!("  coverage_since_oid: {}", coverage),
        None => println!("  coverage_since_oid: not set"),
    }

    // Deep checks with --deep flag
    if config.deep {
        println!();
        println!("Deep checks (--deep):");
        match &results.integrity_check {
            Some(result) => println!("  PRAGMA integrity_check: {}", result),
            None => println!("  PRAGMA integrity_check: not run"),
        }
    }
}

/// Implement --init command: create templates and snippets
fn run_init_command(force: bool) -> Result<()> {
    use std::fs;
    use std::path::Path;

    let regret_dir = Path::new(".regret");

    // Ensure .regret directory exists with safe permissions
    cache_path::ensure_cache_dir(regret_dir)?;

    // Create subdirectories
    let agent_snippets_dir = regret_dir.join("agent-snippets");
    let ci_dir = regret_dir.join("ci");
    let hooks_dir = regret_dir.join("hooks");

    fs::create_dir_all(&agent_snippets_dir)?;
    fs::create_dir_all(&ci_dir)?;
    fs::create_dir_all(&hooks_dir)?;

    #[derive(Clone, Copy)]
    enum WriteOutcome {
        Created,
        Skipped,
        Overwritten,
    }

    fn write_file(path: &Path, content: &str, force: bool) -> Result<WriteOutcome> {
        if path.exists() {
            let metadata = fs::metadata(path)?;
            if !metadata.is_file() {
                bail!("error: expected file at {}, found non-file", path.display());
            }
            if force {
                fs::write(path, content)?;
                return Ok(WriteOutcome::Overwritten);
            }
            return Ok(WriteOutcome::Skipped);
        }

        fs::write(path, content)?;
        Ok(WriteOutcome::Created)
    }

    let rel_path = |path: &Path| {
        path.strip_prefix(regret_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    };

    let mut created_files = Vec::new();
    let mut skipped_files = Vec::new();
    let mut overwritten_files = Vec::new();

    let mut record = |outcome: WriteOutcome, path: &Path| {
        let rel = rel_path(path);
        match outcome {
            WriteOutcome::Created => created_files.push(rel),
            WriteOutcome::Skipped => skipped_files.push(rel),
            WriteOutcome::Overwritten => overwritten_files.push(rel),
        }
    };

    // 1. commit-template.txt
    let commit_template_content = r#"# regret commit template
#
# If this commit fixes a previously delivered change, add an explicit trailer
# referencing the *culprit* commit (full 40-hex SHA; no prefixes):
#
# Fixes-Commit: <full_sha>
# Fixes-SHA: <full_sha>
#
# Optional (context only; never affects scoring; enables CASS/beads join):
# Bead-Ref: <id>
# Work-Ref: <token>
# Session-Ref: <session_id>
"#;

    let commit_template_path = regret_dir.join("commit-template.txt");
    record(
        write_file(&commit_template_path, commit_template_content, force)?,
        &commit_template_path,
    );

    // 2. ADOPTION.md
    let adoption_content = r#"# regret adoption

Enable commit template (local repo):

    git config commit.template .regret/commit-template.txt

Disable commit template (local repo):

    git config --unset commit.template

Check current setting:

    git config --get commit.template
"#;

    let adoption_path = regret_dir.join("ADOPTION.md");
    record(
        write_file(&adoption_path, adoption_content, force)?,
        &adoption_path,
    );

    // 3. agent-snippets/regret-linked-fix.md
    let linked_fix_content = r#"# regret: linked-fix trailers (agent rule)

When you make a follow-up fix for a previous commit, add a trailer referencing the culprit commit:

- Add: `Fixes-Commit: <full 40-hex SHA>` in the commit message trailers/footer section.
- Use the full SHA (no prefixes).
- The SHA MUST be the culprit (the change being fixed), not the evidence/fix commit.
"#;

    let linked_fix_path = agent_snippets_dir.join("regret-linked-fix.md");
    record(
        write_file(&linked_fix_path, linked_fix_content, force)?,
        &linked_fix_path,
    );

    // 4. agent-snippets/regret-session-context.md
    let session_context_content = r#"# regret: session context trailers (agent rule)

Always include work tracking trailers in your commits to enable conversation-to-code traceability:

- Add: `Bead-Ref: <bead_id>` if you are working on a tracked issue (e.g., `Bead-Ref: beads-42`)
- Add: `Work-Ref: <token>` if you have a work token from your orchestrator
- Optionally add: `Session-Ref: <session_id>` if your session ID is available

These trailers enable `cass` and other tools to join commit history back to the conversation that produced it. They do not affect regret scoring—they are context only.
"#;

    let session_context_path = agent_snippets_dir.join("regret-session-context.md");
    record(
        write_file(&session_context_path, session_context_content, force)?,
        &session_context_path,
    );

    // 5. ci/github-actions-regret.yml
    let github_actions_content = r#"# Add this job to your .github/workflows/ci.yml or create a separate workflow

regret-check:
  name: Regret Analysis
  runs-on: ubuntu-latest
  steps:
    - uses: actions/checkout@v4
      with:
        fetch-depth: 0  # Full history for regret analysis

    - name: Install regret
      run: |
        curl -fsSL https://raw.githubusercontent.com/cmdrvl/regret/main/scripts/install.sh | bash
        echo "$HOME/.local/bin" >> $GITHUB_PATH

    - name: Run regret analysis
      run: |
        regret --ndjson --fail-if "regret_events >= 5 or max_score > 50"
"#;

    let github_actions_path = ci_dir.join("github-actions-regret.yml");
    record(
        write_file(&github_actions_path, github_actions_content, force)?,
        &github_actions_path,
    );

    // 6. hooks/commit-msg (POSIX sh, advisory only)
    let commit_msg_hook_content = r#"#!/bin/sh
# regret commit-msg hook (advisory only)
# Always exits 0 - never blocks commits

# Check if commit message contains "Fixes" but no "Fixes-Commit:" trailer
if grep -i "fixes" "$1" >/dev/null 2>&1; then
    if ! grep -E "^Fixes-(Commit|SHA):" "$1" >/dev/null 2>&1; then
        echo "hint: commit mentions 'fixes' but no Fixes-Commit: trailer found"
        echo "hint: consider adding 'Fixes-Commit: <full_sha>' to reference the culprit"
    fi
fi

# Always exit 0 (advisory only, never block)
exit 0
"#;

    let commit_msg_path = hooks_dir.join("commit-msg");
    let hook_outcome = write_file(&commit_msg_path, commit_msg_hook_content, force)?;
    record(hook_outcome, &commit_msg_path);

    // Make the hook executable on Unix (only if written)
    if matches!(
        hook_outcome,
        WriteOutcome::Created | WriteOutcome::Overwritten
    ) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&commit_msg_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&commit_msg_path, perms)?;
        }
    }

    // Summary output
    let total_created = created_files.len();
    let total_skipped = skipped_files.len();
    let total_overwritten = overwritten_files.len();

    if total_created > 0 && total_skipped == 0 {
        println!("Initialized .regret/");
    } else if total_created > 0 {
        println!(
            "Initialized .regret/ ({} new, {} existing)",
            total_created, total_skipped
        );
    } else if total_overwritten > 0 {
        println!("Reinitialized .regret/ ({} overwritten)", total_overwritten);
    } else {
        println!("Already initialized (.regret/ exists)");
    }

    println!("\nSee .regret/ADOPTION.md for setup options (commit template, hooks, CI).");

    Ok(())
}

/// Main entry point
fn main() {
    let config = match parse_args() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("{}", format_error_message(&e));
            process::exit(2);
        }
    };

    if let Err(e) = run(config) {
        eprintln!("{}", format_error_message(&e));
        process::exit(1);
    }
}

/// Main application logic
fn run(config: Config) -> Result<()> {
    let mode = resolve_mode(&config);

    if config.debug {
        eprintln!("Debug: Config = {:?}", config);
        eprintln!("Debug: Resolved Mode = {:?}", mode);
    }

    // Handle modes that don't need cache
    match &mode {
        Mode::Init => return run_init_command(config.force),
        Mode::Doctor => return run_doctor_command(&config),
        _ => {}
    }

    // All other modes need cache access
    let mut store = store::Store::open(std::path::Path::new(".regret"))?;
    let repo_root = std::env::current_dir()?;
    let selected = selected_branch::ensure_selected_branch(&repo_root, &store)?;
    fast_path::ensure_meta_defaults(&store, &selected)?;
    let until_info = time_window::compute_until(&config.until, &repo_root, &selected)?;
    let file_config = config_file::load_config(&repo_root)?;

    let default_ranking_since = parse_duration_config(
        file_config
            .ranking
            .default_since
            .as_deref()
            .unwrap_or("30d"),
        "ranking.default_since",
    )?;
    let default_scan_since = parse_duration_config(
        file_config.scan.bootstrap_since.as_deref().unwrap_or("45d"),
        "scan.bootstrap_since",
    )?;

    let explicit_since_utc = config
        .since
        .as_ref()
        .map(|duration| compute_since(until_info.until, duration));

    let ranking_since_utc = if config.all {
        None
    } else if let Some(since) = explicit_since_utc {
        Some(since)
    } else {
        Some(compute_since(until_info.until, &default_ranking_since))
    };

    let ranking_window = WindowInfo {
        since: ranking_since_utc,
        until: until_info.until,
    };

    if config.debug {
        eprintln!("Debug: selected_branch = {}", selected);
        eprintln!(
            "Debug: window_until_utc = {} ({:?})",
            until_info.until.to_rfc3339(),
            until_info.source
        );
    }

    // Track scan results
    let mut new_commits = 0usize;
    let mut scan_status = output::ScanStatus::Skipped;
    let mut did_scan = false;

    let scan_since_default = compute_since(until_info.until, &default_scan_since);

    // Run scan if needed
    match &mode {
        Mode::Scan => {
            if config.all {
                let summary = scan::full_scan(&repo_root, &mut store, &selected, config.deep)?;
                new_commits += summary.new_commits;
                did_scan = true;
            } else {
                let summary =
                    scan::incremental_scan(&repo_root, &mut store, &selected, config.deep)?;
                new_commits += summary.new_commits;
                did_scan = true;
                if config.debug {
                    eprintln!("Debug: scan_new_commits = {}", summary.new_commits);
                }

                if !summary.rewrite_detected {
                    let scan_since = explicit_since_utc.unwrap_or(scan_since_default);
                    let coverage_since_utc = store.get_meta_value(scan::META_COVERAGE_SINCE_UTC)?;
                    let coverage_since = coverage_since_utc
                        .as_deref()
                        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                        .map(|dt| dt.with_timezone(&Utc));

                    if coverage_since.map(|c| c > scan_since).unwrap_or(true) {
                        let summary = scan::backfill_scan(
                            &repo_root,
                            &mut store,
                            &selected,
                            scan_since,
                            coverage_since,
                            config.deep,
                        )?;
                        new_commits += summary.new_commits;
                    }
                }
            }
        }
        Mode::Default if !config.no_scan => {
            if config.all {
                let summary = scan::full_scan(&repo_root, &mut store, &selected, config.deep)?;
                new_commits += summary.new_commits;
                did_scan = true;
            } else {
                let skip_scan = fast_path::should_skip_scan(&repo_root, &store, &selected)?;
                if config.debug {
                    eprintln!("Debug: fast_path_skip_scan = {}", skip_scan);
                }
                let mut rewrite_detected = false;
                if !skip_scan {
                    let summary =
                        scan::incremental_scan(&repo_root, &mut store, &selected, config.deep)?;
                    new_commits += summary.new_commits;
                    rewrite_detected = summary.rewrite_detected;
                    did_scan = true;
                    if config.debug {
                        eprintln!("Debug: scan_new_commits = {}", summary.new_commits);
                    }
                }

                if !rewrite_detected {
                    if let Some(explicit_since) = explicit_since_utc {
                        let coverage_since_utc =
                            store.get_meta_value(scan::META_COVERAGE_SINCE_UTC)?;
                        let coverage_since = coverage_since_utc
                            .as_deref()
                            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                            .map(|dt| dt.with_timezone(&Utc));

                        if coverage_since.map(|c| c > explicit_since).unwrap_or(true) {
                            let summary = scan::backfill_scan(
                                &repo_root,
                                &mut store,
                                &selected,
                                explicit_since,
                                coverage_since,
                                config.deep,
                            )?;
                            new_commits += summary.new_commits;
                            did_scan = true;
                        }
                    }
                }
            }
        }
        _ => {}
    }

    if did_scan {
        scan_status = output::ScanStatus::Ran;
    }

    // Execute based on resolved mode
    match mode {
        Mode::Init | Mode::Doctor => unreachable!(), // Already handled above
        Mode::Scan => {
            // Scan-only mode: just print commit count
            println!("Scanned {} commits", new_commits);
        }
        Mode::Explain(sha) => {
            run_explain_output(
                &config,
                &store,
                &selected,
                &sha,
                &ranking_window,
                &until_info,
            )?;
        }
        Mode::Default => {
            // Default ranking mode: print output (NDJSON or human-readable)
            run_ranking_output(
                &config,
                &store,
                &selected,
                new_commits,
                scan_status,
                &ranking_window,
                &until_info,
            )?;
        }
    }

    Ok(())
}

/// Run the default ranking output (header + TOP table + RATE + COVERAGE/NO_EVENTS or NDJSON).
fn run_ranking_output(
    config: &Config,
    store: &store::Store,
    selected: &str,
    new_commits: usize,
    scan_status: output::ScanStatus,
    window: &WindowInfo,
    until_info: &time_window::UntilInfo,
) -> Result<()> {
    // Get repo info
    let repo_root = std::env::current_dir()?;
    let repo_basename = repo_root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Get coverage info
    let coverage_since_utc = store.get_meta_value(scan::META_COVERAGE_SINCE_UTC)?;
    let coverage_valid = store.get_meta_bool("coverage_valid")?.unwrap_or(false);
    let cache_valid = store.get_meta_bool("cache_valid")?.unwrap_or(false);

    let window_until_utc = window.until.to_rfc3339();
    let window_since_utc = window.since.map(|since| since.to_rfc3339());
    let window_days = compute_window_days(window.since, window.until);
    let window_hint = compute_window_hint(window.since, window.until);

    let coverage_since = coverage_since_utc
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|dt| dt.with_timezone(&Utc));

    // Compute coverage days (relative to frozen window end)
    let coverage_days = coverage_since.as_ref().map(|since| {
        let duration = window.until.signed_duration_since(*since);
        duration.num_days().max(0) as u64
    });

    // Get total scanned commits (all-time) and windowed commit count
    let scanned_commits = store.get_commit_count()?;
    let commits_in_window =
        store.get_commit_count_in_window(window_since_utc.as_deref(), &window_until_utc)?;

    // Get signals for window and aggregate
    let signals =
        store.get_signals_for_ranking(selected, window_since_utc.as_deref(), &window_until_utc)?;
    let min_confidence = config.min_confidence.unwrap_or(0.0);
    let signals: Vec<_> = signals
        .into_iter()
        .filter(|signal| signal.confidence >= min_confidence)
        .collect();
    let ranked = output::aggregate_culprits(&signals, min_confidence, window.until);

    // Count events and max score (after confidence/window filtering)
    let events: usize = ranked.iter().map(|c| c.events).sum();
    let max_score = ranked.first().map(|c| c.score).unwrap_or(0);

    // Compute events_per_100_commits for policy evaluation
    let events_per_100_commits = if commits_in_window > 0 {
        (events as f64 / commits_in_window as f64) * 100.0
    } else {
        0.0
    };

    // Parse --fail-if expression early to catch parse errors (exit 2)
    let policy_expr = if let Some(ref fail_if) = config.fail_if {
        match policy::parse_expression(fail_if) {
            Ok(expr) => Some(expr),
            Err(e) => {
                eprintln!("error: invalid --fail-if expression: {}", e);
                process::exit(2);
            }
        }
    } else {
        None
    };

    // NDJSON output mode
    if config.ndjson {
        let window_until_source = until_info.source.as_str().to_string();

        output::print_ndjson(&output::NdjsonInfo {
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            repo_path: repo_root,
            repo_basename,
            selected_branch: selected.to_string(),
            window_since_utc: window_since_utc.clone(),
            window_until_utc: window_until_utc.clone(),
            window_until_source,
            coverage_since_utc: coverage_since_utc.clone(),
            coverage_valid,
            cache_valid,
            scan_status,
            new_commits,
            debug: config.debug,
            events,
            max_score,
            commits_in_window,
            coverage_days,
            ranked_culprits: &ranked,
            signals: &signals,
        });

        // Evaluate policy after output (still exits 3 on violation)
        if let Some(expr) = policy_expr {
            let values = policy::MetricValues {
                regret_events: events,
                max_score,
                events_per_100_commits,
            };
            if expr.evaluate(&values) {
                process::exit(3);
            }
        }
        return Ok(());
    }

    // Human-readable output
    let header = output::HeaderInfo {
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        repo_basename,
        selected_branch: selected.to_string(),
        scan_status,
        new_commits,
        scanned_commits,
        coverage_days,
    };
    output::print_header(&header);

    // Print TOP table (if we have culprits)
    let limit = config.limit.unwrap_or(5) as usize;
    output::print_top_table(&ranked, limit);

    if let Some(culprit) = ranked.first() {
        let short_sha = if culprit.culprit_sha.len() >= 7 {
            &culprit.culprit_sha[..7]
        } else {
            &culprit.culprit_sha
        };
        println!("Explain: regret sha:{}", short_sha);
    }

    // Print RATE line (always)
    println!();
    output::print_rate(&output::RateInfo {
        events,
        commits: commits_in_window,
    });

    // Determine coverage status
    let coverage_complete = if window.since.is_none() {
        coverage_since.is_some()
    } else {
        match (coverage_since.as_ref(), window.since.as_ref()) {
            (Some(since), Some(window_since)) => *since <= *window_since,
            _ => false,
        }
    };
    let coverage_status = if !coverage_valid {
        output::CoverageStatus::Invalid
    } else if coverage_complete {
        output::CoverageStatus::Complete
    } else {
        output::CoverageStatus::Incomplete
    };

    // Print COVERAGE line (only when incomplete or invalid)
    if coverage_status != output::CoverageStatus::Complete {
        output::print_coverage(
            &output::CoverageInfo {
                status: coverage_status,
                coverage_since_utc: coverage_since_utc.clone().unwrap_or_default(),
                coverage_valid,
            },
            window_hint.as_deref(),
        );
    }

    // Print NO_EVENTS block (when zero events)
    if events == 0 {
        let (reverts_detected, linked_fix_detected) = store.get_signal_counts_by_type(selected)?;

        // Determine reason for no events
        let reason = if coverage_status == output::CoverageStatus::Incomplete
            || coverage_status == output::CoverageStatus::Invalid
        {
            output::NoEventsReason::CoverageIncomplete
        } else if reverts_detected == 0 && linked_fix_detected == 0 {
            output::NoEventsReason::NoSignalsDetected
        } else {
            // Signals exist but outside window or filtered by confidence
            output::NoEventsReason::SignalsOutsideWindowOrThreshold
        };

        println!();
        output::print_no_events(&output::NoEventsInfo {
            reverts_detected,
            linked_fix_trailers: linked_fix_detected,
            coverage_days,
            window_days,
            window_hint: window_hint.clone(),
            min_confidence,
            reason,
            regret_initialized: std::path::Path::new(".regret").exists(),
        });
    }

    // Evaluate policy after all output (exit 3 on violation)
    if let Some(expr) = policy_expr {
        let values = policy::MetricValues {
            regret_events: events,
            max_score,
            events_per_100_commits,
        };
        if expr.evaluate(&values) {
            process::exit(3);
        }
    }

    Ok(())
}

/// Run explain mode for a specific SHA.
fn run_explain_output(
    config: &Config,
    store: &store::Store,
    selected: &str,
    sha_prefix: &str,
    window: &WindowInfo,
    until_info: &time_window::UntilInfo,
) -> Result<()> {
    // Resolve SHA prefix to full SHA
    let full_sha = match store.resolve_sha_prefix(sha_prefix)? {
        Some(sha) => sha,
        None => {
            // Check if ambiguous or not found
            // Try to get any matches to determine if ambiguous
            eprintln!(
                "error: SHA '{}' not found or ambiguous in cache",
                sha_prefix
            );
            process::exit(1);
        }
    };

    // Get repo info
    let repo_root = std::env::current_dir()?;
    let repo_basename = repo_root
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Get coverage info
    let coverage_since_utc = store.get_meta_value(scan::META_COVERAGE_SINCE_UTC)?;
    let coverage_valid = store.get_meta_bool("coverage_valid")?.unwrap_or(false);
    let cache_valid = store.get_meta_bool("cache_valid")?.unwrap_or(false);

    let window_until_utc = window.until.to_rfc3339();
    let window_since_utc = window.since.map(|since| since.to_rfc3339());

    // Get signals for this culprit within the window
    let signals = store.get_signals_for_culprit(
        selected,
        &full_sha,
        window_since_utc.as_deref(),
        &window_until_utc,
    )?;
    let min_confidence = config.min_confidence.unwrap_or(0.0);
    let signals: Vec<_> = signals
        .into_iter()
        .filter(|signal| signal.confidence >= min_confidence)
        .collect();

    // Build explain info
    let explain_info = output::ExplainInfo {
        culprit_sha: full_sha.clone(),
        signals: &signals,
        tool_version: env!("CARGO_PKG_VERSION").to_string(),
        repo_path: repo_root,
        repo_basename,
        selected_branch: selected.to_string(),
        window_since_utc: window_since_utc.clone(),
        window_until_utc: window_until_utc.clone(),
        window_until_source: until_info.source.as_str().to_string(),
        coverage_since_utc,
        coverage_valid,
        cache_valid,
        debug: config.debug,
    };

    if config.ndjson {
        output::print_explain_ndjson(&explain_info);
    } else {
        output::print_explain_human(&explain_info);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("30d").unwrap(), Duration::Days(30));
        assert_eq!(parse_duration("2w").unwrap(), Duration::Weeks(2));
        assert_eq!(parse_duration("12h").unwrap(), Duration::Hours(12));

        assert!(parse_duration("30").is_err());
        assert!(parse_duration("30x").is_err());
        assert!(parse_duration("").is_err());
    }

    #[test]
    fn test_parse_date() {
        // Test YYYY-MM-DD format
        assert!(matches!(
            parse_date("2026-01-27").unwrap(),
            DateSpec::Iso8601(_)
        ));

        // Test RFC3339 format
        assert!(matches!(
            parse_date("2026-01-27T15:30:00Z").unwrap(),
            DateSpec::Rfc3339(_)
        ));

        // Test invalid formats
        assert!(parse_date("invalid").is_err());
        assert!(parse_date("2026/01/27").is_err());
    }

    #[test]
    fn test_parse_sha_arg() {
        assert_eq!(parse_sha_arg("sha:abc123").unwrap(), "abc123");
        assert_eq!(
            parse_sha_arg("sha:1234567890abcdef").unwrap(),
            "1234567890abcdef"
        );

        assert!(parse_sha_arg("abc123").is_err()); // Missing prefix
        assert!(parse_sha_arg("sha:").is_err()); // Empty SHA
        assert!(parse_sha_arg("sha:xyz").is_err()); // Non-hex characters
        assert!(parse_sha_arg("sha:123").is_err()); // Too short (less than 4)
    }

    #[test]
    fn test_compute_window_days() {
        let until = DateTime::parse_from_rfc3339("2026-01-10T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let since = DateTime::parse_from_rfc3339("2026-01-05T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(compute_window_days(Some(since), until), Some(5));
        assert_eq!(compute_window_days(None, until), None);
    }

    #[test]
    fn test_compute_window_hint() {
        let until = DateTime::parse_from_rfc3339("2026-01-10T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        let since = compute_since(until, &Duration::Hours(12));
        assert_eq!(
            compute_window_hint(Some(since), until),
            Some("12h".to_string())
        );

        let since = compute_since(until, &Duration::Hours(36));
        assert_eq!(
            compute_window_hint(Some(since), until),
            Some("36h".to_string())
        );

        let since = compute_since(until, &Duration::Days(30));
        assert_eq!(
            compute_window_hint(Some(since), until),
            Some("30d".to_string())
        );

        let since = compute_since(until, &Duration::Weeks(2));
        assert_eq!(
            compute_window_hint(Some(since), until),
            Some("2w".to_string())
        );

        assert_eq!(compute_window_hint(None, until), None);
    }

    #[test]
    fn test_compute_since_from_duration() {
        let until = DateTime::parse_from_rfc3339("2026-01-10T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let since = compute_since(until, &Duration::Days(5));
        assert_eq!(since.to_rfc3339(), "2026-01-05T00:00:00+00:00");
    }

    #[test]
    fn test_conflicting_flags_fail() {
        assert!(parse_args_from(&["regret", "--ndjson", "--table"]).is_err());
        assert!(parse_args_from(&["regret", "--scan", "--ndjson"]).is_err());
        assert!(parse_args_from(&["regret", "--all", "--no-scan"]).is_err());
    }

    #[test]
    fn test_min_confidence_validation() {
        assert!(parse_args_from(&["regret", "--min-confidence", "1.5"]).is_err());
        assert!(parse_args_from(&["regret", "--min-confidence", "0.5"]).is_ok());
    }

    #[test]
    fn test_force_requires_init() {
        assert!(parse_args_from(&["regret", "--force"]).is_err());
    }

    #[test]
    fn test_mode_precedence() {
        // Test precedence order: init > doctor > scan > explain > default

        // 1. Init has highest precedence (beats everything)
        let config = Config {
            init: true,
            doctor: true,
            scan: true,
            sha: Some("abc123".to_string()),
            ..Default::default()
        };
        assert_eq!(resolve_mode(&config), Mode::Init);

        // 2. Doctor beats scan and explain (but not init)
        let config = Config {
            doctor: true,
            scan: true,
            sha: Some("abc123".to_string()),
            ..Default::default()
        };
        assert_eq!(resolve_mode(&config), Mode::Doctor);

        // 3. Scan beats explain (but not init/doctor)
        let config = Config {
            scan: true,
            sha: Some("abc123".to_string()),
            ..Default::default()
        };
        assert_eq!(resolve_mode(&config), Mode::Scan);

        // 4. Explain beats default (but not init/doctor/scan)
        let config = Config {
            sha: Some("abc123".to_string()),
            ..Default::default()
        };
        assert_eq!(resolve_mode(&config), Mode::Explain("abc123".to_string()));

        // 5. Default when no other modes
        let config = Config::default();
        assert_eq!(resolve_mode(&config), Mode::Default);
    }
}
