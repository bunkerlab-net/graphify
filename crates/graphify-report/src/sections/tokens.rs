//! Token-cost formatting helper.
//!
//! Extracted from `lib.rs` because `fmt_comma` is called by multiple section
//! renderers; centralising it here avoids duplication.

/// Format an integer with thousands-separator commas, mirroring Python's
/// `f"{n:,}"` format spec.
#[must_use]
pub(crate) fn fmt_comma(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}
