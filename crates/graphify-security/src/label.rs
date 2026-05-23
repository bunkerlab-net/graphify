//! Sanitisation for free-form text rendered into HTML / embedded into JSON.

use std::sync::LazyLock;

use regex::Regex;

const MAX_LABEL_LEN: usize = 256;

#[allow(clippy::expect_used)] // static regex pattern is a literal; cannot fail at runtime
static CONTROL_CHARS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\x00-\x1f\x7f]").expect("static control-char regex"));

/// Strip control characters and cap length at 256 grapheme-aware characters.
///
/// Safe for embedding in JSON inside `<script>` tags. For direct HTML
/// injection, wrap the result with [`htmlescape::encode_minimal`].
///
/// `None` is treated as `""`.
#[must_use]
pub fn sanitize_label(text: Option<&str>) -> String {
    let Some(text) = text else {
        return String::new();
    };
    let cleaned = CONTROL_CHARS.replace_all(text, "");
    if cleaned.chars().count() > MAX_LABEL_LEN {
        cleaned.chars().take(MAX_LABEL_LEN).collect()
    } else {
        cleaned.into_owned()
    }
}
