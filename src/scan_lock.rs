use anyhow::{anyhow, Context, Result};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// Exclusive advisory lock for scan operations
///
/// Implements RAII semantics - lock is automatically released when dropped.
/// Uses platform-specific locking:
/// - Unix: flock(2)
/// - Windows: LockFileEx
#[derive(Debug)]
#[allow(dead_code)]
pub struct ScanLock {
    _file: File,
    #[allow(dead_code)]
    lock_path: PathBuf,
}

impl ScanLock {
    /// Attempt to acquire exclusive advisory lock for scanning
    ///
    /// Lock file location: <cache_dir>/scan.lock
    /// Returns error if another scan is in progress
    #[allow(dead_code)]
    pub fn try_acquire(cache_dir: &Path) -> Result<Self> {
        let lock_path = cache_dir.join("scan.lock");

        // Ensure cache directory exists
        if !cache_dir.exists() {
            return Err(anyhow!(
                "error: cache directory does not exist: {}",
                cache_dir.display()
            ));
        }

        // Open or create lock file
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| {
                format!(
                    "error: unable to open scan lock file {}",
                    lock_path.display()
                )
            })?;

        // Try to acquire exclusive lock (non-blocking)
        if let Err(e) = Self::try_lock_file(&file) {
            return Err(anyhow!(
                "error: another scan is in progress (scan.lock is held): {}",
                e
            ));
        }

        Ok(Self {
            _file: file,
            lock_path,
        })
    }

    /// Platform-specific non-blocking exclusive lock
    #[cfg(unix)]
    fn try_lock_file(file: &File) -> Result<()> {
        use std::os::unix::io::AsRawFd;

        // LOCK_EX | LOCK_NB (exclusive, non-blocking)
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };

        if result == 0 {
            Ok(())
        } else {
            let err = std::io::Error::last_os_error();
            Err(anyhow!("flock failed: {}", err))
        }
    }

    /// Platform-specific non-blocking exclusive lock
    #[cfg(windows)]
    fn try_lock_file(file: &File) -> Result<()> {
        use std::os::windows::io::AsRawHandle;
        use winapi::um::fileapi::LockFileEx;
        use winapi::um::minwinbase::OVERLAPPED;
        use winapi::um::winbase::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY};

        let handle = file.as_raw_handle();
        let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };

        let result = unsafe {
            LockFileEx(
                handle,
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                0,
                !0, // Lock entire file
                0,
                &mut overlapped,
            )
        };

        if result != 0 {
            Ok(())
        } else {
            let err = std::io::Error::last_os_error();
            Err(anyhow!("LockFileEx failed: {}", err))
        }
    }

    /// Get the path to the lock file
    #[allow(dead_code)]
    pub fn lock_path(&self) -> &Path {
        &self.lock_path
    }
}

// Lock is automatically released when ScanLock is dropped
// No explicit unlock needed due to RAII semantics
impl Drop for ScanLock {
    fn drop(&mut self) {
        // File handle is closed automatically, releasing the lock
        // No explicit unlock needed for advisory locks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn test_single_process_acquires_lock() {
        let temp = tempdir().unwrap();
        let cache_dir = temp.path().join(".regret");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let lock = ScanLock::try_acquire(&cache_dir);
        assert!(lock.is_ok(), "Single process should acquire lock");

        let lock = lock.unwrap();
        assert!(lock.lock_path().exists());
    }

    #[test]
    fn test_second_process_fails_with_clear_error() {
        let temp = tempdir().unwrap();
        let cache_dir = temp.path().join(".regret");
        std::fs::create_dir_all(&cache_dir).unwrap();

        // First lock succeeds
        let _lock1 = ScanLock::try_acquire(&cache_dir).unwrap();

        // Second lock fails
        let lock2_result = ScanLock::try_acquire(&cache_dir);
        assert!(lock2_result.is_err(), "Second lock should fail");

        let error_msg = format!("{}", lock2_result.unwrap_err());
        assert!(
            error_msg.contains("another scan is in progress"),
            "Error message should be clear: {}",
            error_msg
        );
        assert!(
            error_msg.contains("scan.lock is held"),
            "Error message should mention lock file: {}",
            error_msg
        );
    }

    #[test]
    fn test_lock_released_after_scope_exit() {
        let temp = tempdir().unwrap();
        let cache_dir = temp.path().join(".regret");
        std::fs::create_dir_all(&cache_dir).unwrap();

        // Acquire and release lock in scope
        {
            let _lock = ScanLock::try_acquire(&cache_dir).unwrap();
            // Lock is held here
        } // Lock is released here when _lock is dropped

        // Should be able to acquire lock again
        let lock2 = ScanLock::try_acquire(&cache_dir);
        assert!(
            lock2.is_ok(),
            "Should be able to acquire lock after release"
        );
    }

    #[test]
    fn test_nonexistent_cache_dir() {
        let temp = tempdir().unwrap();
        let nonexistent_dir = temp.path().join("does_not_exist");

        let result = ScanLock::try_acquire(&nonexistent_dir);
        assert!(result.is_err(), "Should fail for nonexistent directory");

        let error_msg = format!("{}", result.unwrap_err());
        assert!(
            error_msg.contains("cache directory does not exist"),
            "Error should mention missing directory: {}",
            error_msg
        );
    }

    #[test]
    fn test_concurrent_lock_attempts() {
        let temp = tempdir().unwrap();
        let cache_dir = Arc::new(temp.path().join(".regret"));
        std::fs::create_dir_all(cache_dir.as_ref()).unwrap();

        let cache_dir_clone = cache_dir.clone();
        let handle = thread::spawn(move || {
            let _lock = ScanLock::try_acquire(&cache_dir_clone).unwrap();
            // Hold lock for a short time
            thread::sleep(std::time::Duration::from_millis(100));
        });

        // Give first thread time to acquire lock
        thread::sleep(std::time::Duration::from_millis(10));

        // This should fail while first thread holds lock
        let lock2_result = ScanLock::try_acquire(&cache_dir);
        assert!(lock2_result.is_err(), "Concurrent lock should fail");

        handle.join().unwrap();

        // After first thread releases lock, we should be able to acquire it
        thread::sleep(std::time::Duration::from_millis(50));
        let lock3 = ScanLock::try_acquire(&cache_dir);
        assert!(lock3.is_ok(), "Should acquire lock after release");
    }
}
