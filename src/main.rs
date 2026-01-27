use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use clap::{value_parser, Arg, Command};
use regex::Regex;
use std::process;
use std::time::Duration as StdDuration;

mod cache_path;

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


/// Parse command line arguments into Config
fn parse_args() -> Result<Config> {
    let app = Command::new("regret")
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
        .arg(Arg::new("scan")
            .long("scan")
            .help("Scan-only mode (no ranking output)")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("all")
            .long("all")
            .help("Scan entire git history (not just since last scan)")
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
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("since")
            .long("since")
            .help("Scan commits since this duration ago (e.g., 30d, 2w, 12h)")
            .value_name("DURATION")
            .value_parser(parse_duration))
        .arg(Arg::new("until")
            .long("until")
            .help("Scan commits until this date (YYYY-MM-DD or RFC3339)")
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
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("ndjson")
            .long("ndjson")
            .help("Output as newline-delimited JSON (robot mode)")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("debug")
            .long("debug")
            .help("Enable debug output to stderr")
            .action(clap::ArgAction::SetTrue))
        .arg(Arg::new("fail-if")
            .long("fail-if")
            .help("Exit with code 3 if condition is met (policy gates)")
            .value_name("EXPR")
            .value_parser(value_parser!(String)));

    let matches = match app.try_get_matches() {
        Ok(matches) => matches,
        Err(e) => {
            eprintln!("{}", e);
            process::exit(2);
        }
    };

    let config = Config {
        init: matches.get_flag("init"),
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

    Ok(config)
}

/// Main entry point
fn main() {
    let config = match parse_args() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("Error: {}", e);
            process::exit(2);
        }
    };

    if let Err(e) = run(config) {
        eprintln!("Error: {}", e);
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

    let writes_cache = match mode {
        Mode::Init | Mode::Scan => true,
        Mode::Default => !config.no_scan,
        Mode::Doctor | Mode::Explain(_) => false,
    };

    if writes_cache {
        cache_path::ensure_cache_dir(std::path::Path::new(".regret"))?;
    }

    // Execute based on resolved mode (with precedence)
    match mode {
        Mode::Init => {
            println!("TODO: Implement init command (install commit templates)");
        }
        Mode::Doctor => {
            println!("TODO: Implement doctor command (diagnostics)");
        }
        Mode::Scan => {
            println!("TODO: Implement scan command (git history scanning)");
        }
        Mode::Explain(sha) => {
            println!("TODO: Implement explain mode for SHA: {}", sha);
        }
        Mode::Default => {
            println!("TODO: Implement default ranking mode");
        }
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
        config = Config::default();
        config.sha = Some("abc123".to_string());
        assert_eq!(resolve_mode(&config), Mode::Explain("abc123".to_string()));

        // 5. Default when no other modes
        config = Config::default();
        assert_eq!(resolve_mode(&config), Mode::Default);
    }
}
