//! Scoring helpers: normalisation, entropy, shingles, Jaro-Winkler, variant guards.

use std::sync::LazyLock;

use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::minhash::MinHash;

// ── constants ─────────────────────────────────────────────────────────────────

/// Shannon entropy threshold below which a label is skipped by fuzzy matching.
pub const ENTROPY_THRESHOLD: f64 = 2.5;
/// LSH threshold (Jaccard) — candidates whose Jaccard estimate is below this
/// are never paired.
pub const LSH_THRESHOLD: f64 = 0.7;
/// Jaro-Winkler score (× 100) at or above which two nodes are merged.
pub const MERGE_THRESHOLD: f64 = 92.0;
/// Score bonus when both nodes share the same community and their labels are
/// long enough (≥ 12 chars in normalised form).
pub const COMMUNITY_BOOST: f64 = 5.0;

// ── static regex ─────────────────────────────────────────────────────────────

// `\W` in the Rust `regex` crate is Unicode-aware by default (matches anything
// that is not a Unicode word character), so `[\W_]+` collapses runs of
// non-alphanumeric characters while preserving CJK and other Unicode letters
// — matching Python's `re.sub(r"[\W_]+", " ", s, flags=re.UNICODE)`.
#[allow(clippy::expect_used)] // literal pattern; cannot panic at runtime.
static NON_WORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\W_]+").expect("static non-word regex"));

/// Variant-suffix regex.  Matches labels whose trailing token is a
/// version/variant suffix (chip SKUs, codename revisions, etc.).
/// Requires the stem to end in a letter.
#[allow(clippy::expect_used)] // literal pattern; cannot panic at runtime.
pub static VARIANT_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.*[a-z])([0-9]+[a-z]*|[a-z]{2,})$").expect("static variant-suffix regex")
});

/// Digit-run regex for [`numeric_tokens_differ`] — matches maximal runs of
/// decimal digits (`\d+`). Unicode-aware by default in the `regex` crate,
/// matching Python's `re.compile(r"\d+")` under `re.UNICODE`.
#[allow(clippy::expect_used)] // literal pattern; cannot panic at runtime.
static DIGIT_RUN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d+").expect("static digit-run regex"));

// ── normalisation ─────────────────────────────────────────────────────────────

/// Casefold + collapse runs of non-word characters (Unicode-aware) to a
/// single space, then strip leading/trailing whitespace.
///
/// Mirrors Python's
/// `re.sub(r"[\W_]+", " ", unicodedata.normalize("NFKC", label).casefold(),
/// flags=re.UNICODE).strip()`. Uses the `caseless` crate's full Unicode
/// case folding (the Rust equivalent of Python's `str.casefold()`), not
/// `str::to_lowercase`, so German `ß` folds to `ss` and the Greek final
/// sigma normalises correctly — both of which are needed for parity with
/// Python on non-ASCII identifiers.
#[must_use]
pub fn norm(label: &str) -> String {
    let nfkc: String = label.nfkc().collect();
    let folded: String = caseless::default_case_fold_str(&nfkc);
    let replaced = NON_WORD.replace_all(&folded, " ");
    replaced.trim().to_string()
}

// ── entropy ───────────────────────────────────────────────────────────────────

/// Shannon entropy in bits/char of the normalised label.
///
/// Matches `_entropy` in the Python source.
#[must_use]
pub fn entropy(label: &str) -> f64 {
    let s = norm(label);
    if s.is_empty() {
        return 0.0;
    }
    // label lengths and char counts are small; cast to f64 is safe here.
    #[allow(clippy::cast_precision_loss)] // label strings are short; no precision loss in practice.
    let n = s.len() as f64;
    let mut freq: std::collections::HashMap<char, u32> = std::collections::HashMap::new();
    for ch in s.chars() {
        *freq.entry(ch).or_insert(0) += 1;
    }
    -freq.values().fold(0.0_f64, |acc, &c| {
        #[allow(clippy::cast_precision_loss)] // char counts are small; fits in f64.
        let p = f64::from(c) / n;
        acc + p * p.log2()
    })
}

// ── shingles ─────────────────────────────────────────────────────────────────

/// Return k-gram character shingles of `text`.
/// If `text` is shorter than `k`, returns a single element containing `text`.
///
/// Matches `_shingles` in the Python source.
#[must_use]
pub fn shingles(text: &str, k: usize) -> Vec<String> {
    if text.len() < k {
        return vec![text.to_string()];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut result: Vec<String> = (0..=chars.len() - k)
        .map(|i| chars[i..i + k].iter().collect())
        .collect();
    result.sort_unstable();
    result.dedup();
    result
}

/// Build a `MinHash` sketch from a normalised label string.
///
/// Spaces are stripped before shingling so that `"graph extractor"` and
/// `"graphextractor"` share shingles — matching Python's `_make_minhash`.
#[must_use]
pub fn make_minhash(norm_label: &str) -> MinHash {
    let compact = norm_label.replace(' ', "");
    let mut m = MinHash::new();
    for s in shingles(&compact, 3) {
        m.update(s.as_bytes());
    }
    m
}

// ── variant guard ─────────────────────────────────────────────────────────────

/// Returns `true` if `a` and `b` are sibling model/SKU variants (same stem,
/// different suffix). Only applied to short labels (< 12 chars).
///
/// Matches `_is_variant_pair` in the Python source.
#[must_use]
pub fn is_variant_pair(a: &str, b: &str) -> bool {
    if a == b {
        return false;
    }
    if a.len().max(b.len()) >= 12 {
        return false;
    }
    let ma = VARIANT_SUFFIX.captures(a);
    let mb = VARIANT_SUFFIX.captures(b);
    match (ma, mb) {
        (Some(ca), Some(cb)) => {
            let stem_a = ca.get(1).map_or("", |m| m.as_str());
            let stem_b = cb.get(1).map_or("", |m| m.as_str());
            let suf_a = ca.get(2).map_or("", |m| m.as_str());
            let suf_b = cb.get(2).map_or("", |m| m.as_str());
            stem_a == stem_b && suf_a != suf_b
        }
        _ => false,
    }
}

/// Returns `true` when a fuzzy merge of `a` and `b` should be blocked.
///
/// Short labels (< 12 chars) produce spuriously high Jaro-Winkler scores due
/// to the prefix bonus.  We only allow merges for same-length single-char
/// substitutions (genuine typos like `"Extractor"`/`"Extractar"`).
///
/// Matches `_short_label_blocked` in the Python source.
#[must_use]
pub fn short_label_blocked(a: &str, b: &str, jw_score: f64) -> bool {
    if a.len().max(b.len()) >= 12 {
        return false;
    }
    if jw_score >= 97.0 && a.len() == b.len() && damerau_levenshtein(a, b) <= 1 {
        return false;
    }
    true
}

/// Simple Damerau-Levenshtein distance (includes adjacent transpositions).
/// Used only for the `short_label_blocked` guard on very short strings so the
/// result only needs to be accurate for the `<= 1` comparison.
fn damerau_levenshtein(a: &str, b: &str) -> usize {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let m = ac.len();
    let n = bc.len();
    let mut dp = vec![vec![0usize; n + 1]; m + 1];

    for (i, row) in dp.iter_mut().enumerate().take(m + 1) {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate().take(n + 1) {
        *cell = j;
    }

    for i in 1..=m {
        for j in 1..=n {
            let cost = usize::from(ac[i - 1] != bc[j - 1]);
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && ac[i - 1] == bc[j - 2] && ac[i - 2] == bc[j - 1] {
                dp[i][j] = dp[i][j].min(dp[i - 2][j - 2] + 1);
            }
        }
    }
    dp[m][n]
}

/// Compute Jaro-Winkler similarity score in the range `[0, 100]`.
///
/// Wraps `strsim::jaro_winkler` and scales to match the Python
/// `JaroWinkler.normalized_similarity * 100` convention.
#[must_use]
pub fn jaro_winkler_score(a: &str, b: &str) -> f64 {
    strsim::jaro_winkler(a, b) * 100.0
}

/// Compute plain Jaro similarity score in the range `[0, 100]`.
///
/// Wraps `strsim::jaro` and scales to match the Python
/// `Jaro.normalized_similarity * 100` convention. Unlike [`jaro_winkler_score`]
/// it carries no leading-prefix bonus, so cross-file long labels that merely
/// share a prefix but diverge in a distinguishing token ("testing library jest
/// native" vs "testing library react native") fall short of the merge threshold
/// instead of being fabricated into a merge (#1243).
#[must_use]
pub fn jaro_score(a: &str, b: &str) -> f64 {
    strsim::jaro(a, b) * 100.0
}

/// Returns `true` when `a` and `b` carry different embedded numbers (#1284).
///
/// Long labels that differ only in their digit runs ("adr 0011 d5" vs
/// "adr 0013 d4", "3 1 product goals" vs "1 1 product goals", "block3" vs
/// "block13") are numbered/versioned siblings, not duplicates — but the long
/// shared boilerplate keeps Jaro-Winkler above [`MERGE_THRESHOLD`], and
/// [`is_variant_pair`] only covers short trailing suffixes. Digit runs are
/// compared as multisets with leading zeros stripped, so zero-padding ("09" vs
/// "9") is not a difference. Comparison is on the stripped strings (not parsed
/// integers), matching Python's `_numeric_tokens_differ`. Labels with identical
/// numbers, or none at all, are unaffected.
#[must_use]
pub fn numeric_tokens_differ(a: &str, b: &str) -> bool {
    if a == b {
        return false;
    }
    let mut ta: Vec<&str> = DIGIT_RUN
        .find_iter(a)
        .map(|m| strip_leading_zeros(m.as_str()))
        .collect();
    let mut tb: Vec<&str> = DIGIT_RUN
        .find_iter(b)
        .map(|m| strip_leading_zeros(m.as_str()))
        .collect();
    ta.sort_unstable();
    tb.sort_unstable();
    ta != tb
}

/// Strip leading ASCII zeros from a digit run, mapping an all-zero run to `"0"`.
/// Mirrors Python's `t.lstrip("0") or "0"`.
fn strip_leading_zeros(t: &str) -> &str {
    let stripped = t.trim_start_matches('0');
    if stripped.is_empty() { "0" } else { stripped }
}
