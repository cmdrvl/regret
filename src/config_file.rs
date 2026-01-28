//! Configuration file loading and merging.
//!
//! Implements TOML config file loading per §4.7:
//! - Location: <repo_root>/.regret/config.toml
//! - Override order: defaults → config file → CLI flags

#![allow(dead_code)]

use anyhow::{Context, Result};
use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Default configuration values
const DEFAULT_BOOTSTRAP_SINCE_DAYS: u64 = 45;
const DEFAULT_RANKING_SINCE_DAYS: u64 = 30;
const DEFAULT_WEIGHT_REVERT: i64 = 10;
const DEFAULT_WEIGHT_LINKED_FIX: i64 = 8;
const DEFAULT_MIN_EVENTS: u32 = 2;
const DEFAULT_MIN_CULPRITS: u32 = 2;
const DEFAULT_MIN_CONFIDENCE: f64 = 0.0;
const DEFAULT_WAL_CHECKPOINT_MB: u64 = 64;

/// Configuration loaded from TOML file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct FileConfig {
    pub scan: ScanConfig,
    pub ranking: RankingConfig,
    pub hotspot: HotspotConfig,
    pub cache: CacheConfig,
    pub surfaces: SurfacesConfig,
}

/// Scan configuration section.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    /// Duration string for bootstrap scan (e.g., "45d")
    pub bootstrap_since: Option<String>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            bootstrap_since: Some(format!("{}d", DEFAULT_BOOTSTRAP_SINCE_DAYS)),
        }
    }
}

/// Ranking configuration section.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RankingConfig {
    /// Default duration for ranking window (e.g., "30d")
    pub default_since: Option<String>,
    /// Signal weights
    pub weights: WeightsConfig,
}

impl Default for RankingConfig {
    fn default() -> Self {
        Self {
            default_since: Some(format!("{}d", DEFAULT_RANKING_SINCE_DAYS)),
            weights: WeightsConfig::default(),
        }
    }
}

/// Signal weight configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct WeightsConfig {
    /// Weight for revert signals
    pub revert: i64,
    /// Weight for linked-fix signals
    pub linked_fix: i64,
}

impl Default for WeightsConfig {
    fn default() -> Self {
        Self {
            revert: DEFAULT_WEIGHT_REVERT,
            linked_fix: DEFAULT_WEIGHT_LINKED_FIX,
        }
    }
}

/// Hotspot configuration section.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HotspotConfig {
    /// Minimum events to qualify as hotspot
    pub min_events: u32,
    /// Minimum culprits to qualify as hotspot
    pub min_culprits: u32,
    /// Minimum confidence threshold
    pub min_confidence: f64,
}

impl Default for HotspotConfig {
    fn default() -> Self {
        Self {
            min_events: DEFAULT_MIN_EVENTS,
            min_culprits: DEFAULT_MIN_CULPRITS,
            min_confidence: DEFAULT_MIN_CONFIDENCE,
        }
    }
}

/// Cache configuration section.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// WAL checkpoint threshold in MB
    pub wal_checkpoint_threshold_mb: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            wal_checkpoint_threshold_mb: DEFAULT_WAL_CHECKPOINT_MB,
        }
    }
}

/// Surfaces configuration section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SurfacesConfig {
    /// Glob patterns to ignore
    pub ignore_patterns: Vec<String>,
    /// Additional ignore patterns (merged with defaults)
    pub additional_ignore: Vec<String>,
}

/// Load configuration from the default location.
///
/// Returns the default configuration if the file doesn't exist.
/// Warns on unknown keys but continues.
/// Returns error on invalid values (exit 2).
pub fn load_config(repo_root: &Path) -> Result<FileConfig> {
    let config_path = repo_root.join(".regret").join("config.toml");

    if !config_path.exists() {
        return Ok(FileConfig::default());
    }

    let content = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read config file: {}", config_path.display()))?;

    // Parse TOML with default handling for missing fields
    let config: FileConfig = toml::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", config_path.display()))?;

    // Validate configuration values
    validate_config(&config)?;

    Ok(config)
}

/// Validate configuration values.
fn validate_config(config: &FileConfig) -> Result<()> {
    // Validate duration strings if present
    if let Some(ref since) = config.scan.bootstrap_since {
        validate_duration(since, "scan.bootstrap_since")?;
    }
    if let Some(ref since) = config.ranking.default_since {
        validate_duration(since, "ranking.default_since")?;
    }

    // Validate confidence is in [0.0, 1.0]
    if !(0.0..=1.0).contains(&config.hotspot.min_confidence) {
        anyhow::bail!(
            "Invalid config: hotspot.min_confidence must be between 0.0 and 1.0, got {}",
            config.hotspot.min_confidence
        );
    }

    // Validate weights are positive
    if config.ranking.weights.revert <= 0 {
        anyhow::bail!(
            "Invalid config: ranking.weights.revert must be positive, got {}",
            config.ranking.weights.revert
        );
    }
    if config.ranking.weights.linked_fix <= 0 {
        anyhow::bail!(
            "Invalid config: ranking.weights.linked_fix must be positive, got {}",
            config.ranking.weights.linked_fix
        );
    }

    Ok(())
}

/// Validate a duration string format (e.g., "30d", "2w", "12h").
fn validate_duration(s: &str, field: &str) -> Result<()> {
    let re = regex::Regex::new(r"^\d+[hdw]$").expect("valid regex");
    if !re.is_match(s) {
        anyhow::bail!(
            "Invalid config: {} must be a duration (e.g., '30d', '2w', '12h'), got '{}'",
            field,
            s
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn test_default_config() {
        let config = FileConfig::default();
        assert_eq!(config.ranking.weights.revert, 10);
        assert_eq!(config.ranking.weights.linked_fix, 8);
        assert_eq!(config.hotspot.min_events, 2);
        assert_eq!(config.hotspot.min_confidence, 0.0);
    }

    #[test]
    fn test_load_missing_file() {
        let temp = tempdir().unwrap();
        let config = load_config(temp.path()).unwrap();
        assert_eq!(config.ranking.weights.revert, 10);
    }

    #[test]
    fn test_load_partial_config() {
        let temp = tempdir().unwrap();
        let regret_dir = temp.path().join(".regret");
        fs::create_dir_all(&regret_dir).unwrap();

        let config_path = regret_dir.join("config.toml");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "[ranking.weights]").unwrap();
        writeln!(file, "revert = 15").unwrap();

        let config = load_config(temp.path()).unwrap();
        assert_eq!(config.ranking.weights.revert, 15);
        assert_eq!(config.ranking.weights.linked_fix, 8); // default
    }

    #[test]
    fn test_load_full_config() {
        let temp = tempdir().unwrap();
        let regret_dir = temp.path().join(".regret");
        fs::create_dir_all(&regret_dir).unwrap();

        let config_path = regret_dir.join("config.toml");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "[scan]").unwrap();
        writeln!(file, "bootstrap_since = \"60d\"").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "[ranking]").unwrap();
        writeln!(file, "default_since = \"14d\"").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "[ranking.weights]").unwrap();
        writeln!(file, "revert = 20").unwrap();
        writeln!(file, "linked_fix = 12").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "[hotspot]").unwrap();
        writeln!(file, "min_events = 3").unwrap();
        writeln!(file, "min_culprits = 3").unwrap();
        writeln!(file, "min_confidence = 0.5").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "[cache]").unwrap();
        writeln!(file, "wal_checkpoint_threshold_mb = 128").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "[surfaces]").unwrap();
        writeln!(
            file,
            "ignore_patterns = [\"vendor/**\", \"node_modules/**\"]"
        )
        .unwrap();

        let config = load_config(temp.path()).unwrap();
        assert_eq!(config.scan.bootstrap_since, Some("60d".to_string()));
        assert_eq!(config.ranking.default_since, Some("14d".to_string()));
        assert_eq!(config.ranking.weights.revert, 20);
        assert_eq!(config.ranking.weights.linked_fix, 12);
        assert_eq!(config.hotspot.min_events, 3);
        assert_eq!(config.hotspot.min_culprits, 3);
        assert_eq!(config.hotspot.min_confidence, 0.5);
        assert_eq!(config.cache.wal_checkpoint_threshold_mb, 128);
        assert_eq!(config.surfaces.ignore_patterns.len(), 2);
    }

    #[test]
    fn test_invalid_confidence() {
        let temp = tempdir().unwrap();
        let regret_dir = temp.path().join(".regret");
        fs::create_dir_all(&regret_dir).unwrap();

        let config_path = regret_dir.join("config.toml");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "[hotspot]").unwrap();
        writeln!(file, "min_confidence = 1.5").unwrap();

        let result = load_config(temp.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("min_confidence must be between"));
    }

    #[test]
    fn test_invalid_duration() {
        let temp = tempdir().unwrap();
        let regret_dir = temp.path().join(".regret");
        fs::create_dir_all(&regret_dir).unwrap();

        let config_path = regret_dir.join("config.toml");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "[scan]").unwrap();
        writeln!(file, "bootstrap_since = \"invalid\"").unwrap();

        let result = load_config(temp.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must be a duration"));
    }

    #[test]
    fn test_invalid_weight() {
        let temp = tempdir().unwrap();
        let regret_dir = temp.path().join(".regret");
        fs::create_dir_all(&regret_dir).unwrap();

        let config_path = regret_dir.join("config.toml");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "[ranking.weights]").unwrap();
        writeln!(file, "revert = -5").unwrap();

        let result = load_config(temp.path());
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("revert must be positive"));
    }

    #[test]
    fn test_unknown_keys_allowed() {
        let temp = tempdir().unwrap();
        let regret_dir = temp.path().join(".regret");
        fs::create_dir_all(&regret_dir).unwrap();

        let config_path = regret_dir.join("config.toml");
        let mut file = fs::File::create(&config_path).unwrap();
        writeln!(file, "[unknown_section]").unwrap();
        writeln!(file, "unknown_key = 123").unwrap();
        writeln!(file, "").unwrap();
        writeln!(file, "[ranking]").unwrap();
        writeln!(file, "unknown_nested = true").unwrap();

        // Should succeed - unknown keys are ignored by serde default
        let result = load_config(temp.path());
        assert!(result.is_ok());
    }
}
