//! Frontmatter parsing for memory docs.
//!
//! `save_query_result` writes a tiny hand-built YAML subset (no `PyYAML`
//! dependency), so we parse the same subset by hand: scalar `key: "value"`
//! lines and a `source_nodes: ["a", "b"]` flow list. Anything unrecognised is
//! ignored, so foreign `.md` files in `memory/` are skipped cleanly.

use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

#[allow(clippy::expect_used)] // literal patterns; cannot fail at runtime.
static SCALAR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^([A-Za-z_][\w-]*):\s*"(.*)"\s*$"#).expect("scalar regex"));
#[allow(clippy::expect_used)] // literal patterns; cannot fail at runtime.
static LIST_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z_][\w-]*):\s*\[(.*)\]\s*$").expect("list regex"));
#[allow(clippy::expect_used)] // literal patterns; cannot fail at runtime.
static DQ_ITEM_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""((?:[^"\\]|\\.)*)""#).expect("item regex"));

/// Parsed frontmatter of one memory doc.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MemoryDoc {
    /// The `type` field (e.g. `query`, `explain`).
    pub doc_type: Option<String>,
    /// ISO date string; empty when absent.
    pub date: String,
    /// The question text; empty when absent.
    pub question: String,
    /// Outcome signal (`useful` / `dead_end` / `corrected`), if marked.
    pub outcome: Option<String>,
    /// Correction text for a `corrected` outcome.
    pub correction: Option<String>,
    /// Cited source-node labels (always a list, possibly empty).
    pub source_nodes: Vec<String>,
    /// Source filename, set by [`load_memory_docs`] (empty from a bare parse).
    pub path: String,
}

/// Reverse the double-quoted escaping that `ingest::yaml_str` applies.
#[must_use]
pub(crate) fn yaml_unescape(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '\\' && i + 1 < chars.len() {
            let nxt = chars[i + 1];
            let simple = match nxt {
                'n' => Some('\n'),
                'r' => Some('\r'),
                't' => Some('\t'),
                '0' => Some('\0'),
                '"' => Some('"'),
                '\\' => Some('\\'),
                'L' => Some('\u{2028}'),
                'P' => Some('\u{2029}'),
                _ => None,
            };
            if let Some(c) = simple {
                out.push(c);
                i += 2;
                continue;
            }
            if nxt == 'x'
                && i + 3 < chars.len()
                && let Some(c) = hex_char(&chars[i + 2..=i + 3])
            {
                out.push(c);
                i += 4;
                continue;
            }
            if nxt == 'u'
                && i + 5 < chars.len()
                && let Some(c) = hex_char(&chars[i + 2..=i + 5])
            {
                out.push(c);
                i += 6;
                continue;
            }
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Decode a run of hex digits to a `char`, or `None` if invalid.
fn hex_char(digits: &[char]) -> Option<char> {
    let hex: String = digits.iter().collect();
    u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
}

/// Parse the frontmatter of a memory doc, or `None` if it has none.
#[must_use]
pub fn parse_memory_doc(text: &str) -> Option<MemoryDoc> {
    if !text.starts_with("---") {
        return None;
    }
    let lines: Vec<&str> = text.lines().collect();
    if lines.first().map(|l| l.trim()) != Some("---") {
        return None;
    }
    let mut doc = MemoryDoc::default();
    for line in &lines[1..] {
        if line.trim() == "---" {
            break;
        }
        if let Some(caps) = LIST_RE.captures(line)
            && &caps[1] == "source_nodes"
        {
            doc.source_nodes = DQ_ITEM_RE
                .captures_iter(&caps[2])
                .map(|c| yaml_unescape(&c[1]))
                .collect();
            continue;
        }
        if let Some(caps) = SCALAR_RE.captures(line) {
            let val = yaml_unescape(&caps[2]);
            match &caps[1] {
                "type" => doc.doc_type = Some(val),
                "date" => doc.date = val,
                "question" => doc.question = val,
                "outcome" => doc.outcome = Some(val),
                "correction" => doc.correction = Some(val),
                _ => {}
            }
        }
    }
    Some(doc)
}

/// Parse every memory doc under `memory_dir`, sorted by date then filename.
///
/// Docs without recognisable frontmatter (foreign `.md` files, the `LESSONS.md`
/// artifact) are skipped.
#[must_use]
pub fn load_memory_docs(memory_dir: &Path) -> Vec<MemoryDoc> {
    if !memory_dir.exists() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(memory_dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    paths.sort();

    let mut docs: Vec<MemoryDoc> = Vec::new();
    for path in paths {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(mut doc) = parse_memory_doc(&text) {
            doc.path = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            docs.push(doc);
        }
    }
    // Stable order: (date, filename) so output is deterministic across runs.
    docs.sort_by(|a, b| (&a.date, &a.path).cmp(&(&b.date, &b.path)));
    docs
}
