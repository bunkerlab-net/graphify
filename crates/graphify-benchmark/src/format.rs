//! Human-readable formatters for [`BenchmarkResult`] values.

use std::fmt::Write as _;

use crate::types::BenchmarkResult;

/// Return a horizontal rule of `width` U+2500 (`─`) box-drawing
/// characters.
///
/// Mirrors Python `_hr()`. Rust's stdout is always UTF-8 so no encoding
/// fallback is needed.
#[must_use]
pub fn hr(width: usize) -> String {
    "─".repeat(width)
}

/// Format a benchmark result as a human-readable multi-line string.
///
/// Mirrors Python `print_benchmark`. Returns the error message when
/// `result` is `None` (i.e. no nodes matched any sample question).
#[must_use]
pub fn format_benchmark(result: Option<&BenchmarkResult>) -> String {
    let Some(r) = result else {
        return "Benchmark error: No matching nodes found for sample questions. Build the graph first.\n".to_string();
    };

    let mut out = String::new();
    out.push_str("\ngraphify token reduction benchmark\n");
    out.push_str(&hr(50));
    out.push('\n');
    // `write!` on `String` is infallible; the `let _ =` silences the
    // `unused_must_use` warning without hiding real errors.
    let _ = writeln!(
        out,
        "  Corpus:          {} words → ~{} tokens (naive)",
        format_with_commas(r.corpus_words),
        format_with_commas(r.corpus_tokens),
    );
    let _ = writeln!(
        out,
        "  Graph:           {} nodes, {} edges",
        format_with_commas(r.nodes),
        format_with_commas(r.edges),
    );
    let _ = writeln!(
        out,
        "  Avg query cost:  ~{} tokens",
        format_with_commas(r.avg_query_tokens),
    );
    let _ = writeln!(
        out,
        "  Reduction:       {}x fewer tokens per query",
        r.reduction_ratio,
    );
    out.push_str("\n  Per question:\n");
    for p in &r.per_question {
        let truncated = if p.question.len() > 55 {
            &p.question[..55]
        } else {
            &p.question
        };
        let _ = writeln!(out, "    [{}x] {truncated}", p.reduction);
    }
    out.push('\n');
    out
}

/// Print a benchmark result to stdout.
///
/// Python parity: `print_benchmark` calls `print()` on the formatted
/// string. In Rust, `print!` is used so the caller is not forced to add
/// a trailing newline (the [`format_benchmark`] string already ends
/// with `\n`).
pub fn print_benchmark(result: Option<&BenchmarkResult>) {
    print!("{}", format_benchmark(result));
}

/// Format a number with comma thousands separators
/// (e.g. `1_234_567` → `"1,234,567"`).
#[must_use]
fn format_with_commas(n: usize) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let offset = bytes.len() % 3;
    for (i, &b) in bytes.iter().enumerate() {
        if i > 0 && (i % 3 == offset) {
            out.push(',');
        }
        out.push(b as char);
    }
    out
}
