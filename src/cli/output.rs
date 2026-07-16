//! Guarded stdout writers for the CLI (#1807).
//!
//! The `outln!`/`out!` macros (defined in the parent [`crate::cli`] module)
//! replace `println!`/`print!` across `src/cli/**`. A
//! downstream reader that closes the pipe early (`| head`, `Select-Object
//! -First N`, `sed q`) disconnects stdout mid-write; an unguarded `println!`
//! then panics ("failed printing to stdout") and the process exits 101/141, so
//! CI wrappers and agent harnesses read a successful query as a failure. These
//! macros treat a closed pipe as success (exit 0) and any other write error as
//! a hard failure (exit 1). Mirrors graphify-py's `_silence_broken_pipe`.
//!
//! Accepted gap: the library crates (`graphify-*`) still emit their status
//! lines through bare `println!`, which are not routed through this guard. The
//! guarded macros plus the `main`-level `BrokenPipe` mapping cover
//! every path the #1807 regression tests exercise; if a library print is ever
//! observed to panic on a closed pipe, route it through here too.

/// Handle a stdout write error. A closed-pipe reader is treated as success and
/// exits 0 (#1807); any other write error prints to stderr and exits 1. Never
/// returns.
pub(crate) fn handle_stdout_error(e: &std::io::Error) -> ! {
    if is_broken_pipe_write(e) {
        std::process::exit(0);
    }
    eprintln!("graphify: error writing to stdout: {e}");
    std::process::exit(1);
}

/// Whether a stdout write error is a downstream reader closing the pipe.
///
/// `std` maps EPIPE (POSIX) and `ERROR_NO_DATA` (Windows) to `BrokenPipe`, so
/// that kind covers the closed-pipe case on every platform. Windows *also*
/// surfaces a write to a closed pipe as EINVAL (raw 22); that raw code is only
/// treated as a broken pipe there — on Unix, EINVAL is a genuine error and must
/// not be silently mapped to success (matches graphify-py's errno handling).
fn is_broken_pipe_write(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::BrokenPipe || (cfg!(windows) && e.raw_os_error() == Some(22))
}

#[cfg(test)]
mod tests {
    use super::is_broken_pipe_write;
    use std::io::{Error, ErrorKind};

    #[test]
    fn broken_pipe_kind_is_detected() {
        assert!(is_broken_pipe_write(&Error::from(ErrorKind::BrokenPipe)));
    }

    #[test]
    fn unrelated_stdout_error_is_not_a_broken_pipe() {
        // A non-pipe write failure must NOT be mapped to a success exit — on
        // Unix a raw EINVAL (22) is a genuine error, not a closed pipe.
        assert!(!is_broken_pipe_write(&Error::from(
            ErrorKind::PermissionDenied
        )));
        assert!(!is_broken_pipe_write(&Error::from_raw_os_error(28))); // ENOSPC
        #[cfg(not(windows))]
        assert!(!is_broken_pipe_write(&Error::from_raw_os_error(22))); // EINVAL on Unix
    }
}
