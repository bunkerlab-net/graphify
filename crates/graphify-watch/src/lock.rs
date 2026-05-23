//! Per-repo advisory rebuild lock.
//!
//! Ports `_rebuild_lock` from `graphify-py/graphify/watch.py`.
//!
//! Uses `flock(2)` on POSIX platforms so the lock is released automatically
//! if the process is killed — no stale-lock cleanup needed.  On non-POSIX
//! platforms (Windows) the lock is a no-op that always reports acquired.
//!
//! While the lock is held, `.rebuild.lock` contains the owning PID followed
//! by a newline so external pollers can read it.  On release the file is
//! unlinked so downstream tooling that polls for its absence unblocks promptly.

use std::path::{Path, PathBuf};

use crate::WatchError;

/// RAII guard returned by [`RebuildLock::acquire`].
///
/// Drops (releases + unlinks) the lock file when it goes out of scope.
pub struct RebuildLock {
    /// Path to the `.rebuild.lock` file.
    lock_path: PathBuf,
    /// Whether *this* guard holds the lock (false for non-blocking misses).
    acquired: bool,
    /// The open file descriptor on POSIX; `()` on non-POSIX.
    #[cfg(unix)]
    fd: std::os::fd::OwnedFd,
}

impl RebuildLock {
    /// Attempt to acquire the per-repo rebuild lock.
    ///
    /// Returns `Ok(guard)` where `guard.acquired()` reports whether the lock
    /// was obtained.  When `blocking` is `false` and another holder has the
    /// lock, the guard is returned with `acquired() == false`.
    ///
    /// # Errors
    ///
    /// Returns `WatchError::Io` if the lock file cannot be opened or if
    /// `flock` fails for a reason other than `EWOULDBLOCK`.
    pub fn acquire(out_dir: &Path, blocking: bool) -> Result<Self, WatchError> {
        #[cfg(unix)]
        {
            Self::acquire_unix(out_dir, blocking)
        }
        #[cfg(not(unix))]
        {
            let _ = (out_dir, blocking);
            Ok(Self {
                lock_path: out_dir.join(".rebuild.lock"),
                acquired: true,
            })
        }
    }

    /// `true` if this guard holds the lock.
    #[must_use]
    pub fn acquired(&self) -> bool {
        self.acquired
    }

    /// POSIX implementation of [`RebuildLock::acquire`] using `flock(2)`.
    ///
    /// Opens (or creates) `.rebuild.lock` in `out_dir`, calls `flock` with
    /// `LOCK_EX` (and optionally `LOCK_NB` for non-blocking), then writes the
    /// current PID into the file so external pollers can identify the holder.
    #[cfg(unix)]
    fn acquire_unix(out_dir: &Path, blocking: bool) -> Result<Self, WatchError> {
        use std::io::{Seek, Write};
        use std::os::fd::IntoRawFd;

        std::fs::create_dir_all(out_dir).map_err(WatchError::Io)?;
        let lock_path = out_dir.join(".rebuild.lock");

        // "a+" creates without truncating an existing holder's PID payload.
        let mut fh = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&lock_path)
            .map_err(WatchError::Io)?;

        let raw_fd = {
            use std::os::unix::io::AsRawFd;
            fh.as_raw_fd()
        };

        let flags = if blocking {
            libc::LOCK_EX
        } else {
            libc::LOCK_EX | libc::LOCK_NB
        };

        // SAFETY: raw_fd was just unwrapped from an open File; flock semantics are well-defined.
        #[allow(unsafe_code)] // reason: libc::flock has no safe Rust wrapper; FD is valid
        let rc = unsafe { libc::flock(raw_fd, flags) };
        if rc != 0 {
            let errno = std::io::Error::last_os_error();
            // EWOULDBLOCK / EAGAIN — lock is held by another process.
            if errno.raw_os_error() == Some(libc::EWOULDBLOCK) {
                // Non-blocking miss: return a guard that reports not-acquired.
                // We must not truncate the holder's PID, so just drop fh.
                // SAFETY: into_raw_fd transfers ownership; OwnedFd resumes it.
                #[allow(unsafe_code)] // reason: transferring fd ownership from File to OwnedFd
                let fd = unsafe {
                    use std::os::fd::FromRawFd;
                    std::os::fd::OwnedFd::from_raw_fd(fh.into_raw_fd())
                };
                return Ok(Self {
                    lock_path,
                    acquired: false,
                    fd,
                });
            }
            return Err(WatchError::Io(errno));
        }

        // We hold the lock. Truncate and write our PID.
        fh.seek(std::io::SeekFrom::Start(0))
            .map_err(WatchError::Io)?;
        fh.set_len(0).map_err(WatchError::Io)?;
        writeln!(fh, "{}", std::process::id()).map_err(WatchError::Io)?;
        fh.flush().map_err(WatchError::Io)?;

        // SAFETY: into_raw_fd transfers ownership; OwnedFd resumes it.
        #[allow(unsafe_code)] // reason: transferring fd ownership from File to OwnedFd
        let fd = unsafe {
            use std::os::fd::FromRawFd;
            std::os::fd::OwnedFd::from_raw_fd(fh.into_raw_fd())
        };

        Ok(Self {
            lock_path,
            acquired: true,
            fd,
        })
    }
}

impl Drop for RebuildLock {
    /// Releases the advisory lock and unlinks the lock file.
    fn drop(&mut self) {
        if !self.acquired {
            return;
        }

        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // SAFETY: fd is valid until we drop it.
            #[allow(unsafe_code)] // reason: libc::flock has no safe Rust wrapper; FD is valid
            unsafe {
                libc::flock(self.fd.as_raw_fd(), libc::LOCK_UN);
            }
        }

        // Unlink so downstream waiters that poll for absence unblock promptly.
        let _ = std::fs::remove_file(&self.lock_path);
    }
}
