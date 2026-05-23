//! Text-shaping helpers: YAML scalar escaping, URL → filename slugification,
//! URL classification, HTML → markdown conversion.

use std::fmt::Write as FmtWrite;
use std::path::Path;

use crate::regexes::{RE_MULTI_UNDERSCORE, RE_SAFE_FILENAME, RE_SCRIPT, RE_STYLE};

/// Escape a string for embedding in a YAML double-quoted scalar.
///
/// Handles every YAML 1.1/1.2 line-break and control character that could
/// let a hostile value break out of the quoted scalar and inject sibling
/// YAML keys (F-009 / F-019). Matches the Python reference byte-for-byte.
#[must_use]
pub fn yaml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let cp = ch as u32;
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            _ if cp == 0x2028 => out.push_str("\\L"),
            _ if cp == 0x2029 => out.push_str("\\P"),
            _ if cp < 0x20 || cp == 0x7F => {
                // known-good format string; write! only errors on OOM
                let _ = write!(out, "\\x{cp:02x}");
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Turn a URL into a safe filename.
///
/// Joins `netloc + path`, replaces every non-word-or-hyphen character with
/// `_`, collapses consecutive underscores, trims leading/trailing
/// underscores, truncates to 80 chars, then appends `suffix`.
#[must_use]
pub fn safe_filename(url: &str, suffix: &str) -> String {
    let name = match url::Url::parse(url) {
        Ok(u) => {
            let netloc = u.host_str().unwrap_or("");
            let path = u.path();
            format!("{netloc}{path}")
        }
        Err(_) => url.to_string(),
    };
    let name = RE_SAFE_FILENAME.replace_all(&name, "_");
    let name = name.trim_matches('_');
    let name = RE_MULTI_UNDERSCORE.replace_all(name, "_");
    let truncated: String = name.chars().take(80).collect();
    format!("{truncated}{suffix}")
}

/// Classify a URL for targeted extraction.
///
/// Returns one of `"tweet"`, `"arxiv"`, `"github"`, `"youtube"`, `"pdf"`,
/// `"image"`, or `"webpage"`.
#[must_use]
pub fn detect_url_type(url: &str) -> &'static str {
    let lower = url.to_lowercase();
    if lower.contains("twitter.com") || lower.contains("x.com") {
        return "tweet";
    }
    if lower.contains("arxiv.org") {
        return "arxiv";
    }
    if lower.contains("github.com") {
        return "github";
    }
    if lower.contains("youtube.com") || lower.contains("youtu.be") {
        return "youtube";
    }
    let ext = url::Url::parse(url)
        .ok()
        .and_then(|u| {
            Path::new(u.path())
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
        })
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => return "pdf",
        "png" | "jpg" | "jpeg" | "webp" | "gif" => return "image",
        _ => {}
    }
    "webpage"
}

/// Convert HTML to clean markdown, pre-stripping `<script>` and `<style>`
/// blocks so their text never leaks into the output.
#[must_use]
pub fn html_to_markdown(html: &str) -> String {
    let html = RE_SCRIPT.replace_all(html, "");
    let html = RE_STYLE.replace_all(html.as_ref(), "");
    html2md::parse_html(html.as_ref())
}
