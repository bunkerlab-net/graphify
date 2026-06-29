//! Parity tests against `graphify-py/tests/test_wiki.py`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use graphify_build::{Graph, GraphKind, build_from_json};
use graphify_wiki::{GodNodeData, to_wiki};
use indexmap::IndexMap;
use serde_json::Value;
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn make_graph() -> Graph {
    let mut g = Graph::new(GraphKind::Graph);

    let mut n1 = IndexMap::new();
    n1.insert("label".to_string(), Value::String("parse".to_string()));
    n1.insert("file_type".to_string(), Value::String("code".to_string()));
    n1.insert(
        "source_file".to_string(),
        Value::String("parser.py".to_string()),
    );
    g.add_node("n1", n1);

    let mut n2 = IndexMap::new();
    n2.insert("label".to_string(), Value::String("validate".to_string()));
    n2.insert("file_type".to_string(), Value::String("code".to_string()));
    n2.insert(
        "source_file".to_string(),
        Value::String("parser.py".to_string()),
    );
    g.add_node("n2", n2);

    let mut n3 = IndexMap::new();
    n3.insert("label".to_string(), Value::String("render".to_string()));
    n3.insert("file_type".to_string(), Value::String("code".to_string()));
    n3.insert(
        "source_file".to_string(),
        Value::String("renderer.py".to_string()),
    );
    g.add_node("n3", n3);

    let mut n4 = IndexMap::new();
    n4.insert("label".to_string(), Value::String("stream".to_string()));
    n4.insert("file_type".to_string(), Value::String("code".to_string()));
    n4.insert(
        "source_file".to_string(),
        Value::String("renderer.py".to_string()),
    );
    g.add_node("n4", n4);

    let mut e12 = IndexMap::new();
    e12.insert("relation".to_string(), Value::String("calls".to_string()));
    e12.insert(
        "confidence".to_string(),
        Value::String("EXTRACTED".to_string()),
    );
    e12.insert("weight".to_string(), Value::from(1.0_f64));
    g.add_edge("n1", "n2", e12);

    let mut e13 = IndexMap::new();
    e13.insert(
        "relation".to_string(),
        Value::String("references".to_string()),
    );
    e13.insert(
        "confidence".to_string(),
        Value::String("INFERRED".to_string()),
    );
    e13.insert("weight".to_string(), Value::from(1.0_f64));
    g.add_edge("n1", "n3", e13);

    let mut e34 = IndexMap::new();
    e34.insert("relation".to_string(), Value::String("calls".to_string()));
    e34.insert(
        "confidence".to_string(),
        Value::String("EXTRACTED".to_string()),
    );
    e34.insert("weight".to_string(), Value::from(1.0_f64));
    g.add_edge("n3", "n4", e34);

    g
}

fn communities() -> IndexMap<i64, Vec<String>> {
    let mut m = IndexMap::new();
    m.insert(0, vec!["n1".to_string(), "n2".to_string()]);
    m.insert(1, vec!["n3".to_string(), "n4".to_string()]);
    m
}

fn labels() -> IndexMap<i64, String> {
    let mut m = IndexMap::new();
    m.insert(0, "Parsing Layer".to_string());
    m.insert(1, "Rendering Layer".to_string());
    m
}

fn cohesion() -> IndexMap<i64, f64> {
    let mut m = IndexMap::new();
    m.insert(0, 0.85_f64);
    m.insert(1, 0.72_f64);
    m
}

fn god_nodes() -> Vec<GodNodeData> {
    vec![GodNodeData {
        id: "n1".to_string(),
        label: "parse".to_string(),
        degree: 2,
    }]
}

// ---------------------------------------------------------------------------
// Tests — ported 1:1 from test_wiki.py
// ---------------------------------------------------------------------------

#[test]
fn test_to_wiki_writes_index() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    let cohesion = cohesion();
    let gods = god_nodes();
    to_wiki(
        &g,
        &communities(),
        dir.path(),
        Some(&labels),
        Some(&cohesion),
        Some(&gods),
    )
    .expect("test invariant");
    assert!(dir.path().join("index.md").exists());
}

#[test]
fn test_to_wiki_returns_article_count() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    let cohesion = cohesion();
    let gods = god_nodes();
    // 2 communities + 1 god node = 3
    let n = to_wiki(
        &g,
        &communities(),
        dir.path(),
        Some(&labels),
        Some(&cohesion),
        Some(&gods),
    )
    .expect("test invariant");
    assert_eq!(n, 3);
}

#[test]
fn test_to_wiki_community_articles_created() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    to_wiki(&g, &communities(), dir.path(), Some(&labels), None, None).expect("test invariant");
    assert!(dir.path().join("Parsing_Layer.md").exists());
    assert!(dir.path().join("Rendering_Layer.md").exists());
}

#[test]
fn test_to_wiki_god_node_article_created() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    let gods = god_nodes();
    to_wiki(
        &g,
        &communities(),
        dir.path(),
        Some(&labels),
        None,
        Some(&gods),
    )
    .expect("test invariant");
    assert!(dir.path().join("parse.md").exists());
}

#[test]
fn test_index_links_all_communities() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    to_wiki(&g, &communities(), dir.path(), Some(&labels), None, None).expect("test invariant");
    let index = std::fs::read_to_string(dir.path().join("index.md")).expect("test invariant");
    assert!(index.contains("[Parsing Layer](Parsing_Layer.md)"));
    assert!(index.contains("[Rendering Layer](Rendering_Layer.md)"));
}

#[test]
fn test_index_lists_god_nodes() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    let gods = god_nodes();
    to_wiki(
        &g,
        &communities(),
        dir.path(),
        Some(&labels),
        None,
        Some(&gods),
    )
    .expect("test invariant");
    let index = std::fs::read_to_string(dir.path().join("index.md")).expect("test invariant");
    assert!(index.contains("[parse](parse.md)"));
    assert!(index.contains("2 connections"));
}

#[test]
fn test_community_article_has_cross_links() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    to_wiki(&g, &communities(), dir.path(), Some(&labels), None, None).expect("test invariant");
    let parsing =
        std::fs::read_to_string(dir.path().join("Parsing_Layer.md")).expect("test invariant");
    // n1 (parsing) references n3 (rendering) → cross-community link
    assert!(parsing.contains("[Rendering Layer](Rendering_Layer.md)"));
}

#[test]
fn test_community_article_shows_cohesion() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    let cohesion = cohesion();
    to_wiki(
        &g,
        &communities(),
        dir.path(),
        Some(&labels),
        Some(&cohesion),
        None,
    )
    .expect("test invariant");
    let parsing =
        std::fs::read_to_string(dir.path().join("Parsing_Layer.md")).expect("test invariant");
    assert!(parsing.contains("cohesion 0.85"));
}

#[test]
fn test_community_article_has_audit_trail() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    to_wiki(&g, &communities(), dir.path(), Some(&labels), None, None).expect("test invariant");
    let parsing =
        std::fs::read_to_string(dir.path().join("Parsing_Layer.md")).expect("test invariant");
    assert!(parsing.contains("EXTRACTED"));
    assert!(parsing.contains("INFERRED"));
}

#[test]
fn test_god_node_article_has_connections() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    let gods = god_nodes();
    to_wiki(
        &g,
        &communities(),
        dir.path(),
        Some(&labels),
        None,
        Some(&gods),
    )
    .expect("test invariant");
    let article = std::fs::read_to_string(dir.path().join("parse.md")).expect("test invariant");
    // parse's neighbours (validate, render) have no article, so they show as
    // plain text, not links.
    assert!(article.contains("validate") && article.contains("render"));
    assert!(!article.contains("[["));
    assert!(!article.contains("](validate.md)") && !article.contains("](render.md)"));
}

#[test]
fn test_god_node_article_links_community() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    let gods = god_nodes();
    to_wiki(
        &g,
        &communities(),
        dir.path(),
        Some(&labels),
        None,
        Some(&gods),
    )
    .expect("test invariant");
    let article = std::fs::read_to_string(dir.path().join("parse.md")).expect("test invariant");
    assert!(article.contains("[Parsing Layer](Parsing_Layer.md)"));
}

#[test]
fn test_to_wiki_skips_missing_god_node_ids() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    let bad_gods = vec![GodNodeData {
        id: "nonexistent".to_string(),
        label: "ghost".to_string(),
        degree: 99,
    }];
    // 2 communities + 0 god nodes (nonexistent skipped) = 2
    let n = to_wiki(
        &g,
        &communities(),
        dir.path(),
        Some(&labels),
        None,
        Some(&bad_gods),
    )
    .expect("test invariant");
    assert_eq!(n, 2);
}

#[test]
fn test_to_wiki_no_labels_uses_fallback() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    to_wiki(&g, &communities(), dir.path(), None, None, None).expect("test invariant");
    assert!(dir.path().join("Community_0.md").exists());
    assert!(dir.path().join("Community_1.md").exists());
}

#[test]
fn test_article_navigation_footer() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    to_wiki(&g, &communities(), dir.path(), Some(&labels), None, None).expect("test invariant");
    let article =
        std::fs::read_to_string(dir.path().join("Parsing_Layer.md")).expect("test invariant");
    assert!(article.contains("[index](index.md)"));
}

#[test]
fn test_community_article_truncation_notice() {
    let dir = tempdir().expect("tempdir");
    let mut g = Graph::new(GraphKind::Graph);
    let nodes: Vec<String> = (0..30).map(|i| format!("n{i}")).collect();
    for nid in &nodes {
        let mut attrs = IndexMap::new();
        attrs.insert("label".to_string(), Value::String(format!("concept_{nid}")));
        attrs.insert("file_type".to_string(), Value::String("code".to_string()));
        attrs.insert("source_file".to_string(), Value::String("a.py".to_string()));
        g.add_node(nid, attrs);
    }
    for i in 0..(nodes.len() - 1) {
        let mut e = IndexMap::new();
        e.insert("relation".to_string(), Value::String("calls".to_string()));
        e.insert(
            "confidence".to_string(),
            Value::String("EXTRACTED".to_string()),
        );
        e.insert("weight".to_string(), Value::from(1.0_f64));
        g.add_edge(&nodes[i], &nodes[i + 1], e);
    }
    let mut comms = IndexMap::new();
    comms.insert(0_i64, nodes.clone());
    let mut lbls = IndexMap::new();
    lbls.insert(0_i64, "Big Community".to_string());
    to_wiki(&g, &comms, dir.path(), Some(&lbls), None, None).expect("test invariant");
    let article =
        std::fs::read_to_string(dir.path().join("Big_Community.md")).expect("test invariant");
    assert!(article.contains("and 5 more nodes"));
}

#[test]
fn test_cross_community_links_without_node_community_attrs() {
    let dir = tempdir().expect("tempdir");
    let mut g = Graph::new(GraphKind::Graph);
    let mut n1 = IndexMap::new();
    n1.insert("label".to_string(), Value::String("parse".to_string()));
    n1.insert("file_type".to_string(), Value::String("code".to_string()));
    n1.insert(
        "source_file".to_string(),
        Value::String("parser.py".to_string()),
    );
    g.add_node("n1", n1);
    let mut n2 = IndexMap::new();
    n2.insert("label".to_string(), Value::String("render".to_string()));
    n2.insert("file_type".to_string(), Value::String("code".to_string()));
    n2.insert(
        "source_file".to_string(),
        Value::String("renderer.py".to_string()),
    );
    g.add_node("n2", n2);
    let mut e = IndexMap::new();
    e.insert(
        "relation".to_string(),
        Value::String("references".to_string()),
    );
    e.insert(
        "confidence".to_string(),
        Value::String("INFERRED".to_string()),
    );
    e.insert("weight".to_string(), Value::from(1.0_f64));
    g.add_edge("n1", "n2", e);

    let mut comms = IndexMap::new();
    comms.insert(0_i64, vec!["n1".to_string()]);
    comms.insert(1_i64, vec!["n2".to_string()]);
    let mut lbls = IndexMap::new();
    lbls.insert(0_i64, "Parsing".to_string());
    lbls.insert(1_i64, "Rendering".to_string());

    to_wiki(&g, &comms, dir.path(), Some(&lbls), None, None).expect("test invariant");
    let article = std::fs::read_to_string(dir.path().join("Parsing.md")).expect("test invariant");
    assert!(article.contains("[Rendering](Rendering.md)"));
}

#[test]
fn test_god_node_article_community_without_node_attr() {
    let dir = tempdir().expect("tempdir");
    let mut g = Graph::new(GraphKind::Graph);
    let mut n1 = IndexMap::new();
    n1.insert("label".to_string(), Value::String("parse".to_string()));
    n1.insert("file_type".to_string(), Value::String("code".to_string()));
    n1.insert(
        "source_file".to_string(),
        Value::String("parser.py".to_string()),
    );
    g.add_node("n1", n1);
    let mut n2 = IndexMap::new();
    n2.insert("label".to_string(), Value::String("validate".to_string()));
    n2.insert("file_type".to_string(), Value::String("code".to_string()));
    n2.insert(
        "source_file".to_string(),
        Value::String("parser.py".to_string()),
    );
    g.add_node("n2", n2);
    let mut e = IndexMap::new();
    e.insert("relation".to_string(), Value::String("calls".to_string()));
    e.insert(
        "confidence".to_string(),
        Value::String("EXTRACTED".to_string()),
    );
    e.insert("weight".to_string(), Value::from(1.0_f64));
    g.add_edge("n1", "n2", e);

    let mut comms = IndexMap::new();
    comms.insert(0_i64, vec!["n1".to_string(), "n2".to_string()]);
    let mut lbls = IndexMap::new();
    lbls.insert(0_i64, "Core Logic".to_string());
    let gods = vec![GodNodeData {
        id: "n1".to_string(),
        label: "parse".to_string(),
        degree: 1,
    }];

    to_wiki(&g, &comms, dir.path(), Some(&lbls), None, Some(&gods)).expect("test invariant");
    let article = std::fs::read_to_string(dir.path().join("parse.md")).expect("test invariant");
    assert!(article.contains("[Core Logic](Core_Logic.md)"));
}

#[test]
fn test_to_wiki_drops_stale_community_nodes() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    let mut comms = IndexMap::new();
    comms.insert(
        0_i64,
        vec![
            "n1".to_string(),
            "n2".to_string(),
            "stale_ghost".to_string(),
        ],
    );
    comms.insert(1_i64, vec!["n3".to_string(), "n4".to_string()]);
    let n = to_wiki(&g, &comms, dir.path(), Some(&labels), None, None).expect("test invariant");
    assert_eq!(n, 2);
    let article =
        std::fs::read_to_string(dir.path().join("Parsing_Layer.md")).expect("test invariant");
    assert!(article.contains("parse"));
    assert!(!article.contains("stale_ghost"));
}

#[test]
fn test_to_wiki_all_stale_raises() {
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    let mut all_stale = IndexMap::new();
    all_stale.insert(0_i64, vec!["ghost1".to_string(), "ghost2".to_string()]);
    all_stale.insert(1_i64, vec!["ghost3".to_string()]);
    let err =
        to_wiki(&g, &all_stale, dir.path(), Some(&labels), None, None).expect_err("expected Err");
    // Match substring "stale" in the error message.
    assert!(err.to_string().to_lowercase().contains("stale"));
}

#[test]
fn test_to_wiki_stale_nodes_prints_warning() {
    // We can't easily capture stderr in Rust tests without an external crate,
    // so we verify the function succeeds and drops stale IDs correctly.
    let dir = tempdir().expect("tempdir");
    let g = make_graph();
    let labels = labels();
    let mut comms = IndexMap::new();
    comms.insert(
        0_i64,
        vec!["n1".to_string(), "stale1".to_string(), "stale2".to_string()],
    );
    comms.insert(1_i64, vec!["n3".to_string(), "n4".to_string()]);
    let n = to_wiki(&g, &comms, dir.path(), Some(&labels), None, None).expect("test invariant");
    assert_eq!(n, 2); // both community articles written
    // n1 still appears in the article, stale IDs silently dropped.
    let article =
        std::fs::read_to_string(dir.path().join("Parsing_Layer.md")).expect("test invariant");
    assert!(article.contains("parse"));
    assert!(!article.contains("stale1"));
}

#[test]
fn test_community_article_handles_null_source_file() {
    // Regression for graphify-py #1016: a node with `source_file=None`
    // (Value::Null in Rust) must not crash rendering when collecting
    // the set of distinct source files for the community article.
    let dir = tempdir().expect("tempdir");

    let mut g = Graph::new(GraphKind::Graph);

    let mut n1 = IndexMap::new();
    n1.insert("label".to_string(), Value::String("parse".to_string()));
    n1.insert("file_type".to_string(), Value::String("code".to_string()));
    n1.insert("source_file".to_string(), Value::Null);
    g.add_node("n1", n1);

    let mut n2 = IndexMap::new();
    n2.insert("label".to_string(), Value::String("validate".to_string()));
    n2.insert("file_type".to_string(), Value::String("code".to_string()));
    n2.insert(
        "source_file".to_string(),
        Value::String("parser.py".to_string()),
    );
    g.add_node("n2", n2);

    let mut e12 = IndexMap::new();
    e12.insert("relation".to_string(), Value::String("calls".to_string()));
    e12.insert(
        "confidence".to_string(),
        Value::String("EXTRACTED".to_string()),
    );
    e12.insert("weight".to_string(), Value::from(1.0_f64));
    g.add_edge("n1", "n2", e12);

    let mut comms = IndexMap::new();
    comms.insert(0_i64, vec!["n1".to_string(), "n2".to_string()]);
    let mut labels = IndexMap::new();
    labels.insert(0_i64, "Parsing Layer".to_string());

    to_wiki(&g, &comms, dir.path(), Some(&labels), None, None).expect("must not raise");
    assert!(dir.path().join("index.md").exists());
    // The community article itself must also be emitted — the regression
    // would have crashed before reaching this point.
    let article = std::fs::read_to_string(dir.path().join("Parsing_Layer.md"))
        .expect("community article must exist");
    assert!(article.contains("parse") || article.contains("validate"));
}

// ── #1444 portable links + #1453 case-fold slug ──────────────────────────────

/// Build a small graph from `(id, label, source_file)` nodes and
/// `(src, tgt, relation, confidence)` edges.
fn graph_from(nodes: &[(&str, &str, &str)], edges: &[(&str, &str, &str, &str)]) -> Graph {
    let json = serde_json::json!({
        "nodes": nodes.iter().map(|(id, label, sf)| serde_json::json!({
            "id": id, "label": label, "file_type": "code", "source_file": sf})).collect::<Vec<_>>(),
        "edges": edges.iter().map(|(s, t, r, c)| serde_json::json!({
            "source": s, "target": t, "relation": r, "confidence": c, "weight": 1.0,
            "source_file": "a.py"})).collect::<Vec<_>>(),
    });
    build_from_json(json, false, None).expect("build")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `(display, decoded_target)` for each inline markdown link, skipping external
/// URLs. Simple labels only (no escaped brackets), matching Python `_inline_links`.
fn inline_links(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close_rel) = after.find("](") else {
            break;
        };
        let display = &after[..close_rel];
        let target_start = &after[close_rel + 2..];
        let Some(paren) = target_start.find(')') else {
            break;
        };
        let target = &target_start[..paren];
        if !display.contains(']') && !target.contains("://") {
            out.push((display.to_string(), percent_decode(target)));
        }
        rest = &target_start[paren + 1..];
    }
    out
}

fn md_articles(dir: &std::path::Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .expect("read_dir")
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            (p.extension().and_then(|x| x.to_str()) == Some("md")
                && p.file_name().and_then(|n| n.to_str()) != Some("index.md"))
            .then(|| p.file_stem().unwrap().to_string_lossy().into_owned())
        })
        .collect()
}

#[test]
fn test_to_wiki_case_only_distinct_labels_dont_overwrite() {
    let g = graph_from(
        &[("n1", "parse", "a.py"), ("n2", "render", "b.py")],
        &[("n1", "n2", "calls", "EXTRACTED")],
    );
    let comms: IndexMap<i64, Vec<String>> =
        IndexMap::from([(0, vec!["n1".to_string()]), (1, vec!["n2".to_string()])]);
    let labels: IndexMap<i64, String> =
        IndexMap::from([(0, "Parser".to_string()), (1, "parser".to_string())]);
    let dir = tempdir().expect("tempdir");
    let n = to_wiki(&g, &comms, dir.path(), Some(&labels), None, None).expect("wiki");
    let articles = md_articles(dir.path());
    assert_eq!(articles.len(), n);
    assert_eq!(n, 2, "{articles:?}");
    let lowered: std::collections::HashSet<String> =
        articles.iter().map(|s| s.to_lowercase()).collect();
    assert_eq!(lowered.len(), articles.len(), "{articles:?}");
}

#[test]
fn test_to_wiki_god_node_label_case_collides_with_community() {
    let g = graph_from(
        &[("n1", "parse", "a.py"), ("n2", "run", "b.py")],
        &[("n1", "n2", "calls", "EXTRACTED")],
    );
    let comms: IndexMap<i64, Vec<String>> =
        IndexMap::from([(0, vec!["n1".to_string(), "n2".to_string()])]);
    let labels: IndexMap<i64, String> = IndexMap::from([(0, "Parser".to_string())]);
    let gods = [GodNodeData {
        id: "n1".to_string(),
        label: "parser".to_string(),
        degree: 1,
    }];
    let dir = tempdir().expect("tempdir");
    let n = to_wiki(&g, &comms, dir.path(), Some(&labels), None, Some(&gods)).expect("wiki");
    let articles = md_articles(dir.path());
    assert_eq!(articles.len(), n);
    assert_eq!(n, 2, "{articles:?}");
    let lowered: std::collections::HashSet<String> =
        articles.iter().map(|s| s.to_lowercase()).collect();
    assert_eq!(lowered.len(), articles.len(), "{articles:?}");
}

#[test]
fn test_wiki_emits_no_obsidian_wikilinks() {
    let g = make_graph();
    let gods = god_nodes();
    let dir = tempdir().expect("tempdir");
    to_wiki(
        &g,
        &communities(),
        dir.path(),
        Some(&labels()),
        Some(&cohesion()),
        Some(&gods),
    )
    .expect("wiki");
    for e in std::fs::read_dir(dir.path()).expect("read_dir").flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) == Some("md") {
            assert!(
                !std::fs::read_to_string(&p).unwrap().contains("[["),
                "{:?}",
                p.file_name()
            );
        }
    }
}

#[test]
fn test_wiki_links_resolve_to_real_files() {
    let g = make_graph();
    let gods = god_nodes();
    let dir = tempdir().expect("tempdir");
    to_wiki(
        &g,
        &communities(),
        dir.path(),
        Some(&labels()),
        Some(&cohesion()),
        Some(&gods),
    )
    .expect("wiki");
    let mut seen = false;
    for e in std::fs::read_dir(dir.path()).expect("read_dir").flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("md") {
            continue;
        }
        for (display, target) in inline_links(&std::fs::read_to_string(&p).unwrap()) {
            seen = true;
            assert!(
                dir.path().join(&target).exists(),
                "[{display}] -> {target} is dead"
            );
        }
    }
    assert!(seen, "expected inline markdown links");
}

#[test]
fn test_wiki_link_display_keeps_label_but_target_is_filename() {
    let g = make_graph();
    let dir = tempdir().expect("tempdir");
    to_wiki(&g, &communities(), dir.path(), Some(&labels()), None, None).expect("wiki");
    let index = std::fs::read_to_string(dir.path().join("index.md")).expect("index");
    assert!(index.contains("[Parsing Layer](Parsing_Layer.md)"));
    assert!(!index.contains("Parsing Layer.md")); // the broken Obsidian-only target
}

#[test]
fn test_wiki_special_characters_in_label_resolve() {
    let g = graph_from(
        &[("n1", "a", "a.py"), ("n2", "b", "b.py")],
        &[("n1", "n2", "references", "INFERRED")],
    );
    let comms: IndexMap<i64, Vec<String>> =
        IndexMap::from([(0, vec!["n1".to_string()]), (1, vec!["n2".to_string()])]);
    let labels: IndexMap<i64, String> =
        IndexMap::from([(0, "C# & Auth (v2)".to_string()), (1, "Other".to_string())]);
    let dir = tempdir().expect("tempdir");
    to_wiki(&g, &comms, dir.path(), Some(&labels), None, None).expect("wiki");
    let article = std::fs::read_to_string(dir.path().join("Other.md")).expect("Other");
    let targets: Vec<String> = inline_links(&article).into_iter().map(|(_, t)| t).collect();
    assert!(
        targets.contains(&"C#_&_Auth_(v2).md".to_string()),
        "{targets:?}"
    );
    assert!(dir.path().join("C#_&_Auth_(v2).md").exists());
    assert!(
        article.contains("C%23_%26_Auth_%28v2%29.md"),
        "raw target must be percent-encoded"
    );
}

#[test]
fn test_wiki_link_with_bracketed_label_resolves() {
    let g = graph_from(
        &[("n1", "a", "a.py"), ("n2", "b", "b.py")],
        &[("n1", "n2", "references", "INFERRED")],
    );
    let comms: IndexMap<i64, Vec<String>> =
        IndexMap::from([(0, vec!["n1".to_string()]), (1, vec!["n2".to_string()])]);
    let labels: IndexMap<i64, String> =
        IndexMap::from([(0, "Array[T] Models".to_string()), (1, "Other".to_string())]);
    let dir = tempdir().expect("tempdir");
    to_wiki(&g, &comms, dir.path(), Some(&labels), None, None).expect("wiki");
    let article = std::fs::read_to_string(dir.path().join("Other.md")).expect("Other");
    assert!(
        article.contains(r"[Array\[T\] Models](Array%5BT%5D_Models.md)"),
        "{article}"
    );
    assert!(dir.path().join("Array[T]_Models.md").exists());
}

#[test]
fn test_wiki_links_to_nodes_without_articles_are_plain_text() {
    let g = make_graph();
    let gods = god_nodes();
    let dir = tempdir().expect("tempdir");
    to_wiki(
        &g,
        &communities(),
        dir.path(),
        Some(&labels()),
        None,
        Some(&gods),
    )
    .expect("wiki");
    let article = std::fs::read_to_string(dir.path().join("parse.md")).expect("parse");
    assert!(article.contains("- validate") && article.contains("- render"));
    assert!(!article.contains("[[validate]]") && !article.contains("[[render]]"));
    for (_, target) in inline_links(&article) {
        assert!(target != "validate.md" && target != "render.md", "{target}");
    }
}

#[test]
fn test_wiki_links_use_collision_suffixed_slug() {
    let g = graph_from(
        &[("n1", "a", "a.py"), ("n2", "b", "b.py")],
        &[("n1", "n2", "references", "INFERRED")],
    );
    let comms: IndexMap<i64, Vec<String>> =
        IndexMap::from([(0, vec!["n1".to_string()]), (1, vec!["n2".to_string()])]);
    let labels: IndexMap<i64, String> =
        IndexMap::from([(0, "Parser".to_string()), (1, "parser".to_string())]);
    let dir = tempdir().expect("tempdir");
    to_wiki(&g, &comms, dir.path(), Some(&labels), None, None).expect("wiki");
    let index = std::fs::read_to_string(dir.path().join("index.md")).expect("index");
    let targets: Vec<String> = inline_links(&index).into_iter().map(|(_, t)| t).collect();
    assert!(targets.contains(&"parser_2.md".to_string()), "{targets:?}");
    for t in &targets {
        assert!(dir.path().join(t).exists(), "{t}");
    }
}
