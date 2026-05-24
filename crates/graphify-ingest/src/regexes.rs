//! Static regex patterns shared across ingest helpers.
//!
//! Every pattern is a fixed literal validated at compile time; `expect`
//! documents the invariant that the pattern must be syntactically valid.

use std::sync::LazyLock;

use regex::Regex;

/// Matches any character that is not a word character (`\w`) or a hyphen,
/// used to sanitise URL components into safe filename segments.
#[allow(clippy::expect_used)]
pub(crate) static RE_SAFE_FILENAME: LazyLock<Regex> = LazyLock::new(|| {
    // Replace anything that isn't a word char or hyphen.
    Regex::new(r"[^\w\-]").expect("literal pattern is valid")
});

/// Matches one or more consecutive underscores, used to collapse duplicate
/// separators produced after [`RE_SAFE_FILENAME`] substitution.
#[allow(clippy::expect_used)]
pub(crate) static RE_MULTI_UNDERSCORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_+").expect("literal pattern is valid"));

/// Matches `<script>…</script>` blocks (case-insensitive, dot-matches-newline)
/// so they can be stripped before HTML→Markdown conversion.
#[allow(clippy::expect_used)]
pub(crate) static RE_SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?si)<script[^>]*>.*?</script>").expect("literal pattern is valid")
});

/// Matches `<style>…</style>` blocks (case-insensitive, dot-matches-newline)
/// so they can be stripped before HTML→Markdown conversion.
#[allow(clippy::expect_used)]
pub(crate) static RE_STYLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?si)<style[^>]*>.*?</style>").expect("literal pattern is valid")
});

/// Captures the text content of an HTML `<title>` element.
#[allow(clippy::expect_used)]
pub(crate) static RE_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?si)<title[^>]*>(.*?)</title>").expect("literal pattern is valid")
});

/// Matches one or more whitespace characters, used to normalise title text.
#[allow(clippy::expect_used)]
pub(crate) static RE_WHITESPACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+").expect("literal pattern is valid"));

/// Matches any HTML tag, used to strip markup from tweet and arXiv text.
#[allow(clippy::expect_used)]
pub(crate) static RE_TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[^>]+>").expect("literal pattern is valid"));

/// Captures an arXiv paper ID in `YYMM.NNNNN` format from a URL.
#[allow(clippy::expect_used)]
pub(crate) static RE_ARXIV_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{4}\.\d{4,5})").expect("literal pattern is valid"));

/// Captures the abstract text from an arXiv abstract page.
///
/// Matches the `<blockquote class="abstract ...">…</blockquote>` element.
#[allow(clippy::expect_used)]
pub(crate) static RE_ARXIV_ABSTRACT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)class="abstract[^"]*"[^>]*>(.*?)</blockquote>"#)
        .expect("literal pattern is valid")
});

/// Captures the paper title from an arXiv abstract page `<h1 class="title …">` element.
#[allow(clippy::expect_used)]
pub(crate) static RE_ARXIV_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)class="title[^"]*"[^>]*>(.*?)</h1>"#).expect("literal pattern is valid")
});

/// Captures the author list from an arXiv abstract page `<div class="authors">` element.
#[allow(clippy::expect_used)]
pub(crate) static RE_ARXIV_AUTHORS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)class="authors"[^>]*>(.*?)</div>"#).expect("literal pattern is valid")
});

/// Matches any non-word character, used to slugify query text into filenames.
#[allow(clippy::expect_used)]
pub(crate) static RE_NON_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\w]").expect("literal pattern is valid"));
