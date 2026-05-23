//! Scoring helpers: normalisation, entropy, shingles, Jaro-Winkler, variant guards.

use std::sync::LazyLock;

use regex::Regex;

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

#[allow(clippy::expect_used)] // literal pattern; cannot panic at runtime.
static NON_ALNUM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^a-z0-9]+").expect("static non-alnum regex"));

/// Variant-suffix regex.  Matches labels whose trailing token is a
/// version/variant suffix (chip SKUs, codename revisions, etc.).
/// Requires the stem to end in a letter.
#[allow(clippy::expect_used)] // literal pattern; cannot panic at runtime.
pub static VARIANT_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(.*[a-z])([0-9]+[a-z]*|[a-z]{2,})$").expect("static variant-suffix regex")
});

// ── normalisation ─────────────────────────────────────────────────────────────

/// Lowercase + collapse runs of non-alphanumeric characters to a single space,
/// then strip leading/trailing whitespace.
///
/// Matches `_norm` in the Python source.
#[must_use]
pub fn norm(label: &str) -> String {
    let lower = label.to_lowercase();
    let replaced = NON_ALNUM.replace_all(&lower, " ");
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
