//! Parity tests against `graphify-py/tests/test_build.py`.
//!
//! Tests that depend on `graphify-extract` or `graphify-export` are deferred
//! to those crates' parity suites — the build crate only tests the
//! pipeline-independent behaviour of `build_from_json`, `build`,
//! `edge_data`, `edge_datas`, `deduplicate_by_label`, `prefix_graph_for_global`,
//! and `prune_repo_from_graph`.
#![allow(clippy::expect_used)]

use graphify_build::{
    Graph, GraphKind, build, build_from_json, build_merge, build_merge_with_graph_cap,
    deduplicate_by_label, norm_label, prefix_graph_for_global, prune_repo_from_graph,
};
use serde_json::{Value, json};

fn node_labels(g: &Graph) -> std::collections::HashSet<String> {
    g.nodes()
        .filter_map(|(_, attrs)| {
            attrs
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect()
}

fn small_extraction() -> Value {
    json!({
        "nodes": [
            {"id": "n_transformer", "label": "Transformer", "file_type": "code", "source_file": "tx.py"},
            {"id": "n_attention", "label": "Attention", "file_type": "code", "source_file": "att.py"},
            {"id": "n_layernorm", "label": "LayerNorm", "file_type": "code", "source_file": "ln.py"},
            {"id": "n_concept_attn", "label": "self-attention", "file_type": "concept", "source_file": "att.py"},
        ],
        "edges": [
            {"source": "n_transformer", "target": "n_attention", "relation": "uses",
             "confidence": "EXTRACTED", "source_file": "tx.py"},
            {"source": "n_transformer", "target": "n_layernorm", "relation": "uses",
             "confidence": "EXTRACTED", "source_file": "tx.py"},
            {"source": "n_attention", "target": "n_concept_attn", "relation": "implements",
             "confidence": "INFERRED", "source_file": "att.py"},
            {"source": "n_layernorm", "target": "n_concept_attn", "relation": "stabilizes",
             "confidence": "AMBIGUOUS", "source_file": "ln.py"},
        ],
    })
}

#[test]
fn build_from_json_node_count() {
    let g = build_from_json(small_extraction(), false, None).expect("build");
    assert_eq!(g.node_count(), 4);
}

#[test]
fn build_from_json_edge_count() {
    let g = build_from_json(small_extraction(), false, None).expect("build");
    assert_eq!(g.edge_count(), 4);
}

#[test]
fn nodes_have_label() {
    let g = build_from_json(small_extraction(), false, None).expect("build");
    let attrs = g.node_data("n_transformer").expect("node");
    assert_eq!(
        attrs.get("label").and_then(Value::as_str),
        Some("Transformer")
    );
}

#[test]
fn edges_have_confidence() {
    let g = build_from_json(small_extraction(), false, None).expect("build");
    let attrs = g.edge_data("n_attention", "n_concept_attn").expect("edge");
    assert_eq!(
        attrs.get("confidence").and_then(Value::as_str),
        Some("INFERRED")
    );
}

#[test]
fn ambiguous_edge_preserved() {
    let g = build_from_json(small_extraction(), false, None).expect("build");
    let attrs = g.edge_data("n_layernorm", "n_concept_attn").expect("edge");
    assert_eq!(
        attrs.get("confidence").and_then(Value::as_str),
        Some("AMBIGUOUS")
    );
}

#[test]
fn legacy_node_source_canonicalized() {
    let ext = json!({
        "nodes": [{"id": "n1", "label": "A", "file_type": "code", "source": "a.py"}],
        "edges": [],
        "input_tokens": 0, "output_tokens": 0,
    });
    let g = build_from_json(ext, false, None).expect("build");
    let attrs = g.node_data("n1").expect("node");
    assert_eq!(
        attrs.get("source_file").and_then(Value::as_str),
        Some("a.py")
    );
    assert!(!attrs.contains_key("source"));
}

#[test]
fn legacy_edge_from_to_canonicalized() {
    let ext = json!({
        "nodes": [
            {"id": "n1", "label": "A", "file_type": "code", "source_file": "a.py"},
            {"id": "n2", "label": "B", "file_type": "code", "source_file": "b.py"},
        ],
        "edges": [{"from": "n1", "to": "n2", "relation": "calls",
                   "confidence": "EXTRACTED", "source_file": "a.py", "weight": 1.0}],
        "input_tokens": 0, "output_tokens": 0,
    });
    let g = build_from_json(ext, false, None).expect("build");
    assert_eq!(g.edge_count(), 1);
}

#[test]
fn source_file_backslash_normalized() {
    let ext = json!({
        "nodes": [
            {"id": "n1", "label": "A", "file_type": "code", "source_file": "src\\middleware\\auth.py"},
            {"id": "n2", "label": "B", "file_type": "code", "source_file": "src/middleware/auth.py"},
        ],
        "edges": [],
        "input_tokens": 0, "output_tokens": 0,
    });
    let g = build_from_json(ext, false, None).expect("build");
    let sources: std::collections::BTreeSet<String> = g
        .nodes()
        .filter_map(|(_, attrs)| {
            attrs
                .get("source_file")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    assert_eq!(
        sources,
        std::collections::BTreeSet::from(["src/middleware/auth.py".to_string()])
    );
}

#[test]
fn build_merges_multiple_extractions() {
    let ext1 = json!({
        "nodes": [{"id": "n1", "label": "A", "file_type": "code", "source_file": "a.py"}],
        "edges": [], "input_tokens": 0, "output_tokens": 0,
    });
    let ext2 = json!({
        "nodes": [{"id": "n2", "label": "B", "file_type": "document", "source_file": "b.md"}],
        "edges": [{"source": "n1", "target": "n2", "relation": "references",
                   "confidence": "INFERRED", "source_file": "b.md", "weight": 1.0}],
        "input_tokens": 0, "output_tokens": 0,
    });
    let g = build(&[ext1, ext2], false, true, None).expect("build");
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.edge_count(), 1);
}

#[test]
fn none_file_type_defaults_to_concept() {
    let ext = json!({
        "nodes": [
            {"id": "n1", "label": "Stub", "file_type": null, "source_file": "a.py"},
            {"id": "n2", "label": "Real", "file_type": "code", "source_file": "b.py"},
        ],
        "edges": [], "input_tokens": 0, "output_tokens": 0,
    });
    let g = build_from_json(ext, false, None).expect("build");
    assert_eq!(
        g.node_data("n1")
            .and_then(|a| a.get("file_type"))
            .and_then(Value::as_str),
        Some("concept")
    );
    assert_eq!(
        g.node_data("n2")
            .and_then(|a| a.get("file_type"))
            .and_then(Value::as_str),
        Some("code")
    );
}

#[test]
fn missing_file_type_defaults_to_concept() {
    let ext = json!({
        "nodes": [{"id": "n1", "label": "Bare", "source_file": "a.py"}],
        "edges": [], "input_tokens": 0, "output_tokens": 0,
    });
    let g = build_from_json(ext, false, None).expect("build");
    assert_eq!(
        g.node_data("n1")
            .and_then(|a| a.get("file_type"))
            .and_then(Value::as_str),
        Some("concept")
    );
}

#[test]
fn real_invalid_file_type_coerced_to_concept() {
    let ext = json!({
        "nodes": [{"id": "n1", "label": "Bad", "file_type": "weird_type", "source_file": "a.py"}],
        "edges": [], "input_tokens": 0, "output_tokens": 0,
    });
    let g = build_from_json(ext, false, None).expect("build");
    assert_eq!(
        g.node_data("n1")
            .and_then(|a| a.get("file_type"))
            .and_then(Value::as_str),
        Some("concept")
    );
}

#[test]
fn file_type_synonym_mapping() {
    let ext = json!({
        "nodes": [
            {"id": "n1", "label": "MD", "file_type": "markdown", "source_file": "a.md"},
            {"id": "n2", "label": "Tool", "file_type": "tool", "source_file": "b.py"},
            {"id": "n3", "label": "Pat", "file_type": "pattern", "source_file": "c.md"},
        ],
        "edges": [], "input_tokens": 0, "output_tokens": 0,
    });
    let g = build_from_json(ext, false, None).expect("build");
    assert_eq!(
        g.node_data("n1")
            .and_then(|a| a.get("file_type"))
            .and_then(Value::as_str),
        Some("document")
    );
    assert_eq!(
        g.node_data("n2")
            .and_then(|a| a.get("file_type"))
            .and_then(Value::as_str),
        Some("code")
    );
    assert_eq!(
        g.node_data("n3")
            .and_then(|a| a.get("file_type"))
            .and_then(Value::as_str),
        Some("concept")
    );
}

#[test]
fn edge_data_simple_graph() {
    let mut g = Graph::new(GraphKind::Graph);
    let mut attrs = indexmap::IndexMap::new();
    attrs.insert("relation".to_string(), json!("calls"));
    attrs.insert("confidence".to_string(), json!("EXTRACTED"));
    g.add_node("a", indexmap::IndexMap::new());
    g.add_node("b", indexmap::IndexMap::new());
    g.add_edge("a", "b", attrs);
    let d = g.edge_data("a", "b").expect("edge");
    assert_eq!(d.get("relation").and_then(Value::as_str), Some("calls"));
}

#[test]
fn edge_datas_simple_graph_returns_singleton() {
    let mut g = Graph::new(GraphKind::Graph);
    let mut attrs = indexmap::IndexMap::new();
    attrs.insert("relation".to_string(), json!("calls"));
    g.add_node("a", indexmap::IndexMap::new());
    g.add_node("b", indexmap::IndexMap::new());
    g.add_edge("a", "b", attrs);
    let ds = g.edge_datas("a", "b");
    assert_eq!(ds.len(), 1);
}

#[test]
fn edge_data_multigraph_with_parallel_edges() {
    let mut g = Graph::new(GraphKind::MultiGraph);
    g.add_node("a", indexmap::IndexMap::new());
    g.add_node("b", indexmap::IndexMap::new());
    let mut a1 = indexmap::IndexMap::new();
    a1.insert("relation".to_string(), json!("calls"));
    g.add_edge("a", "b", a1);
    let mut a2 = indexmap::IndexMap::new();
    a2.insert("relation".to_string(), json!("references"));
    g.add_edge("a", "b", a2);
    let ds = g.edge_datas("a", "b");
    assert_eq!(ds.len(), 2);
    let relations: std::collections::BTreeSet<&str> = ds
        .iter()
        .filter_map(|d| d.get("relation").and_then(Value::as_str))
        .collect();
    assert_eq!(
        relations,
        std::collections::BTreeSet::from(["calls", "references"])
    );
}

#[test]
fn build_from_json_relativizes_absolute_source_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let tmp_canon = tmp.path().canonicalize().expect("canonicalize tmp");
    let root = tmp_canon.join("myproject");
    std::fs::create_dir(&root).expect("mkdir");
    let abs = root.join("docs").join("overview.md");
    let abs_str = abs.to_string_lossy().into_owned();
    let ext = json!({
        "nodes": [{"id": "overview_intro", "label": "Intro", "source_file": abs_str,
                   "file_type": "document"}],
        "edges": [{"source": "overview_intro", "target": "overview_intro",
                   "relation": "self", "confidence": "EXTRACTED",
                   "source_file": abs_str}],
    });
    let g = build_from_json(ext, false, Some(&root)).expect("build");
    let sf = g
        .node_data("overview_intro")
        .and_then(|a| a.get("source_file"))
        .and_then(Value::as_str)
        .expect("sf");
    assert!(!sf.starts_with('/'), "still absolute: {sf}");
    assert_eq!(sf, "docs/overview.md");
}

#[test]
fn build_relativizes_absolute_source_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let tmp_canon = tmp.path().canonicalize().expect("canonicalize tmp");
    let root = tmp_canon.join("proj");
    std::fs::create_dir(&root).expect("mkdir");
    let abs = root.join("src").join("main.py");
    let abs_str = abs.to_string_lossy().into_owned();
    let ext = json!({
        "nodes": [{"id": "main_fn", "label": "main", "source_file": abs_str,
                   "file_type": "code"}],
        "edges": [],
    });
    let g = build(&[ext], false, true, Some(&root)).expect("build");
    let sf = g
        .node_data("main_fn")
        .and_then(|a| a.get("source_file"))
        .and_then(Value::as_str)
        .expect("sf");
    assert_eq!(sf, "src/main.py");
}

#[test]
fn build_from_json_relative_source_file_unchanged() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ext = json!({
        "nodes": [{"id": "foo_bar", "label": "bar", "source_file": "src/foo.py",
                   "file_type": "code"}],
        "edges": [],
    });
    let g = build_from_json(ext, false, Some(tmp.path())).expect("build");
    assert_eq!(
        g.node_data("foo_bar")
            .and_then(|a| a.get("source_file"))
            .and_then(Value::as_str),
        Some("src/foo.py")
    );
}

#[test]
fn build_merge_preserves_call_edge_direction() {
    // #760: build_merge must read source/target verbatim, not re-derive edge
    // endpoints from node insertion order (which flips directional `calls`).
    let tmp = tempfile::tempdir().expect("tempdir");
    let graph_path = tmp.path().join("graph.json");
    // Callee `b` is inserted before caller `a`; the edge is a -> b.
    let graph_json = json!({
        "nodes": [
            {"id": "b", "label": "b()", "file_type": "code", "source_file": "x.js"},
            {"id": "a", "label": "a()", "file_type": "code", "source_file": "x.js"},
        ],
        "links": [
            {"source": "a", "target": "b", "relation": "calls", "confidence": "EXTRACTED",
             "source_file": "x.js", "weight": 1.0},
        ],
    });
    std::fs::write(
        &graph_path,
        serde_json::to_string(&graph_json).expect("ser"),
    )
    .expect("write");

    let g = build_merge(&[], &graph_path, None, false, false, None).expect("build_merge");
    let calls: Vec<_> = g
        .edges()
        .filter(|e| e.attrs.get("relation").and_then(Value::as_str) == Some("calls"))
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].attrs.get("_src").and_then(Value::as_str),
        Some("a")
    );
    assert_eq!(
        calls[0].attrs.get("_tgt").and_then(Value::as_str),
        Some("b")
    );
}

#[test]
fn build_from_json_preserves_first_direction_on_bidirectional_pair() {
    // Regression for #1061. When an extraction emits two `calls` edges between
    // the same pair in opposite directions, the undirected graph collapses them
    // into one edge. The deterministic (source, target, relation) sort means the
    // lexicographically-later direction wrote second and clobbered the first
    // edge's _src/_tgt — the surviving edge then exported with caller and callee
    // systematically swapped. First-seen direction (a_handler -> z_emitter) must
    // win instead.
    let extraction = json!({
        "nodes": [
            {"id": "a_handler", "label": "a", "file_type": "code", "source_file": "a.ts"},
            {"id": "z_emitter", "label": "z", "file_type": "code", "source_file": "z.ts"},
        ],
        "edges": [
            {"source": "a_handler", "target": "z_emitter", "relation": "calls",
             "confidence": "EXTRACTED", "source_file": "a.ts"},
            {"source": "z_emitter", "target": "a_handler", "relation": "calls",
             "confidence": "EXTRACTED", "source_file": "z.ts"},
        ],
        "input_tokens": 0,
        "output_tokens": 0,
    });
    let g = build_from_json(extraction, false, None)
        .expect("build_from_json should succeed for a valid bidirectional extraction");

    // Only one undirected edge survives, but its stored direction must be the
    // first-seen one (a_handler -> z_emitter).
    let calls: Vec<_> = g
        .edges()
        .filter(|e| e.attrs.get("relation").and_then(Value::as_str) == Some("calls"))
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "bidirectional pair must collapse to one edge"
    );
    let data = g
        .edge_data("a_handler", "z_emitter")
        .expect("edge between the pair");
    assert_eq!(
        data.get("_src").and_then(Value::as_str),
        Some("a_handler"),
        "calls edge source flipped on bidirectional collision"
    );
    assert_eq!(
        data.get("_tgt").and_then(Value::as_str),
        Some("z_emitter"),
        "calls edge target flipped on bidirectional collision"
    );
}

#[test]
fn build_merge_prune_absolute_paths_match_relative_nodes() {
    // #1007: manifest stores absolute paths, graph nodes store relative paths.
    // prune_sources with absolute paths must still remove the right nodes/edges.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canon").join("corpus");
    std::fs::create_dir(&root).expect("mkdir");
    let graph_path = tmp.path().join("graph.json");
    let graph_json = json!({
        "nodes": [
            {"id": "n1", "label": "login", "file_type": "code", "source_file": "module_a/auth.py"},
            {"id": "n2", "label": "format_date", "file_type": "code", "source_file": "module_b/utils.py"},
        ],
        "edges": [
            {"source": "n1", "target": "n2", "relation": "calls", "confidence": "EXTRACTED",
             "source_file": "module_b/utils.py", "weight": 1.0},
        ],
    });
    std::fs::write(
        &graph_path,
        serde_json::to_string(&graph_json).expect("ser"),
    )
    .expect("write");

    let deleted_abs = root
        .join("module_b")
        .join("utils.py")
        .to_string_lossy()
        .into_owned();
    let g = build_merge(
        &[],
        &graph_path,
        Some(&[deleted_abs]),
        false,
        false,
        Some(&root),
    )
    .expect("build_merge");

    let labels = node_labels(&g);
    assert!(
        !labels.contains("format_date"),
        "stale node should be pruned"
    );
    assert!(labels.contains("login"), "unrelated node must survive");
    assert_eq!(
        g.edge_count(),
        0,
        "edge from deleted source_file should be pruned"
    );
}

#[test]
fn build_merge_prune_windows_backslash_paths() {
    // #1007: prune_sources with Windows-style backslash absolute paths must match.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canon").join("corpus");
    std::fs::create_dir(&root).expect("mkdir");
    let graph_path = tmp.path().join("graph.json");
    let graph_json = json!({
        "nodes": [
            {"id": "n1", "label": "parse_date", "file_type": "code", "source_file": "module_b/utils.py"},
        ],
        "edges": [],
    });
    std::fs::write(
        &graph_path,
        serde_json::to_string(&graph_json).expect("ser"),
    )
    .expect("write");

    let win_path = root
        .join("module_b")
        .join("utils.py")
        .to_string_lossy()
        .replace('/', "\\");
    let g = build_merge(
        &[],
        &graph_path,
        Some(&[win_path]),
        false,
        false,
        Some(&root),
    )
    .expect("build_merge");

    let labels = node_labels(&g);
    assert!(
        !labels.contains("parse_date"),
        "node should be pruned even with a backslash path"
    );
}

#[test]
fn build_merge_rejects_oversized_existing_graph() {
    // #F4: build_merge must refuse to read an existing graph.json over the size
    // cap rather than parsing it into memory.
    let tmp = tempfile::tempdir().expect("tempdir");
    let graph_path = tmp.path().join("graph.json");
    std::fs::write(
        &graph_path,
        serde_json::to_string(&json!({"nodes": [], "links": []})).expect("ser"),
    )
    .expect("write");
    let err = build_merge_with_graph_cap(&[], &graph_path, None, false, false, None, 8)
        .expect_err("should reject oversized graph");
    assert!(err.to_string().contains("exceeds"), "got: {err}");
}

#[test]
fn deduplicate_by_label_collapses_identical_labels() {
    let nodes = vec![
        json!({"id": "n1", "label": "Foo Bar"}),
        json!({"id": "n1_c1", "label": "Foo Bar"}),
        json!({"id": "n2", "label": "Other"}),
    ];
    let edges = vec![json!({"source": "n1_c1", "target": "n2"})];
    let (deduped_nodes, deduped_edges) = deduplicate_by_label(&nodes, &edges);
    assert_eq!(deduped_nodes.len(), 2);
    let ids: std::collections::BTreeSet<&str> = deduped_nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str))
        .collect();
    assert!(ids.contains("n1"));
    assert!(ids.contains("n2"));
    assert_eq!(deduped_edges.len(), 1);
    assert_eq!(
        deduped_edges[0].get("source").and_then(Value::as_str),
        Some("n1")
    );
}

#[test]
fn prefix_graph_for_global_relabels_and_tags() {
    let ext = json!({
        "nodes": [
            {"id": "a", "label": "A", "file_type": "code", "source_file": "a.py"},
            {"id": "b", "label": "B", "file_type": "code", "source_file": "b.py"},
        ],
        "edges": [{"source": "a", "target": "b", "relation": "calls",
                   "confidence": "EXTRACTED", "source_file": "a.py"}],
    });
    let g = build_from_json(ext, false, None).expect("build");
    let h = prefix_graph_for_global(&g, "myrepo");
    assert!(h.contains_node("myrepo::a"));
    assert!(h.contains_node("myrepo::b"));
    let attrs = h.node_data("myrepo::a").expect("node");
    assert_eq!(attrs.get("repo").and_then(Value::as_str), Some("myrepo"));
    assert_eq!(attrs.get("local_id").and_then(Value::as_str), Some("a"));
}

#[test]
fn prune_repo_from_graph_removes_tagged_nodes() {
    let ext = json!({
        "nodes": [
            {"id": "a", "label": "A", "file_type": "code", "source_file": "a.py"},
            {"id": "b", "label": "B", "file_type": "code", "source_file": "b.py"},
        ],
        "edges": [],
    });
    let g = build_from_json(ext, false, None).expect("build");
    let mut h = prefix_graph_for_global(&g, "tag1");
    let n = prune_repo_from_graph(&mut h, "tag1");
    assert_eq!(n, 2);
    assert_eq!(h.node_count(), 0);
}

// ---------------------------------------------------------------------------
// norm_label: NFKC + Unicode-aware (#937)
// ---------------------------------------------------------------------------

#[test]
fn norm_label_preserves_cjk() {
    assert_eq!(norm_label("認証"), "認証");
    assert_eq!(norm_label("身份验证 API"), "身份验证 api");
}

#[test]
fn norm_label_collapses_punctuation_to_spaces() {
    assert_eq!(norm_label("foo--bar__baz"), "foo bar baz");
}

#[test]
fn norm_label_nfkc_normalizes_fullwidth() {
    assert_eq!(norm_label("ＡＢＣ"), "abc");
}

// ---------------------------------------------------------------------------
// build_from_json: drop cross-language INFERRED `calls` edges (#993, #991)
// ---------------------------------------------------------------------------

#[test]
fn build_drops_cross_language_inferred_calls_edge() {
    let ext = json!({
        "nodes": [
            {"id": "py_parse", "label": "parse", "file_type": "code", "source_file": "src/lib.py"},
            {"id": "rs_parse", "label": "parse", "file_type": "code", "source_file": "src/lib.rs"},
        ],
        "edges": [
            {"source": "py_parse", "target": "rs_parse", "relation": "calls",
             "confidence": "INFERRED", "source_file": "src/lib.py"},
        ],
    });
    let g = build_from_json(ext, true, None).expect("build");
    assert_eq!(g.edge_count(), 0);
}

#[test]
fn build_keeps_same_language_inferred_calls_edge() {
    let ext = json!({
        "nodes": [
            {"id": "p1", "label": "parse", "file_type": "code", "source_file": "src/a.py"},
            {"id": "p2", "label": "parse_inner", "file_type": "code", "source_file": "src/b.py"},
        ],
        "edges": [
            {"source": "p1", "target": "p2", "relation": "calls",
             "confidence": "INFERRED", "source_file": "src/a.py"},
        ],
    });
    let g = build_from_json(ext, true, None).expect("build");
    assert_eq!(g.edge_count(), 1);
}

#[test]
fn build_keeps_extracted_cross_language_edges() {
    // The cross-language filter only applies to INFERRED edges. EXTRACTED
    // edges (from real tree-sitter parse evidence) survive.
    let ext = json!({
        "nodes": [
            {"id": "py", "label": "foo", "file_type": "code", "source_file": "src/a.py"},
            {"id": "rs", "label": "foo", "file_type": "code", "source_file": "src/a.rs"},
        ],
        "edges": [
            {"source": "py", "target": "rs", "relation": "calls",
             "confidence": "EXTRACTED", "source_file": "src/a.py"},
        ],
    });
    let g = build_from_json(ext, true, None).expect("build");
    assert_eq!(g.edge_count(), 1);
}

#[test]
fn build_keeps_inferred_uses_edge_across_languages() {
    // The filter is scoped to `calls` only — other relations (`uses`,
    // `implements`, etc.) are not dropped across language boundaries.
    let ext = json!({
        "nodes": [
            {"id": "py", "label": "Foo", "file_type": "code", "source_file": "src/a.py"},
            {"id": "rs", "label": "Foo", "file_type": "code", "source_file": "src/a.rs"},
        ],
        "edges": [
            {"source": "py", "target": "rs", "relation": "uses",
             "confidence": "INFERRED", "source_file": "src/a.py"},
        ],
    });
    let g = build_from_json(ext, true, None).expect("build");
    assert_eq!(g.edge_count(), 1);
}

#[test]
fn build_drops_inferred_calls_when_one_side_has_unknown_extension() {
    // Mirrors Python `_LANG_FAMILY.get(src_ext) != _LANG_FAMILY.get(tgt_ext)`:
    // an unknown extension maps to `None`, and `Some("py") != None` is true,
    // so the INFERRED `calls` edge is dropped. Document this explicitly so
    // a future refactor doesn't accidentally start treating unknown
    // extensions as "matches everything" (which would diverge from
    // Python's behaviour).
    let ext = json!({
        "nodes": [
            {"id": "py", "label": "parse", "file_type": "code", "source_file": "src/a.py"},
            {"id": "unk", "label": "parse", "file_type": "code", "source_file": "src/b.unknown"},
        ],
        "edges": [
            {"source": "py", "target": "unk", "relation": "calls",
             "confidence": "INFERRED", "source_file": "src/a.py"},
        ],
    });
    let g = build_from_json(ext, true, None).expect("build");
    assert_eq!(g.edge_count(), 0);
}
