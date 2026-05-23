//! Best-effort process niceness and RSS cap.

/// Apply best-effort nice(10) + optional RSS memory cap.
///
/// Reads `GRAPHIFY_REBUILD_MEMORY_LIMIT_MB` from the environment. Uses
/// `RLIMIT_DATA` on macOS and `RLIMIT_AS` on Linux, silently skipping
/// when the platform does not support it.
///
/// Ports `_apply_resource_limits` from Python.
///
/// Called from hook shell scripts — the Python entrypoint is referenced
/// by those scripts and this Rust equivalent must exist at the same
/// symbol path so the hooks continue to work after the Rust port lands.
pub fn apply_resource_limits() {
    // Best-effort only — failures are silently swallowed, matching Python.
    #[cfg(unix)]
    {
        // SAFETY: nice(2) is always safe to call; we ignore the return value.
        #[allow(unsafe_code)] // reason: libc::nice has no safe Rust wrapper
        unsafe {
            libc::nice(10);
        }
        let mb_str = std::env::var("GRAPHIFY_REBUILD_MEMORY_LIMIT_MB").unwrap_or_default();
        let mb_str = mb_str.trim();
        if mb_str.is_empty() {
            return;
        }
        let Ok(mb) = mb_str.parse::<u64>() else {
            return;
        };
        let limit = mb * 1024 * 1024;
        // SAFETY: setrlimit is safe to call with valid resource constants.
        #[allow(unsafe_code)] // reason: libc::setrlimit/getrlimit have no safe Rust wrapper
        unsafe {
            #[cfg(target_os = "macos")]
            let resource = libc::RLIMIT_DATA;
            #[cfg(not(target_os = "macos"))]
            let resource = libc::RLIMIT_AS;

            let mut rl = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            if libc::getrlimit(resource, &raw mut rl) == 0 {
                let new_hard = if rl.rlim_max != libc::RLIM_INFINITY && rl.rlim_max < limit {
                    rl.rlim_max
                } else {
                    limit
                };
                let new_rl = libc::rlimit {
                    rlim_cur: limit,
                    rlim_max: new_hard,
                };
                libc::setrlimit(resource, &raw const new_rl);
            }
        }
    }
}
