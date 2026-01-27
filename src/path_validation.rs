use lru::LruCache;
use std::num::NonZeroUsize;

/// Maximum allowed path length before truncation
#[allow(dead_code)]
const MAX_PATH_LENGTH: usize = 4096;

/// Marker appended to truncated paths
#[allow(dead_code)]
const TRUNCATED_MARKER: &[u8] = b"<TRUNCATED>";

/// Maximum number of cached paths to retain
#[allow(dead_code)]
const PATH_CACHE_CAPACITY: usize = 8192;

/// File ID type for caching validated paths
#[allow(dead_code)]
pub type FileId = u64;

/// LRU cache for validated path bytes to file ID mapping
#[allow(dead_code)]
pub struct PathCache {
    cache: LruCache<Vec<u8>, FileId>,
    next_id: FileId,
}

impl PathCache {
    #[allow(dead_code)]
    pub fn new() -> Self {
        let capacity =
            NonZeroUsize::new(PATH_CACHE_CAPACITY).expect("PATH_CACHE_CAPACITY must be non-zero");
        Self {
            cache: LruCache::new(capacity),
            next_id: 1,
        }
    }

    /// Get or insert a file ID for the given validated path
    #[allow(dead_code)]
    pub fn get_or_insert(&mut self, path_bytes: Vec<u8>) -> FileId {
        if let Some(&id) = self.cache.get(&path_bytes) {
            id
        } else {
            let id = self.next_id;
            self.next_id += 1;
            self.cache.put(path_bytes, id);
            id
        }
    }
}

/// Validation error types
#[allow(dead_code)]
#[derive(Debug, PartialEq)]
pub enum PathValidationError {
    ContainsParentSegment,
    IsAbsolute,
    ContainsNulByte,
}

impl std::fmt::Display for PathValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PathValidationError::ContainsParentSegment => write!(f, "path contains .. segment"),
            PathValidationError::IsAbsolute => write!(f, "path is absolute"),
            PathValidationError::ContainsNulByte => write!(f, "path contains NUL byte"),
        }
    }
}

impl std::error::Error for PathValidationError {}

/// Fast, safe file path validation for git paths
///
/// Rules:
/// - Reject .. segments
/// - Reject absolute paths
/// - Reject NUL bytes
/// - Normalize separators to /
/// - Truncate >4096 bytes with marker
///
/// Returns normalized path bytes on success
#[allow(dead_code)]
pub fn validate_path(path_bytes: &[u8]) -> Result<Vec<u8>, PathValidationError> {
    // Early exit for empty paths
    if path_bytes.is_empty() {
        return Ok(Vec::new());
    }

    // Check for absolute paths (starts with / or \ or drive letter on Windows)
    if is_absolute_path(path_bytes) {
        return Err(PathValidationError::IsAbsolute);
    }

    // Single pass validation and normalization
    let mut normalized = Vec::with_capacity(path_bytes.len().min(MAX_PATH_LENGTH));
    let mut segment_start = 0;

    for (i, &byte) in path_bytes.iter().enumerate() {
        // Check for NUL bytes
        if byte == 0 {
            return Err(PathValidationError::ContainsNulByte);
        }

        // Handle path separators (/ or \)
        if byte == b'/' || byte == b'\\' {
            // Check if current segment is ".."
            if is_parent_segment(&path_bytes[segment_start..i]) {
                return Err(PathValidationError::ContainsParentSegment);
            }

            // Add normalized segment (if not empty)
            if i > segment_start {
                if !normalized.is_empty() {
                    normalized.push(b'/');
                }
                normalized.extend_from_slice(&path_bytes[segment_start..i]);
            }

            segment_start = i + 1;

            // Check for truncation
            if normalized.len() >= MAX_PATH_LENGTH {
                break;
            }
        } else {
            // Regular character - will be added with the segment
        }
    }

    // Handle final segment (no trailing separator)
    if segment_start < path_bytes.len() {
        let final_segment = &path_bytes[segment_start..];
        if is_parent_segment(final_segment) {
            return Err(PathValidationError::ContainsParentSegment);
        }

        // Add final segment
        if !final_segment.is_empty() {
            if !normalized.is_empty() {
                normalized.push(b'/');
            }

            let remaining_space = MAX_PATH_LENGTH.saturating_sub(normalized.len());
            if final_segment.len() <= remaining_space {
                normalized.extend_from_slice(final_segment);
            } else {
                // Truncate and add marker
                let truncate_at = remaining_space.saturating_sub(TRUNCATED_MARKER.len());
                normalized.extend_from_slice(&final_segment[..truncate_at]);
                normalized.extend_from_slice(TRUNCATED_MARKER);
            }
        }
    }

    // Handle case where we hit the limit during segment processing
    if path_bytes.len() > MAX_PATH_LENGTH && !normalized.ends_with(TRUNCATED_MARKER) {
        // Remove bytes to make room for marker
        let truncate_to = MAX_PATH_LENGTH.saturating_sub(TRUNCATED_MARKER.len());
        if normalized.len() > truncate_to {
            normalized.truncate(truncate_to);
        }
        normalized.extend_from_slice(TRUNCATED_MARKER);
    }

    Ok(normalized)
}

/// Check if path bytes represent an absolute path
#[allow(dead_code)]
fn is_absolute_path(path_bytes: &[u8]) -> bool {
    if path_bytes.is_empty() {
        return false;
    }

    // Unix-style absolute path
    if path_bytes[0] == b'/' {
        return true;
    }

    // Windows-style absolute paths
    if path_bytes[0] == b'\\' {
        return true;
    }

    // Windows drive letter (e.g., C:\ or C:/)
    if path_bytes.len() >= 2 && path_bytes[0].is_ascii_alphabetic() && path_bytes[1] == b':' {
        return true;
    }

    false
}

/// Check if segment bytes represent a parent directory reference (..)
#[allow(dead_code)]
fn is_parent_segment(segment_bytes: &[u8]) -> bool {
    segment_bytes == b".."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_paths() {
        assert_eq!(validate_path(b"src/main.rs").unwrap(), b"src/main.rs");
        assert_eq!(validate_path(b"README.md").unwrap(), b"README.md");
        assert_eq!(validate_path(b"").unwrap(), b"");
        assert_eq!(validate_path(b"a").unwrap(), b"a");
    }

    #[test]
    fn test_windows_separator_normalization() {
        assert_eq!(
            validate_path(b"windows\\path.rs").unwrap(),
            b"windows/path.rs"
        );
        assert_eq!(
            validate_path(b"mixed/path\\test.rs").unwrap(),
            b"mixed/path/test.rs"
        );
    }

    #[test]
    fn test_parent_segment_rejection() {
        assert_eq!(
            validate_path(b"../escape.rs"),
            Err(PathValidationError::ContainsParentSegment)
        );
        assert_eq!(
            validate_path(b"src/../escape.rs"),
            Err(PathValidationError::ContainsParentSegment)
        );
        assert_eq!(
            validate_path(b"src/.."),
            Err(PathValidationError::ContainsParentSegment)
        );
    }

    #[test]
    fn test_absolute_path_rejection() {
        assert_eq!(
            validate_path(b"/absolute/path.rs"),
            Err(PathValidationError::IsAbsolute)
        );
        assert_eq!(
            validate_path(b"\\windows\\absolute.rs"),
            Err(PathValidationError::IsAbsolute)
        );
        assert_eq!(
            validate_path(b"C:\\drive\\path.rs"),
            Err(PathValidationError::IsAbsolute)
        );
        assert_eq!(
            validate_path(b"C:/drive/path.rs"),
            Err(PathValidationError::IsAbsolute)
        );
    }

    #[test]
    fn test_nul_byte_rejection() {
        assert_eq!(
            validate_path(b"path\x00null.rs"),
            Err(PathValidationError::ContainsNulByte)
        );
        assert_eq!(
            validate_path(b"\x00start.rs"),
            Err(PathValidationError::ContainsNulByte)
        );
    }

    #[test]
    fn test_path_truncation() {
        let long_path = vec![b'a'; 5000];
        let result = validate_path(&long_path).unwrap();

        assert!(result.len() <= MAX_PATH_LENGTH);
        assert!(result.ends_with(TRUNCATED_MARKER));

        let expected_content_len = MAX_PATH_LENGTH - TRUNCATED_MARKER.len();
        assert_eq!(
            &result[..expected_content_len],
            &vec![b'a'; expected_content_len][..]
        );
    }

    #[test]
    fn test_path_cache() {
        let mut cache = PathCache::new();

        let path1 = b"src/main.rs".to_vec();
        let path2 = b"src/lib.rs".to_vec();

        let id1_first = cache.get_or_insert(path1.clone());
        let id1_second = cache.get_or_insert(path1);
        let id2 = cache.get_or_insert(path2);

        assert_eq!(id1_first, id1_second); // Same path should get same ID
        assert_ne!(id1_first, id2); // Different paths should get different IDs
    }

    #[test]
    fn test_edge_cases() {
        // Single dot is valid
        assert_eq!(validate_path(b".").unwrap(), b".");
        assert_eq!(validate_path(b"./test").unwrap(), b"./test");

        // Three dots is not parent segment
        assert_eq!(validate_path(b"...").unwrap(), b"...");

        // Leading/trailing separators
        assert_eq!(
            validate_path(b"/").unwrap_err(),
            PathValidationError::IsAbsolute
        );

        // Multiple separators
        assert_eq!(validate_path(b"a//b").unwrap(), b"a/b");
        assert_eq!(validate_path(b"a\\\\b").unwrap(), b"a/b");
    }
}
