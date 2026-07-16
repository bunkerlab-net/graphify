//! `graphify` CLI binary.
//!
//! Ports `graphify-py/graphify/__main__.py`. All orchestration lives in
//! [`cli::run`]; this file is intentionally just an entry point so the
//! testable surface stays inside the `cli` module tree.

mod cli;

fn main() {
    // #1807: a downstream reader that closes stdout early (`| head`, `sed q`)
    // disconnects the pipe mid-write. The guarded `outln!`/`out!` macros already
    // catch that at each write site; this is the backstop for a `BrokenPipe`
    // (or raw EPIPE/EINVAL) that surfaced as an `io::Error` anywhere in the
    // error chain — treat it as success, matching graphify-py's console wrapper.
    match cli::run() {
        Ok(()) => {}
        Err(err) => {
            if is_broken_pipe(&err) {
                std::process::exit(0);
            }
            // Preserve the Debug (anyhow chain) formatting the previous
            // `Result`-returning main produced via `Termination`.
            eprintln!("Error: {err:?}");
            std::process::exit(1);
        }
    }
}

/// `true` when any error in the chain is a closed-pipe write (#1807): an
/// `io::Error` of kind `BrokenPipe`, or a raw EPIPE (32) / EINVAL (22).
fn is_broken_pipe(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<std::io::Error>().is_some_and(|io| {
            io.kind() == std::io::ErrorKind::BrokenPipe
                || matches!(io.raw_os_error(), Some(32 | 22))
        })
    })
}
