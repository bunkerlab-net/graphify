//! Sanitisation for free-form text rendered into HTML / embedded into JSON.

use std::sync::LazyLock;

use regex::Regex;

const MAX_LABEL_LEN: usize = 256;

#[allow(clippy::expect_used)] // static regex pattern is a literal; cannot fail at runtime
static CONTROL_CHARS: LazyLock<Regex> = LazyLock::new(|| {
    // C0 (U+0000–U+001F) + DEL (U+007F) + C1 (U+0080–U+009F) + the two
    // JavaScript-grammar line terminators U+2028 / U+2029 that would
    // otherwise break embedded JSON in `<script>` tags.
    Regex::new(r"[\x00-\x1f\x7f\u{80}-\u{9f}\u{2028}\u{2029}]").expect("static control-char regex")
});

/// Strip control characters and cap length at 256 chars (not graphemes).
///
/// Safe for embedding in JSON inside `<script>` tags. For direct HTML
/// injection, wrap the result with [`htmlescape::encode_minimal`].
///
/// `None` is treated as `""`. The length cap counts `char`s, which
/// matches Python's `len(str)` on a code-point basis but is not
/// grapheme-aware (so a combining-mark cluster counts as multiple
/// chars).
#[must_use]
pub fn sanitize_label(text: Option<&str>) -> String {
    let Some(text) = text else {
        return String::new();
    };
    let cleaned = CONTROL_CHARS.replace_all(text, "");
    let mut chars = cleaned.chars();
    let truncated: String = chars.by_ref().take(MAX_LABEL_LEN).collect();
    // Iterate only once: if there are leftover chars, we hit the cap.
    if chars.next().is_some() {
        truncated
    } else {
        cleaned.into_owned()
    }
}
