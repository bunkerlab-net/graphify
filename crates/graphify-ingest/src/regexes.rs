//! Static regex patterns shared across ingest helpers.
//!
//! Every pattern is a fixed literal that has been validated to compile;
//! `unwrap()` inside `LazyLock` is an invariant rather than a panic risk.

use std::sync::LazyLock;

use regex::Regex;

#[allow(clippy::unwrap_used)]
pub(crate) static RE_SAFE_FILENAME: LazyLock<Regex> = LazyLock::new(|| {
    // Replace anything that isn't a word char or hyphen.
    Regex::new(r"[^\w\-]").unwrap()
});

#[allow(clippy::unwrap_used)]
pub(crate) static RE_MULTI_UNDERSCORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_+").unwrap());

#[allow(clippy::unwrap_used)]
pub(crate) static RE_SCRIPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<script[^>]*>.*?</script>").unwrap());

#[allow(clippy::unwrap_used)]
pub(crate) static RE_STYLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<style[^>]*>.*?</style>").unwrap());

#[allow(clippy::unwrap_used)]
pub(crate) static RE_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<title[^>]*>(.*?)</title>").unwrap());

#[allow(clippy::unwrap_used)]
pub(crate) static RE_WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

#[allow(clippy::unwrap_used)]
pub(crate) static RE_TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

#[allow(clippy::unwrap_used)]
pub(crate) static RE_ARXIV_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{4}\.\d{4,5})").unwrap());

#[allow(clippy::unwrap_used)]
pub(crate) static RE_ARXIV_ABSTRACT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?si)class="abstract[^"]*"[^>]*>(.*?)</blockquote>"#).unwrap());

#[allow(clippy::unwrap_used)]
pub(crate) static RE_ARXIV_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?si)class="title[^"]*"[^>]*>(.*?)</h1>"#).unwrap());

#[allow(clippy::unwrap_used)]
pub(crate) static RE_ARXIV_AUTHORS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?si)class="authors"[^>]*>(.*?)</div>"#).unwrap());

#[allow(clippy::unwrap_used)]
pub(crate) static RE_NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w]").unwrap());
