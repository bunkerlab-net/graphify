//! Helpers for building Obsidian wikilinks from community labels.

use std::sync::LazyLock;

use regex::Regex;

/// Characters to strip from community labels when building Obsidian
/// wikilinks. Regex literal is validated at compile-time via the
/// `LazyLock` initialiser.
#[allow(clippy::expect_used)] // literal pattern; cannot fail at runtime
static UNSAFE_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"[\\/*?:"<>|#^\[\]]"#).expect("literal pattern is valid"));

/// Strip `.md` / `.mdx` / `.markdown` suffix (case-insensitive).
#[allow(clippy::expect_used)] // literal pattern; cannot fail at runtime
static MD_EXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.(md|mdx|markdown)$").expect("literal pattern is valid"));

/// Normalise a community label for use as a wikilink target.
///
/// - Collapses `\r\n`/`\r`/`\n` to spaces.
/// - Strips Obsidian-unsafe characters (`\\ / * ? : " < > | # ^ [ ]`).
/// - Strips `.md` / `.mdx` / `.markdown` extensions.
/// - Returns `"unnamed"` if everything was stripped.
///
/// Mirrors Python `_safe_community_name`.
pub(crate) fn safe_community_name(label: &str) -> String {
    let normalised = label.replace("\r\n", " ").replace(['\r', '\n'], " ");
    let cleaned = UNSAFE_CHARS.replace_all(&normalised, "");
    let cleaned = cleaned.trim();
    let cleaned = MD_EXT.replace(cleaned, "");
    if cleaned.is_empty() {
        "unnamed".to_string()
    } else {
        cleaned.into_owned()
    }
}
