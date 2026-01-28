//! Hotspot detection for files with repeated regret from multiple culprits.
//!
//! Per §9.2: A hotspot is a file path that appears in multiple signals
//! from distinct culprits, indicating systemic issues.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use crate::config_file::HotspotConfig;

/// A detected hotspot with aggregated statistics.
#[derive(Debug, Clone)]
pub struct Hotspot {
    /// File path that is the hotspot
    pub path: String,
    /// Total weighted score from all signals
    pub score: i64,
    /// Number of regret events involving this file
    pub events: usize,
    /// Number of distinct culprit commits
    pub culprits: usize,
}

/// Input signal for hotspot analysis.
#[derive(Debug, Clone)]
pub struct HotspotSignal {
    /// Culprit commit SHA
    pub culprit_sha: String,
    /// Signal weight
    pub weight: i64,
    /// Signal confidence
    pub confidence: f64,
    /// Files touched by the culprit commit
    pub file_paths: Vec<String>,
}

/// Detect the top hotspot from a collection of signals.
///
/// Returns None if no file meets the hotspot thresholds.
pub fn detect_hotspot(signals: &[HotspotSignal], config: &HotspotConfig) -> Option<Hotspot> {
    // Group by file path
    let mut file_stats: HashMap<String, FileStats> = HashMap::new();

    for signal in signals {
        // Skip signals below confidence threshold
        if signal.confidence < config.min_confidence {
            continue;
        }

        for path in &signal.file_paths {
            let stats = file_stats.entry(path.clone()).or_default();
            stats.events += 1;
            stats.score += signal.weight;
            stats.culprits.insert(signal.culprit_sha.clone());
        }
    }

    // Filter by thresholds and find max
    file_stats
        .into_iter()
        .filter(|(_, stats)| {
            stats.events >= config.min_events as usize
                && stats.culprits.len() >= config.min_culprits as usize
        })
        .map(|(path, stats)| Hotspot {
            path,
            score: stats.score,
            events: stats.events,
            culprits: stats.culprits.len(),
        })
        // Sort by score descending, then path ascending (lexical tie-break)
        .max_by(|a, b| {
            a.score
                .cmp(&b.score)
                .then_with(|| b.path.cmp(&a.path)) // Reversed for ascending
        })
}

/// Internal stats for a file path.
#[derive(Default)]
struct FileStats {
    events: usize,
    score: i64,
    culprits: HashSet<String>,
}

/// A surface (single file with highest score) as fallback when no hotspot.
#[derive(Debug, Clone)]
pub struct TopSurface {
    /// File path with the highest score
    pub path: String,
    /// Total weighted score
    pub score: i64,
    /// Number of regret events
    pub events: usize,
    /// Number of distinct culprits
    pub culprits: usize,
}

/// Find the top surface (highest score file) when no hotspot meets thresholds.
///
/// This is a fallback for when hotspot thresholds aren't met.
/// Returns None if there are no signals or no files.
pub fn find_top_surface(signals: &[HotspotSignal], min_confidence: f64) -> Option<TopSurface> {
    // Group by file path (same as hotspot but no threshold filtering)
    let mut file_stats: HashMap<String, FileStats> = HashMap::new();

    for signal in signals {
        // Skip signals below confidence threshold
        if signal.confidence < min_confidence {
            continue;
        }

        for path in &signal.file_paths {
            let stats = file_stats.entry(path.clone()).or_default();
            stats.events += 1;
            stats.score += signal.weight;
            stats.culprits.insert(signal.culprit_sha.clone());
        }
    }

    // Find max score (no threshold filtering)
    file_stats
        .into_iter()
        .map(|(path, stats)| TopSurface {
            path,
            score: stats.score,
            events: stats.events,
            culprits: stats.culprits.len(),
        })
        // Sort by score descending, then path ascending (lexical tie-break)
        .max_by(|a, b| {
            a.score
                .cmp(&b.score)
                .then_with(|| b.path.cmp(&a.path)) // Reversed for ascending
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> HotspotConfig {
        HotspotConfig {
            min_events: 2,
            min_culprits: 2,
            min_confidence: 0.0,
        }
    }

    #[test]
    fn test_two_events_two_culprits_is_hotspot() {
        let signals = vec![
            HotspotSignal {
                culprit_sha: "aaa".to_string(),
                weight: 10,
                confidence: 0.95,
                file_paths: vec!["src/main.rs".to_string()],
            },
            HotspotSignal {
                culprit_sha: "bbb".to_string(),
                weight: 8,
                confidence: 0.90,
                file_paths: vec!["src/main.rs".to_string()],
            },
        ];

        let hotspot = detect_hotspot(&signals, &default_config());
        assert!(hotspot.is_some());
        let h = hotspot.unwrap();
        assert_eq!(h.path, "src/main.rs");
        assert_eq!(h.events, 2);
        assert_eq!(h.culprits, 2);
        assert_eq!(h.score, 18);
    }

    #[test]
    fn test_two_events_one_culprit_not_hotspot() {
        let signals = vec![
            HotspotSignal {
                culprit_sha: "aaa".to_string(),
                weight: 10,
                confidence: 0.95,
                file_paths: vec!["src/main.rs".to_string()],
            },
            HotspotSignal {
                culprit_sha: "aaa".to_string(), // Same culprit
                weight: 8,
                confidence: 0.90,
                file_paths: vec!["src/main.rs".to_string()],
            },
        ];

        let hotspot = detect_hotspot(&signals, &default_config());
        assert!(hotspot.is_none());
    }

    #[test]
    fn test_max_score_wins() {
        let signals = vec![
            // File A: 2 events, 2 culprits, score=18
            HotspotSignal {
                culprit_sha: "aaa".to_string(),
                weight: 10,
                confidence: 0.95,
                file_paths: vec!["src/a.rs".to_string()],
            },
            HotspotSignal {
                culprit_sha: "bbb".to_string(),
                weight: 8,
                confidence: 0.90,
                file_paths: vec!["src/a.rs".to_string()],
            },
            // File B: 2 events, 2 culprits, score=20
            HotspotSignal {
                culprit_sha: "ccc".to_string(),
                weight: 10,
                confidence: 0.95,
                file_paths: vec!["src/b.rs".to_string()],
            },
            HotspotSignal {
                culprit_sha: "ddd".to_string(),
                weight: 10,
                confidence: 0.90,
                file_paths: vec!["src/b.rs".to_string()],
            },
        ];

        let hotspot = detect_hotspot(&signals, &default_config());
        assert!(hotspot.is_some());
        let h = hotspot.unwrap();
        assert_eq!(h.path, "src/b.rs"); // Higher score
        assert_eq!(h.score, 20);
    }

    #[test]
    fn test_score_tie_lexical_wins() {
        let signals = vec![
            // File Z: 2 events, 2 culprits, score=18
            HotspotSignal {
                culprit_sha: "aaa".to_string(),
                weight: 10,
                confidence: 0.95,
                file_paths: vec!["src/z.rs".to_string()],
            },
            HotspotSignal {
                culprit_sha: "bbb".to_string(),
                weight: 8,
                confidence: 0.90,
                file_paths: vec!["src/z.rs".to_string()],
            },
            // File A: 2 events, 2 culprits, score=18 (same)
            HotspotSignal {
                culprit_sha: "ccc".to_string(),
                weight: 10,
                confidence: 0.95,
                file_paths: vec!["src/a.rs".to_string()],
            },
            HotspotSignal {
                culprit_sha: "ddd".to_string(),
                weight: 8,
                confidence: 0.90,
                file_paths: vec!["src/a.rs".to_string()],
            },
        ];

        let hotspot = detect_hotspot(&signals, &default_config());
        assert!(hotspot.is_some());
        let h = hotspot.unwrap();
        assert_eq!(h.path, "src/a.rs"); // Lexically first
        assert_eq!(h.score, 18);
    }

    #[test]
    fn test_confidence_filter() {
        let config = HotspotConfig {
            min_events: 2,
            min_culprits: 2,
            min_confidence: 0.9, // Higher threshold
        };

        let signals = vec![
            HotspotSignal {
                culprit_sha: "aaa".to_string(),
                weight: 10,
                confidence: 0.95,
                file_paths: vec!["src/main.rs".to_string()],
            },
            HotspotSignal {
                culprit_sha: "bbb".to_string(),
                weight: 8,
                confidence: 0.85, // Below threshold
                file_paths: vec!["src/main.rs".to_string()],
            },
        ];

        let hotspot = detect_hotspot(&signals, &config);
        assert!(hotspot.is_none()); // Only 1 event passes confidence
    }

    #[test]
    fn test_multiple_files_per_signal() {
        let signals = vec![
            HotspotSignal {
                culprit_sha: "aaa".to_string(),
                weight: 10,
                confidence: 0.95,
                file_paths: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
            },
            HotspotSignal {
                culprit_sha: "bbb".to_string(),
                weight: 8,
                confidence: 0.90,
                file_paths: vec!["src/main.rs".to_string()], // Only main.rs
            },
        ];

        let hotspot = detect_hotspot(&signals, &default_config());
        assert!(hotspot.is_some());
        let h = hotspot.unwrap();
        assert_eq!(h.path, "src/main.rs"); // 2 events, 2 culprits
        assert_eq!(h.events, 2);
        assert_eq!(h.culprits, 2);
        // lib.rs only has 1 event, 1 culprit - not a hotspot
    }

    #[test]
    fn test_empty_signals() {
        let signals: Vec<HotspotSignal> = vec![];
        let hotspot = detect_hotspot(&signals, &default_config());
        assert!(hotspot.is_none());
    }

    #[test]
    fn test_single_signal_not_hotspot() {
        let signals = vec![HotspotSignal {
            culprit_sha: "aaa".to_string(),
            weight: 10,
            confidence: 0.95,
            file_paths: vec!["src/main.rs".to_string()],
        }];

        let hotspot = detect_hotspot(&signals, &default_config());
        assert!(hotspot.is_none()); // Needs min 2 events
    }

    // Top surface fallback tests

    #[test]
    fn test_top_surface_when_no_hotspot() {
        // Single event, single culprit - not a hotspot but should be top surface
        let signals = vec![HotspotSignal {
            culprit_sha: "aaa".to_string(),
            weight: 10,
            confidence: 0.95,
            file_paths: vec!["src/main.rs".to_string()],
        }];

        let hotspot = detect_hotspot(&signals, &default_config());
        assert!(hotspot.is_none());

        let surface = find_top_surface(&signals, 0.0);
        assert!(surface.is_some());
        let s = surface.unwrap();
        assert_eq!(s.path, "src/main.rs");
        assert_eq!(s.score, 10);
        assert_eq!(s.events, 1);
        assert_eq!(s.culprits, 1);
    }

    #[test]
    fn test_top_surface_max_score() {
        let signals = vec![
            HotspotSignal {
                culprit_sha: "aaa".to_string(),
                weight: 10,
                confidence: 0.95,
                file_paths: vec!["src/a.rs".to_string()],
            },
            HotspotSignal {
                culprit_sha: "bbb".to_string(),
                weight: 20,
                confidence: 0.95,
                file_paths: vec!["src/b.rs".to_string()],
            },
        ];

        let surface = find_top_surface(&signals, 0.0);
        assert!(surface.is_some());
        let s = surface.unwrap();
        assert_eq!(s.path, "src/b.rs"); // Higher score
        assert_eq!(s.score, 20);
    }

    #[test]
    fn test_top_surface_tie_lexical() {
        let signals = vec![
            HotspotSignal {
                culprit_sha: "aaa".to_string(),
                weight: 10,
                confidence: 0.95,
                file_paths: vec!["src/z.rs".to_string()],
            },
            HotspotSignal {
                culprit_sha: "bbb".to_string(),
                weight: 10,
                confidence: 0.95,
                file_paths: vec!["src/a.rs".to_string()],
            },
        ];

        let surface = find_top_surface(&signals, 0.0);
        assert!(surface.is_some());
        let s = surface.unwrap();
        assert_eq!(s.path, "src/a.rs"); // Lexically first
    }

    #[test]
    fn test_top_surface_empty_signals() {
        let signals: Vec<HotspotSignal> = vec![];
        let surface = find_top_surface(&signals, 0.0);
        assert!(surface.is_none());
    }

    #[test]
    fn test_top_surface_confidence_filter() {
        let signals = vec![HotspotSignal {
            culprit_sha: "aaa".to_string(),
            weight: 10,
            confidence: 0.85,
            file_paths: vec!["src/main.rs".to_string()],
        }];

        // With high confidence threshold, should return None
        let surface = find_top_surface(&signals, 0.9);
        assert!(surface.is_none());

        // With lower threshold, should return surface
        let surface = find_top_surface(&signals, 0.8);
        assert!(surface.is_some());
    }
}
