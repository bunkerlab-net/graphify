//! Scoring and aggregation of parsed memory docs into a lessons structure.

use std::cmp::Ordering;
use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, NaiveDateTime, Utc};
use indexmap::IndexMap;

use crate::graph::doc_community;
use crate::parse::MemoryDoc;

/// Rounding for the signed score keeps sort order and the contested verdict
/// stable across platforms (the last ULP of `powf` can differ).
const SCORE_NDIGITS: i32 = 9;

/// Outcome tallies for a bucket: the three signals plus `unmarked`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OutcomeCounts {
    /// Count of `useful` docs.
    pub useful: usize,
    /// Count of `dead_end` docs.
    pub dead_end: usize,
    /// Count of `corrected` docs.
    pub corrected: usize,
    /// Count of docs with no (or an unrecognised) outcome.
    pub unmarked: usize,
}

/// A positive-only source node: `preferred` (corroborated) or `tentative`.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceEntry {
    /// The cited node id/label.
    pub node: String,
    /// Distinct `useful` results citing it.
    pub n: usize,
    /// Signed, time-decayed score (used for ordering only).
    pub score: f64,
}

/// A node with both positive and negative signals; recency decides the verdict.
#[derive(Clone, Debug, PartialEq)]
pub struct ContestedEntry {
    /// The cited node id/label.
    pub node: String,
    /// Distinct `useful` results.
    pub pos: usize,
    /// Distinct `dead_end`/`corrected` results.
    pub neg: usize,
    /// Signed, time-decayed score.
    pub score: f64,
    /// `"useful"`, `"dead end"`, or `"even"` by the sign of `score`.
    pub verdict: String,
    /// Most recent event date seen for this node.
    pub last: String,
}

/// A `dead_end` question and the nodes it cited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadEnd {
    /// The question that led nowhere.
    pub question: String,
    /// Cited source nodes.
    pub nodes: Vec<String>,
    /// ISO date.
    pub date: String,
}

/// A `corrected` question and the right answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Correction {
    /// The question that was corrected.
    pub question: String,
    /// What the right answer was.
    pub correction: String,
    /// ISO date.
    pub date: String,
}

/// One community's (or the overall) finalized lessons.
#[derive(Clone, Debug)]
pub struct Bucket {
    /// Outcome tallies for this bucket.
    pub counts: OutcomeCounts,
    /// Corroborated positive-only sources.
    pub preferred: Vec<SourceEntry>,
    /// Not-yet-corroborated positive-only sources.
    pub tentative: Vec<SourceEntry>,
    /// Mixed-signal sources.
    pub contested: Vec<ContestedEntry>,
    /// Dead-end questions.
    pub dead_ends: Vec<DeadEnd>,
    /// Corrections.
    pub corrections: Vec<Correction>,
}

/// The full aggregate produced by [`aggregate_lessons`].
#[derive(Clone, Debug)]
pub struct AggResult {
    /// Total docs aggregated.
    pub total: usize,
    /// Overall outcome tallies.
    pub counts: OutcomeCounts,
    /// The corroboration threshold used (echoed for rendering).
    pub min_corroboration: usize,
    /// Overall preferred sources.
    pub preferred: Vec<SourceEntry>,
    /// Overall tentative sources.
    pub tentative: Vec<SourceEntry>,
    /// Overall contested sources.
    pub contested: Vec<ContestedEntry>,
    /// Overall dead ends.
    pub dead_ends: Vec<DeadEnd>,
    /// Overall corrections.
    pub corrections: Vec<Correction>,
    /// Per-community buckets; empty unless a graph was supplied.
    pub by_community: IndexMap<String, Bucket>,
    /// Overall per-node (date, question, outcome) provenance trail, most-recent
    /// order determined by the consumer. Feeds the learning-overlay sidecar.
    pub node_provenance: IndexMap<String, Vec<(String, String, String)>>,
}

/// Mutable accumulator threaded through aggregation.
#[derive(Default)]
struct AggBucket {
    counts: OutcomeCounts,
    node_score: IndexMap<String, f64>,
    node_pos: IndexMap<String, usize>,
    node_neg: IndexMap<String, usize>,
    node_last: IndexMap<String, String>,
    dead_ends: Vec<DeadEnd>,
    corrections: Vec<Correction>,
    /// Per-node (date, question, outcome) trail feeding the learning-overlay
    /// sidecar's provenance; never rendered into LESSONS.md.
    node_provenance: IndexMap<String, Vec<(String, String, String)>>,
}

/// Parse an ISO date/datetime to an aware UTC datetime, or `None`.
fn parse_dt(date_str: &str) -> Option<DateTime<Utc>> {
    if date_str.is_empty() {
        return None;
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(date_str, "%Y-%m-%dT%H:%M:%S") {
        return Some(ndt.and_utc());
    }
    if let Ok(nd) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        return Some(nd.and_hms_opt(0, 0, 0)?.and_utc());
    }
    None
}

/// Time-decay weight in (0, 1]: halves every `half_life_days`. Undated/future
/// signals keep full weight (1.0).
fn decay(date_str: &str, now: DateTime<Utc>, half_life_days: f64) -> f64 {
    let Some(dt) = parse_dt(date_str) else {
        return 1.0;
    };
    if half_life_days <= 0.0 {
        return 1.0;
    }
    // Seconds-since-epoch differences stay far within f64's exact-integer range.
    #[allow(clippy::cast_precision_loss)]
    let age_days = ((now - dt).num_seconds() as f64 / 86_400.0).max(0.0);
    0.5_f64.powf(age_days / half_life_days)
}

/// Round a score to [`SCORE_NDIGITS`] decimal places.
fn round_score(x: f64) -> f64 {
    let factor = 10_f64.powi(SCORE_NDIGITS);
    (x * factor).round() / factor
}

fn record_node(
    b: &mut AggBucket,
    node: &str,
    sign: i32,
    weight: f64,
    date: &str,
    question: &str,
    outcome: &str,
) {
    *b.node_score.entry(node.to_owned()).or_insert(0.0) += f64::from(sign) * weight;
    if sign > 0 {
        *b.node_pos.entry(node.to_owned()).or_insert(0) += 1;
    } else if sign < 0 {
        *b.node_neg.entry(node.to_owned()).or_insert(0) += 1;
    }
    let cur = b.node_last.get(node).map_or("", String::as_str);
    if date > cur {
        b.node_last.insert(node.to_owned(), date.to_owned());
    }
    // Provenance records only useful/corrected events (the experiential trail:
    // what cited this node, and how it turned out), matching graphify-py
    // `_record_node`. A `dead_end` (sign -1) updates the score but leaves no
    // provenance entry, and a neutral `sign == 0` doc never reaches here
    // (`apply_doc` gates the call) - recording either would diverge from the
    // reference (contra CodeRabbit's "record even when sign == 0" suggestion).
    if matches!(outcome, "useful" | "corrected") {
        b.node_provenance.entry(node.to_owned()).or_default().push((
            date.to_owned(),
            question.to_owned(),
            outcome.to_owned(),
        ));
    }
}

fn cmp_score_then_node(sa: f64, na: &str, sb: f64, nb: &str) -> Ordering {
    sb.partial_cmp(&sa)
        .unwrap_or(Ordering::Equal)
        .then_with(|| na.cmp(nb))
}

/// Split a bucket's scored nodes into preferred / tentative / contested.
fn finalize_sources(
    b: &AggBucket,
    k: usize,
) -> (Vec<SourceEntry>, Vec<SourceEntry>, Vec<ContestedEntry>) {
    let mut preferred: Vec<SourceEntry> = Vec::new();
    let mut tentative: Vec<SourceEntry> = Vec::new();
    let mut contested: Vec<ContestedEntry> = Vec::new();
    for (node, raw) in &b.node_score {
        let pos = b.node_pos.get(node).copied().unwrap_or(0);
        let neg = b.node_neg.get(node).copied().unwrap_or(0);
        let score = round_score(*raw);
        if pos > 0 && neg > 0 {
            let verdict = if score > 0.0 {
                "useful"
            } else if score < 0.0 {
                "dead end"
            } else {
                "even"
            };
            contested.push(ContestedEntry {
                node: node.clone(),
                pos,
                neg,
                score,
                verdict: verdict.to_owned(),
                last: b.node_last.get(node).cloned().unwrap_or_default(),
            });
        } else if pos > 0 {
            let entry = SourceEntry {
                node: node.clone(),
                n: pos,
                score,
            };
            if pos >= k {
                preferred.push(entry);
            } else {
                tentative.push(entry);
            }
        }
        // negative-only nodes are surfaced via the dead-ends questions, not here.
    }
    preferred.sort_by(|a, c| cmp_score_then_node(a.score, &a.node, c.score, &c.node));
    tentative.sort_by(|a, c| cmp_score_then_node(a.score, &a.node, c.score, &c.node));
    contested.sort_by(|a, c| cmp_score_then_node(a.score, &a.node, c.score, &c.node));
    (preferred, tentative, contested)
}

/// Collapse repeated questions to one entry (last/most-recent text wins),
/// ordered by (date, question).
fn dedupe_by_question<T>(
    items: Vec<T>,
    question: impl Fn(&T) -> String,
    date: impl Fn(&T) -> String,
) -> Vec<T> {
    let mut latest: IndexMap<String, T> = IndexMap::new();
    for item in items {
        latest.insert(question(&item), item);
    }
    let mut out: Vec<T> = latest.into_values().collect();
    out.sort_by_key(|item| (date(item), question(item)));
    out
}

fn dedupe_dead_ends(items: Vec<DeadEnd>) -> Vec<DeadEnd> {
    dedupe_by_question(items, |d| d.question.clone(), |d| d.date.clone())
}

fn dedupe_corrections(items: Vec<Correction>) -> Vec<Correction> {
    dedupe_by_question(items, |c| c.question.clone(), |c| c.date.clone())
}

/// Apply one doc's signal to a bucket (counts, node scores, dead ends/corrections).
fn apply_doc(b: &mut AggBucket, doc: &MemoryDoc, nodes: &[String], sign: i32, weight: f64) {
    let date = doc.date.as_str();
    match doc.outcome.as_deref() {
        Some("useful") => b.counts.useful += 1,
        Some("dead_end") => b.counts.dead_end += 1,
        Some("corrected") => b.counts.corrected += 1,
        _ => b.counts.unmarked += 1,
    }
    if sign != 0 {
        let outcome = doc.outcome.as_deref().unwrap_or("");
        for n in nodes {
            record_node(b, n, sign, weight, date, &doc.question, outcome);
        }
    }
    match doc.outcome.as_deref() {
        Some("dead_end") => b.dead_ends.push(DeadEnd {
            question: doc.question.clone(),
            nodes: nodes.to_vec(),
            date: doc.date.clone(),
        }),
        Some("corrected") => b.corrections.push(Correction {
            question: doc.question.clone(),
            correction: doc.correction.clone().unwrap_or_default(),
            date: doc.date.clone(),
        }),
        _ => {}
    }
}

fn finalize_bucket(b: &AggBucket, k: usize) -> Bucket {
    let (preferred, tentative, contested) = finalize_sources(b, k);
    Bucket {
        counts: b.counts.clone(),
        preferred,
        tentative,
        contested,
        dead_ends: dedupe_dead_ends(b.dead_ends.clone()),
        corrections: dedupe_corrections(b.corrections.clone()),
    }
}

/// Aggregate parsed memory docs into a deterministic lessons structure.
///
/// `now` anchors the time-decay (pass it explicitly for byte-stable output).
/// `known_nodes` (when given) gates out source nodes no longer in the graph.
/// `by_community` is empty unless `node_community` is supplied and non-empty.
#[must_use]
// `None` call sites can't infer a generic hasher; callers build the default-hasher set.
#[allow(clippy::implicit_hasher)]
pub fn aggregate_lessons(
    docs: &[MemoryDoc],
    node_community: Option<&IndexMap<String, String>>,
    now: DateTime<Utc>,
    half_life_days: f64,
    min_corroboration: usize,
    known_nodes: Option<&HashSet<String>>,
) -> AggResult {
    let mut overall = AggBucket::default();
    let mut by_community: IndexMap<String, AggBucket> = IndexMap::new();

    for doc in docs {
        // One event per node per doc; drop nodes the graph no longer knows.
        let mut seen: HashSet<&str> = HashSet::new();
        let nodes: Vec<String> = doc
            .source_nodes
            .iter()
            .filter(|n| known_nodes.is_none_or(|k| k.contains(n.as_str())))
            .filter(|n| seen.insert(n.as_str()))
            .cloned()
            .collect();
        let community = doc_community(&nodes, node_community);

        let sign = match doc.outcome.as_deref() {
            Some("useful") => 1,
            Some("dead_end" | "corrected") => -1,
            _ => 0,
        };
        let weight = if sign == 0 {
            0.0
        } else {
            decay(&doc.date, now, half_life_days)
        };

        let bucket = by_community.entry(community).or_default();
        for target in [&mut overall, bucket] {
            apply_doc(target, doc, &nodes, sign, weight);
        }
    }

    let mut community_out: IndexMap<String, Bucket> = IndexMap::new();
    if node_community.is_some_and(|m| !m.is_empty()) {
        for (label, b) in &by_community {
            community_out.insert(label.clone(), finalize_bucket(b, min_corroboration));
        }
    }

    let (preferred, tentative, contested) = finalize_sources(&overall, min_corroboration);
    AggResult {
        total: docs.len(),
        counts: overall.counts.clone(),
        min_corroboration,
        preferred,
        tentative,
        contested,
        dead_ends: dedupe_dead_ends(overall.dead_ends),
        corrections: dedupe_corrections(overall.corrections),
        by_community: community_out,
        node_provenance: overall.node_provenance,
    }
}
