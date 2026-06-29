//! Parity tests against `graphify-py/tests/test_reflect.py`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;

use chrono::{DateTime, Duration, TimeZone, Utc};
use graphify_ingest::save_query_result;
use graphify_reflect::{
    AggResult, GraphPaths, MemoryDoc, aggregate_lessons, lessons_fresh, load_memory_docs,
    parse_memory_doc, reflect, render_lessons_md,
};
use indexmap::IndexMap;

/// Fixed clock so time-decay scoring is byte-stable (mirrors Python `_NOW`).
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 1, 0, 0, 0).unwrap()
}

fn days_before(n: i64) -> String {
    (now() - Duration::days(n)).to_rfc3339()
}

/// Build a `MemoryDoc` (mirrors Python `_doc`).
fn doc(
    outcome: Option<&str>,
    nodes: &[&str],
    question: &str,
    correction: &str,
    date: &str,
) -> MemoryDoc {
    MemoryDoc {
        doc_type: None,
        date: date.to_string(),
        question: question.to_string(),
        outcome: outcome.map(str::to_string),
        correction: if correction.is_empty() {
            None
        } else {
            Some(correction.to_string())
        },
        source_nodes: nodes.iter().map(|s| (*s).to_string()).collect(),
        path: String::new(),
    }
}

/// `aggregate_lessons` with the test defaults (no graph, fixed clock, k=2).
fn agg(docs: &[MemoryDoc]) -> AggResult {
    aggregate_lessons(docs, None, now(), 30.0, 2, None)
}

fn community(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(n, c)| ((*n).to_string(), (*c).to_string()))
        .collect()
}

// --- frontmatter parsing -------------------------------------------------------

#[test]
fn parse_round_trips_a_saved_doc() {
    let tmp = tempfile::tempdir().unwrap();
    let mem = tmp.path().join("memory");
    let out = save_query_result(
        "what is \"attention\"?",
        "softmax",
        &mem,
        "explain",
        Some(&["AttentionLayer".to_string(), "SoftmaxFunc".to_string()]),
        Some("useful"),
        None,
    )
    .unwrap();
    let text = std::fs::read_to_string(&out).unwrap();
    let parsed = parse_memory_doc(&text).expect("frontmatter parses");
    assert_eq!(parsed.doc_type.as_deref(), Some("explain"));
    assert_eq!(parsed.question, "what is \"attention\"?");
    assert_eq!(parsed.outcome.as_deref(), Some("useful"));
    assert_eq!(parsed.source_nodes, vec!["AttentionLayer", "SoftmaxFunc"]);
}

#[test]
fn parse_returns_none_for_foreign_doc() {
    assert!(parse_memory_doc("# just a note\n\nno frontmatter here\n").is_none());
    assert!(parse_memory_doc("").is_none());
}

#[test]
fn round_trip_survives_backslash_newline_and_quoted_node() {
    let tmp = tempfile::tempdir().unwrap();
    let mem = tmp.path().join("memory");
    let out = save_query_result(
        r#"path is C:\Users and a "quote""#,
        "a",
        &mem,
        "query",
        Some(&[r#"Node"With\Quote"#.to_string()]),
        Some("corrected"),
        Some("line1\nline2"),
    )
    .unwrap();
    let parsed = parse_memory_doc(&std::fs::read_to_string(&out).unwrap()).unwrap();
    assert_eq!(parsed.question, r#"path is C:\Users and a "quote""#);
    assert_eq!(parsed.correction.as_deref(), Some("line1\nline2"));
    assert_eq!(parsed.source_nodes, vec![r#"Node"With\Quote"#]);
}

#[test]
fn parse_handles_crlf() {
    let doc = "---\r\ntype: \"query\"\r\noutcome: \"useful\"\r\nsource_nodes: [\"A\"]\r\n---\r\n# body\r\n";
    let parsed = parse_memory_doc(doc).unwrap();
    assert_eq!(parsed.outcome.as_deref(), Some("useful"));
    assert_eq!(parsed.source_nodes, vec!["A"]);
}

#[test]
fn load_memory_docs_missing_dir_is_empty() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(load_memory_docs(&tmp.path().join("nope")).is_empty());
}

fn write_raw_doc(mem: &std::path::Path, filename: &str, date: &str, outcome: &str, question: &str) {
    std::fs::create_dir_all(mem).unwrap();
    let body = format!(
        "---\ntype: \"query\"\ndate: \"{date}\"\nquestion: \"{question}\"\ncontributor: \"graphify\"\noutcome: \"{outcome}\"\n---\n\n# Q: {question}\n"
    );
    std::fs::write(mem.join(filename), body).unwrap();
}

#[test]
fn load_memory_docs_skips_foreign_and_sorts() {
    let tmp = tempfile::tempdir().unwrap();
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem).unwrap();
    std::fs::write(mem.join("foreign.md"), "# not a memory doc\n").unwrap();
    write_raw_doc(&mem, "a.md", "2026-01-01", "useful", "first");
    write_raw_doc(&mem, "b.md", "2026-01-02", "dead_end", "second");
    let docs = load_memory_docs(&mem);
    assert_eq!(docs.len(), 2);
    let outcomes: HashSet<&str> = docs.iter().filter_map(|d| d.outcome.as_deref()).collect();
    assert_eq!(outcomes, HashSet::from(["useful", "dead_end"]));
}

#[test]
fn load_memory_docs_orders_by_date_then_filename() {
    let tmp = tempfile::tempdir().unwrap();
    let mem = tmp.path().join("memory");
    write_raw_doc(&mem, "z.md", "2026-03-01", "dead_end", "march");
    write_raw_doc(&mem, "a.md", "2026-01-01", "dead_end", "january");
    write_raw_doc(&mem, "b.md", "2026-02-01", "dead_end", "february");
    let docs = load_memory_docs(&mem);
    let questions: Vec<&str> = docs.iter().map(|d| d.question.as_str()).collect();
    assert_eq!(questions, vec!["january", "february", "march"]);
}

// --- aggregation ---------------------------------------------------------------

#[test]
fn aggregate_counts_each_outcome() {
    let docs = [
        doc(Some("useful"), &["A"], "q", "", "2026-01-01"),
        doc(Some("useful"), &["A", "B"], "q", "", "2026-01-01"),
        doc(Some("dead_end"), &["C"], "q", "", "2026-01-01"),
        doc(Some("corrected"), &[], "q", "use D", "2026-01-01"),
        doc(None, &[], "q", "", "2026-01-01"),
    ];
    let a = agg(&docs);
    assert_eq!(a.total, 5);
    assert_eq!(a.counts.useful, 2);
    assert_eq!(a.counts.dead_end, 1);
    assert_eq!(a.counts.corrected, 1);
    assert_eq!(a.counts.unmarked, 1);
}

fn node_names(entries: &[graphify_reflect::SourceEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.node.as_str()).collect()
}

#[test]
fn sources_split_into_preferred_tentative_contested() {
    let docs = [
        doc(Some("useful"), &["A", "B"], "q", "", "2026-01-01"),
        doc(Some("useful"), &["A", "B"], "q", "", "2026-01-01"),
        doc(Some("useful"), &["C"], "q", "", "2026-01-01"),
        doc(Some("dead_end"), &["A"], "q", "", "2026-01-01"),
    ];
    let a = agg(&docs);
    assert_eq!(node_names(&a.preferred), vec!["B"]);
    assert_eq!(node_names(&a.tentative), vec!["C"]);
    assert_eq!(
        a.contested
            .iter()
            .map(|e| e.node.as_str())
            .collect::<Vec<_>>(),
        vec!["A"]
    );
}

#[test]
fn corroboration_threshold_promotes_only_repeated_nodes() {
    let one = agg(&[doc(Some("useful"), &["A"], "q", "", "2026-01-01")]);
    assert_eq!(node_names(&one.tentative), vec!["A"]);
    assert!(one.preferred.is_empty());

    let two = agg(&[
        doc(Some("useful"), &["A"], "q", "", "2026-01-01"),
        doc(Some("useful"), &["A"], "q", "", "2026-01-01"),
    ]);
    assert_eq!(node_names(&two.preferred), vec!["A"]);
    assert!(two.tentative.is_empty());
}

#[test]
fn recency_decides_contested_verdict() {
    let a = agg(&[
        doc(Some("useful"), &["N"], "q", "", &days_before(120)),
        doc(Some("dead_end"), &["N"], "q", "", &days_before(1)),
    ]);
    assert_eq!(a.contested.len(), 1);
    assert_eq!(a.contested[0].node, "N");
    assert_eq!(a.contested[0].verdict, "dead end");

    let flipped = agg(&[
        doc(Some("useful"), &["N"], "q", "", &days_before(1)),
        doc(Some("dead_end"), &["N"], "q", "", &days_before(120)),
    ]);
    assert_eq!(flipped.contested[0].verdict, "useful");
}

#[test]
fn node_existence_gate_drops_stale_nodes() {
    let docs = [
        doc(Some("useful"), &["Alive", "Deleted"], "q", "", "2026-01-01"),
        doc(Some("useful"), &["Alive", "Deleted"], "q", "", "2026-01-01"),
    ];
    let known: HashSet<String> = HashSet::from(["Alive".to_string()]);
    let a = aggregate_lessons(&docs, None, now(), 30.0, 2, Some(&known));
    let names: Vec<&str> = a
        .preferred
        .iter()
        .chain(&a.tentative)
        .map(|e| e.node.as_str())
        .collect();
    assert!(!names.contains(&"Deleted"));
    assert!(names.contains(&"Alive"));
}

#[test]
fn corroboration_counts_distinct_docs_not_citations() {
    let a = agg(&[doc(Some("useful"), &["A", "A"], "q", "", "2026-01-01")]);
    assert!(a.preferred.is_empty());
    assert_eq!(node_names(&a.tentative), vec!["A"]);
    assert_eq!(a.tentative[0].n, 1);
}

#[test]
fn min_corroboration_is_honored_not_hardcoded() {
    let docs = [
        doc(Some("useful"), &["A"], "q", "", "2026-01-01"),
        doc(Some("useful"), &["A"], "q", "", "2026-01-01"),
    ];
    let k2 = aggregate_lessons(&docs, None, now(), 30.0, 2, None);
    assert_eq!(node_names(&k2.preferred), vec!["A"]);
    let k3 = aggregate_lessons(&docs, None, now(), 30.0, 3, None);
    assert!(k3.preferred.is_empty());
    assert_eq!(node_names(&k3.tentative), vec!["A"]);
}

#[test]
fn half_life_actually_feeds_decay() {
    let docs = [
        doc(Some("useful"), &["N"], "q", "", &days_before(90)),
        doc(Some("useful"), &["N"], "q", "", &days_before(90)),
        doc(Some("dead_end"), &["N"], "q", "", &days_before(1)),
    ];
    let long_hl = aggregate_lessons(&docs, None, now(), 100_000.0, 2, None);
    let short_hl = aggregate_lessons(&docs, None, now(), 10.0, 2, None);
    assert_eq!(long_hl.contested[0].verdict, "useful");
    assert_eq!(short_hl.contested[0].verdict, "dead end");
}

#[test]
fn evenly_split_and_nonpositive_half_life() {
    let day = days_before(5);
    let a = agg(&[
        doc(Some("useful"), &["N"], "q", "", &day),
        doc(Some("dead_end"), &["N"], "q", "", &day),
    ]);
    assert_eq!(a.contested[0].verdict, "even");
    assert!(render_lessons_md(&a).contains("evenly split"));

    let docs = [
        doc(Some("useful"), &["N"], "q", "", &days_before(365)),
        doc(Some("dead_end"), &["N"], "q", "", &days_before(1)),
    ];
    let no_decay = aggregate_lessons(&docs, None, now(), 0.0, 2, None);
    assert_eq!(no_decay.contested[0].verdict, "even");
}

#[test]
fn negative_only_node_absent_from_sources() {
    let a = agg(&[doc(Some("dead_end"), &["Bad"], "why?", "", "2026-01-01")]);
    let names: Vec<&str> = a
        .preferred
        .iter()
        .chain(&a.tentative)
        .map(|e| e.node.as_str())
        .collect();
    assert!(!names.contains(&"Bad"));
    assert_eq!(a.dead_ends[0].nodes, vec!["Bad"]);
}

#[test]
fn dead_ends_and_corrections_collected() {
    let a = agg(&[
        doc(
            Some("dead_end"),
            &["RedisClient"],
            "where is the cache?",
            "",
            "2026-01-01",
        ),
        doc(
            Some("corrected"),
            &[],
            "what hashes pw?",
            "bcrypt",
            "2026-01-01",
        ),
    ]);
    assert_eq!(a.dead_ends[0].question, "where is the cache?");
    assert_eq!(a.dead_ends[0].nodes, vec!["RedisClient"]);
    assert_eq!(a.corrections[0].correction, "bcrypt");
}

#[test]
fn no_community_grouping_without_graph() {
    let a = agg(&[doc(Some("useful"), &["A"], "q", "", "2026-01-01")]);
    assert!(a.by_community.is_empty());
}

#[test]
fn doc_community_tie_breaks_to_smallest_label() {
    let nc = community(&[("x", "Zeta"), ("y", "Alpha")]);
    let a1 = aggregate_lessons(
        &[doc(Some("useful"), &["x", "y"], "q", "", "2026-01-01")],
        Some(&nc),
        now(),
        30.0,
        2,
        None,
    );
    assert!(a1.by_community.contains_key("Alpha"));
    assert!(!a1.by_community.contains_key("Zeta"));
}

#[test]
fn community_grouping_uses_plurality_community() {
    let nc = community(&[("A", "Auth"), ("B", "Auth"), ("C", "Cache")]);
    let docs = [
        doc(Some("useful"), &["A", "B", "C"], "q", "", "2026-01-01"),
        doc(Some("dead_end"), &["C"], "q", "", "2026-01-01"),
        doc(Some("useful"), &["Z"], "q", "", "2026-01-01"),
    ];
    let a = aggregate_lessons(&docs, Some(&nc), now(), 30.0, 2, None);
    let keys: HashSet<&str> = a.by_community.keys().map(String::as_str).collect();
    assert_eq!(keys, HashSet::from(["Auth", "Cache", "Uncategorized"]));
    assert_eq!(a.by_community["Auth"].counts.useful, 1);
    assert_eq!(a.by_community["Cache"].counts.dead_end, 1);
    assert_eq!(a.by_community["Uncategorized"].counts.useful, 1);
}

#[test]
fn dead_ends_and_corrections_dedupe_by_question() {
    let docs = [
        doc(Some("dead_end"), &[], "ws server?", "", "2026-01-01"),
        doc(Some("dead_end"), &[], "ws server?", "", "2026-01-02"),
        doc(Some("corrected"), &[], "hash?", "SHA-1", "2026-01-01"),
        doc(Some("corrected"), &[], "hash?", "SHA-256", "2026-01-03"),
    ];
    let a = agg(&docs);
    assert_eq!(
        a.dead_ends
            .iter()
            .map(|d| d.question.as_str())
            .collect::<Vec<_>>(),
        vec!["ws server?"]
    );
    assert_eq!(a.corrections.len(), 1);
    assert_eq!(a.corrections[0].correction, "SHA-256");
}

// --- rendering -----------------------------------------------------------------

#[test]
fn render_has_summary_and_sections() {
    let docs = [
        doc(Some("useful"), &["AuthMiddleware"], "q", "", "2026-01-01"),
        doc(
            Some("dead_end"),
            &["RedisClient"],
            "where is the cache?",
            "",
            "2026-01-01",
        ),
        doc(Some("corrected"), &[], "pw?", "bcrypt", "2026-01-01"),
    ];
    let md = render_lessons_md(&agg(&docs));
    assert!(md.contains("# Lessons"));
    assert!(md.contains("1 useful · 1 dead ends · 1 corrected"));
    assert!(md.contains("`AuthMiddleware`"));
    assert!(md.contains("where is the cache?"));
    assert!(md.contains("bcrypt"));
    assert!(!md.contains("## By topic"));
}

#[test]
fn render_includes_by_topic_when_graph_present() {
    let nc = community(&[("A", "Auth")]);
    let md = render_lessons_md(&aggregate_lessons(
        &[doc(Some("useful"), &["A"], "q", "", "2026-01-01")],
        Some(&nc),
        now(),
        30.0,
        2,
        None,
    ));
    assert!(md.contains("## By topic"));
    assert!(md.contains("### Auth"));
}

#[test]
fn topic_sections_alpha_with_uncategorized_last() {
    let nc = community(&[("a", "Zeta"), ("b", "Alpha")]);
    let docs = [
        doc(Some("useful"), &["a"], "q", "", "2026-01-01"),
        doc(Some("useful"), &["b"], "q", "", "2026-01-01"),
        doc(Some("useful"), &["unknown"], "q", "", "2026-01-01"),
    ];
    let md = render_lessons_md(&aggregate_lessons(&docs, Some(&nc), now(), 30.0, 2, None));
    let headers: Vec<&str> = md.lines().filter_map(|l| l.strip_prefix("### ")).collect();
    assert_eq!(headers, vec!["Alpha", "Zeta", "Uncategorized"]);
}

#[test]
fn render_byte_stable_across_independent_aggregations() {
    let tmp = tempfile::tempdir().unwrap();
    let mem = tmp.path().join("memory");
    write_raw_doc(&mem, "a.md", "2026-01-01", "useful", "first");
    write_raw_doc(&mem, "b.md", "2026-01-02", "dead_end", "dead?");
    let first = render_lessons_md(&aggregate_lessons(
        &load_memory_docs(&mem),
        None,
        now(),
        30.0,
        2,
        None,
    ));
    let second = render_lessons_md(&aggregate_lessons(
        &load_memory_docs(&mem),
        None,
        now(),
        30.0,
        2,
        None,
    ));
    assert_eq!(first, second);
}

#[test]
fn contested_node_renders_once_under_contested() {
    let docs = [
        doc(Some("useful"), &["N"], "q", "", "2026-01-01"),
        doc(Some("dead_end"), &["N"], "bad?", "", "2026-01-01"),
    ];
    let md = render_lessons_md(&agg(&docs));
    assert!(md.contains("**Contested**"));
    let lines: Vec<&str> = md
        .lines()
        .filter(|l| l.starts_with("- `N` —") && l.contains("useful") && l.contains("dead end"))
        .collect();
    assert_eq!(lines.len(), 1);
}

#[test]
fn header_is_cautious() {
    let md = render_lessons_md(&agg(&[doc(Some("useful"), &["A"], "q", "", "2026-01-01")]));
    assert!(md.contains("verify before relying"));
    assert!(!md.contains("reuse what worked"));
}

#[test]
fn lessons_artifact_cannot_be_globbed_back_into_memory() {
    let md = render_lessons_md(&agg(&[doc(Some("useful"), &["A"], "q", "", "2026-01-01")]));
    assert!(parse_memory_doc(&md).is_none());
    let tmp = tempfile::tempdir().unwrap();
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem).unwrap();
    std::fs::write(mem.join("LESSONS.md"), &md).unwrap();
    save_query_result("real", "a", &mem, "query", None, Some("useful"), None).unwrap();
    let docs = load_memory_docs(&mem);
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].question, "real");
}

#[test]
fn render_empty_memory_is_graceful() {
    let md = render_lessons_md(&agg(&[]));
    assert!(md.contains("from 0 session memories"));
    assert!(md.contains("_No marked outcomes yet._"));
}

// --- orchestrator --------------------------------------------------------------

#[test]
fn reflect_writes_lessons_file() {
    let tmp = tempfile::tempdir().unwrap();
    let mem = tmp.path().join("memory");
    save_query_result(
        "q1",
        "a1",
        &mem,
        "query",
        Some(&["A".to_string()]),
        Some("useful"),
        None,
    )
    .unwrap();
    let out = tmp.path().join("reflections").join("LESSONS.md");
    let (out_path, a) = reflect(
        &mem,
        &out,
        graphify_reflect::GraphPaths::default(),
        now(),
        30.0,
        2,
    )
    .unwrap();
    assert!(out_path.exists());
    assert_eq!(a.total, 1);
    assert!(std::fs::read_to_string(&out_path).unwrap().contains("`A`"));
}

#[test]
fn second_session_benefits_from_the_first() {
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("graphify-out");
    let mem = out.join("memory");
    save_query_result(
        "how does auth work?",
        "JWT in middleware",
        &mem,
        "query",
        Some(&["AuthMiddleware".to_string()]),
        Some("useful"),
        None,
    )
    .unwrap();
    save_query_result(
        "where is the cache?",
        "looked at RedisClient, not it",
        &mem,
        "query",
        Some(&["RedisClient".to_string()]),
        Some("dead_end"),
        None,
    )
    .unwrap();
    let lessons = out.join("reflections").join("LESSONS.md");
    reflect(
        &mem,
        &lessons,
        graphify_reflect::GraphPaths::default(),
        now(),
        30.0,
        2,
    )
    .unwrap();
    let body = std::fs::read_to_string(&lessons).unwrap();
    assert!(body.contains("`AuthMiddleware`"));
    assert!(body.contains("where is the cache?"));
}

// --- lessons_fresh -------------------------------------------------------------

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[test]
fn lessons_fresh_missing_output_is_not_fresh() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem)?;
    std::fs::write(mem.join("q.md"), "x")?;
    assert!(!lessons_fresh(
        &tmp.path().join("LESSONS.md"),
        &mem,
        GraphPaths::default()
    ));
    Ok(())
}

#[test]
fn lessons_fresh_true_when_output_newer_than_inputs() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem)?;
    let doc = mem.join("q.md");
    std::fs::write(&doc, "x")?;
    let out = tmp.path().join("LESSONS.md");
    std::fs::write(&out, "y")?;
    set_mtime(&doc, 1000);
    set_mtime(&out, 2000);
    assert!(lessons_fresh(&out, &mem, GraphPaths::default()));
    Ok(())
}

#[test]
fn lessons_fresh_false_when_memory_newer() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem)?;
    let doc = mem.join("q.md");
    std::fs::write(&doc, "x")?;
    let out = tmp.path().join("LESSONS.md");
    std::fs::write(&out, "y")?;
    set_mtime(&out, 1000);
    set_mtime(&doc, 2000);
    assert!(!lessons_fresh(&out, &mem, GraphPaths::default()));
    Ok(())
}

#[test]
fn lessons_fresh_false_when_graph_newer() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem)?;
    let doc = mem.join("q.md");
    std::fs::write(&doc, "x")?;
    let out = tmp.path().join("LESSONS.md");
    std::fs::write(&out, "y")?;
    let graph = tmp.path().join("graph.json");
    std::fs::write(&graph, "{}")?;
    set_mtime(&doc, 1000);
    set_mtime(&out, 1500);
    set_mtime(&graph, 2000);
    assert!(!lessons_fresh(
        &out,
        &mem,
        GraphPaths {
            graph: Some(&graph),
            ..Default::default()
        }
    ));
    Ok(())
}

/// A graph sidecar (`.graphify_analysis.json` / `.graphify_labels.json`) newer
/// than the output makes lessons stale even when graph.json itself is older
/// (#1470). Exercises BOTH sidecars (mirrors the Python parametrized test).
fn assert_stale_when_sidecar_newer(sidecar_name: &str) -> TestResult {
    let tmp = tempfile::tempdir()?;
    let mem = tmp.path().join("memory");
    std::fs::create_dir_all(&mem)?;
    let doc = mem.join("q.md");
    std::fs::write(&doc, "x")?;
    let out = tmp.path().join("LESSONS.md");
    std::fs::write(&out, "y")?;
    let graph = tmp.path().join("graph.json");
    std::fs::write(&graph, "{}")?;
    let sidecar = tmp.path().join(sidecar_name);
    std::fs::write(&sidecar, "{}")?;
    set_mtime(&doc, 1000);
    set_mtime(&graph, 1000);
    set_mtime(&out, 1500);
    // The sidecar (resolved as graph's sibling) is newer than the output.
    set_mtime(&sidecar, 2000);
    assert!(!lessons_fresh(
        &out,
        &mem,
        GraphPaths {
            graph: Some(&graph),
            ..Default::default()
        }
    ));
    Ok(())
}

#[test]
fn lessons_fresh_false_when_analysis_newer() -> TestResult {
    assert_stale_when_sidecar_newer(".graphify_analysis.json")
}

#[test]
fn lessons_fresh_false_when_labels_newer() -> TestResult {
    assert_stale_when_sidecar_newer(".graphify_labels.json")
}

/// Set a file's mtime to `secs` after the Unix epoch.
fn set_mtime(path: &std::path::Path, secs: u64) {
    let mtime = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs);
    filetime_set(path, mtime);
}

fn filetime_set(path: &std::path::Path, mtime: std::time::SystemTime) {
    // Re-open and set times via a small platform-agnostic shim: write then use
    // `File::set_modified` (stable since 1.75).
    let f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.set_modified(mtime).unwrap();
}
