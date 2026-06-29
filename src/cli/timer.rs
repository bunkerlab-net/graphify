//! Per-stage wall-clock timing for `--timing` (#1490).
//!
//! Mirrors Python `_StageTimer` in `__main__.py`: monotonic, diagnostic-only.
//! Emits `[graphify timing] <stage>: N.Ns` to stderr after each stage and a
//! final total. Off by default, so normal output is byte-identical and the
//! machine-read stdout / `graph.json` are untouched.

use std::time::Instant;

/// Tracks elapsed time between stage marks, printing to stderr when enabled.
pub(crate) struct StageTimer {
    enabled: bool,
    start: Instant,
    last: Instant,
}

impl StageTimer {
    /// Create a timer; `enabled` gates all output (off → silent no-op).
    #[must_use]
    pub(crate) fn new(enabled: bool) -> Self {
        let now = Instant::now();
        Self {
            enabled,
            start: now,
            last: now,
        }
    }

    /// Print the elapsed time since the previous mark as `<stage>` and reset the
    /// per-stage clock.
    pub(crate) fn mark(&mut self, stage: &str) {
        let now = Instant::now();
        if self.enabled {
            eprintln!(
                "[graphify timing] {stage}: {:.1}s",
                now.duration_since(self.last).as_secs_f64()
            );
        }
        self.last = now;
    }

    /// Print the total elapsed time since construction.
    pub(crate) fn total(&self) {
        if self.enabled {
            eprintln!(
                "[graphify timing] total: {:.1}s",
                self.start.elapsed().as_secs_f64()
            );
        }
    }
}
