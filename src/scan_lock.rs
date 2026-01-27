use crate::cache_path;
use anyhow::{anyhow, Context, Result};
use std::fs::File;
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

        // Ensure cache directory exists and is safe
        cache_path::ensure_cache_dir(cache_dir)?;

        // Open or create lock file safely
        let file = cache_path::create_file_secure(&lock_path).with_context(|| {
            format!(
                "error: unable to open scan lock file {}",
                lock_path.display()
            )
        })?;

        // Try to acquire exclusive lock (non-blocking)
        if Self::try_lock_file(&file).is_err() {
            return Err(anyhow!(
                "error: another scan is in progress (scan.lock is held)"
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
    use std::path::{Path, PathBuf};
    use std::sync::{mpsc, Arc};
    use std::thread;
    use tempfile::tempdir;

    fn real_path(path: &Path) -> PathBuf {
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
    }

    #[test]
    fn test_single_process_acquires_lock() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        let cache_dir = base.join(".regret");
        std::fs::create_dir_all(&cache_dir).unwrap();

        let lock = ScanLock::try_acquire(&cache_dir);
        assert!(lock.is_ok(), "Single process should acquire lock");

        let lock = lock.unwrap();
        assert!(lock.lock_path().exists());
    }

    #[test]
    fn test_second_process_fails_with_clear_error() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        let cache_dir = base.join(".regret");
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
        let base = real_path(temp.path());
        let cache_dir = base.join(".regret");
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
        let base = real_path(temp.path());
        let nonexistent_dir = base.join("does_not_exist");

        let result = ScanLock::try_acquire(&nonexistent_dir);
        assert!(result.is_ok(), "Should create cache directory if missing");
    }

    #[test]
    fn test_concurrent_lock_attempts() {
        let temp = tempdir().unwrap();
        let base = real_path(temp.path());
        let cache_dir = Arc::new(base.join(".regret"));
        std::fs::create_dir_all(cache_dir.as_ref()).unwrap();

        let cache_dir_clone = cache_dir.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let _lock = ScanLock::try_acquire(&cache_dir_clone).unwrap();
            ready_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });

        ready_rx.recv().unwrap();

        // This should fail while first thread holds lock
        let lock2_result = ScanLock::try_acquire(&cache_dir);
        assert!(lock2_result.is_err(), "Concurrent lock should fail");

        release_tx.send(()).unwrap();
        handle.join().unwrap();

        // After first thread releases lock, we should be able to acquire it
        let lock3 = ScanLock::try_acquire(&cache_dir);
        assert!(lock3.is_ok(), "Should acquire lock after release");
    }
}
