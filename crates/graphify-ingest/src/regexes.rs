//! Static regex patterns shared across ingest helpers.
//!
//! Every pattern is a fixed literal validated at compile time; `expect`
//! documents the invariant that the pattern must be syntactically valid.

use std::sync::LazyLock;

use regex::Regex;

#[allow(clippy::expect_used)]
pub(crate) static RE_SAFE_FILENAME: LazyLock<Regex> = LazyLock::new(|| {
    // Replace anything that isn't a word char or hyphen.
    Regex::new(r"[^\w\-]").expect("literal pattern is valid")
});

#[allow(clippy::expect_used)]
pub(crate) static RE_MULTI_UNDERSCORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_+").expect("literal pattern is valid"));

#[allow(clippy::expect_used)]
pub(crate) static RE_SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?si)<script[^>]*>.*?</script>").expect("literal pattern is valid")
});

#[allow(clippy::expect_used)]
pub(crate) static RE_STYLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?si)<style[^>]*>.*?</style>").expect("literal pattern is valid")
});

#[allow(clippy::expect_used)]
pub(crate) static RE_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?si)<title[^>]*>(.*?)</title>").expect("literal pattern is valid")
});

#[allow(clippy::expect_used)]
pub(crate) static RE_WHITESPACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s+").expect("literal pattern is valid"));

#[allow(clippy::expect_used)]
pub(crate) static RE_TAGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<[^>]+>").expect("literal pattern is valid"));

#[allow(clippy::expect_used)]
pub(crate) static RE_ARXIV_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{4}\.\d{4,5})").expect("literal pattern is valid"));

#[allow(clippy::expect_used)]
pub(crate) static RE_ARXIV_ABSTRACT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)class="abstract[^"]*"[^>]*>(.*?)</blockquote>"#)
        .expect("literal pattern is valid")
});

#[allow(clippy::expect_used)]
pub(crate) static RE_ARXIV_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)class="title[^"]*"[^>]*>(.*?)</h1>"#).expect("literal pattern is valid")
});

#[allow(clippy::expect_used)]
pub(crate) static RE_ARXIV_AUTHORS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?si)class="authors"[^>]*>(.*?)</div>"#).expect("literal pattern is valid")
});

#[allow(clippy::expect_used)]
pub(crate) static RE_NON_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\w]").expect("literal pattern is valid"));
