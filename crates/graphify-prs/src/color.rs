//! Thin ANSI-color helpers. Mirrors Python's `_c`, `green`, `red`, etc.
//!
//! Color output is suppressed when:
//! - `NO_COLOR` is set in the environment, OR
//! - stdout is not a terminal.
//!
//! The `is_terminal` check uses the `IsTerminal` trait stabilised in Rust 1.70.

use std::io::IsTerminal as _;
use std::sync::LazyLock;

use regex::Regex;

/// `true` when ANSI escape codes should be omitted.
static NO_COLOR: LazyLock<bool> =
    LazyLock::new(|| std::env::var_os("NO_COLOR").is_some() || !std::io::stdout().is_terminal());

// Regex matching ANSI escape sequences; used by `pad`.
#[allow(clippy::unwrap_used)] // literal is valid; invariant upheld by construction
static ANSI_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*m").unwrap());

/// Wrap `text` in the given ANSI SGR escape code.
#[must_use]
pub fn ansi(code: &str, text: &str) -> String {
    if *NO_COLOR {
        text.to_string()
    } else {
        format!("\x1b[{code}m{text}\x1b[0m")
    }
}

#[must_use]
pub fn green(t: &str) -> String {
    ansi("32", t)
}

#[must_use]
pub fn red(t: &str) -> String {
    ansi("31", t)
}

#[must_use]
pub fn yellow(t: &str) -> String {
    ansi("33", t)
}

#[must_use]
pub fn cyan(t: &str) -> String {
    ansi("36", t)
}

#[must_use]
pub fn bold(t: &str) -> String {
    ansi("1", t)
}

#[must_use]
pub fn dim(t: &str) -> String {
    ansi("2", t)
}

#[must_use]
pub fn magenta(t: &str) -> String {
    ansi("35", t)
}

/// Pad an ANSI-colored string to the given *visible* width.
#[must_use]
pub fn pad(s: &str, width: usize) -> String {
    let visible = ANSI_RE.replace_all(s, "").len();
    let spaces = width.saturating_sub(visible);
    format!("{s}{}", " ".repeat(spaces))
}
