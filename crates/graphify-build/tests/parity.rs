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
    dedupe_edges, dedupe_nodes, deduplicate_by_label, graph_has_legacy_ids, norm_label,
    prefix_graph_for_global, prune_repo_from_graph,
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
    // #1504 re-keys the old short id ("overview_intro") to its full-path form.
    let sf = g
        .node_data("docs_overview_intro")
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
    // #1504 re-keys the old short id ("main_fn") to its full-path form.
    let sf = g
        .node_data("src_main_fn")
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
    // source_file is untouched; the id is re-keyed to the full-path form (#1504).
    assert_eq!(
        g.node_data("src_foo_bar")
            .and_then(|a| a.get("source_file"))
            .and_then(Value::as_str),
        Some("src/foo.py")
    );
}

#[test]
fn build_from_json_relativizes_hyperedge_source_file() {
    // #1418: hyperedge source_file must be relativized like nodes and edges, so
    // `to_json` — which writes `graph.hyperedges` verbatim and has no root —
    // never leaks an absolute path from a semantic subagent.
    let tmp = tempfile::tempdir().expect("tempdir");
    let base = tmp.path().canonicalize().expect("canonicalize");
    let abs_doc = base.join("docs").join("CLAUDE.md");
    let abs_str = abs_doc.to_string_lossy().into_owned();
    let ext = json!({
        "nodes": [
            {"id": "a", "label": "A", "file_type": "document", "source_file": abs_str.clone()},
        ],
        "edges": [],
        "hyperedges": [
            {"id": "arch", "label": "Architecture", "nodes": ["a"],
             "relation": "participate_in", "confidence": "INFERRED",
             "confidence_score": 0.75, "source_file": abs_str},
        ],
    });
    let g = build_from_json(ext, false, Some(&base)).expect("build");
    let he = g
        .graph_attrs
        .get("hyperedges")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .expect("hyperedge present");
    assert_eq!(
        he.get("source_file").and_then(Value::as_str),
        Some("docs/CLAUDE.md")
    );
    // Anchor: the node path is relativized the same way (the contract this mirrors).
    assert_eq!(
        g.node_data("a")
            .and_then(|a| a.get("source_file"))
            .and_then(Value::as_str),
        Some("docs/CLAUDE.md")
    );
}

#[test]
fn build_from_json_skips_non_hashable_node_id() {
    // A malformed LLM extraction can emit a list-valued id; build_from_json must
    // skip it and still build the graph from the well-formed nodes.
    let ext = json!({
        "nodes": [
            {"id": "a", "label": "A", "file_type": "code", "source_file": "a.py"},
            {"id": ["x", "y"], "label": "B", "file_type": "code", "source_file": "b.py"},
            {"label": "C", "file_type": "code", "source_file": "c.py"},
        ],
        "edges": [],
    });
    let g = build_from_json(ext, false, None).expect("build");
    let ids: std::collections::BTreeSet<String> = g.nodes().map(|(id, _)| id.clone()).collect();
    assert_eq!(ids, ["a".to_string()].into_iter().collect());
}

#[test]
fn build_from_json_skips_edge_with_non_hashable_endpoint() {
    // A list-valued edge endpoint must be skipped; the well-formed edge survives.
    let ext = json!({
        "nodes": [
            {"id": "a", "label": "A", "file_type": "code", "source_file": "a.py"},
            {"id": "b", "label": "B", "file_type": "code", "source_file": "b.py"},
        ],
        "edges": [
            {"source": "a", "target": ["b", "c"], "relation": "calls",
             "confidence": "INFERRED", "source_file": "a.py"},
            {"source": "a", "target": "b", "relation": "imports",
             "confidence": "EXTRACTED", "source_file": "a.py"},
        ],
    });
    let g = build_from_json(ext, false, None).expect("build");
    assert_eq!(g.node_count(), 2);
    assert_eq!(g.edge_count(), 1);
    assert!(g.edge_data("a", "b").is_some());
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
fn add_edge_preserves_first_direction_on_bidirectional_pair() {
    // #1061 on the single-call path. `add_edge` (used e.g. by the global-graph
    // merge) must apply the same first-seen-direction guard as `bulk_add_edges`:
    // adding `a_handler -> z_emitter` then the reverse must collapse to one
    // undirected edge that keeps the first-seen _src/_tgt.
    let mut g = Graph::new(GraphKind::Graph);
    g.add_node("a_handler", indexmap::IndexMap::new());
    g.add_node("z_emitter", indexmap::IndexMap::new());

    let mut forward = indexmap::IndexMap::new();
    forward.insert("relation".to_string(), json!("calls"));
    forward.insert("_src".to_string(), json!("a_handler"));
    forward.insert("_tgt".to_string(), json!("z_emitter"));
    g.add_edge("a_handler", "z_emitter", forward);

    let mut reverse = indexmap::IndexMap::new();
    reverse.insert("relation".to_string(), json!("calls"));
    reverse.insert("_src".to_string(), json!("z_emitter"));
    reverse.insert("_tgt".to_string(), json!("a_handler"));
    g.add_edge("z_emitter", "a_handler", reverse);

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
        "add_edge flipped the calls edge source on bidirectional collision"
    );
    assert_eq!(
        data.get("_tgt").and_then(Value::as_str),
        Some("z_emitter"),
        "add_edge flipped the calls edge target on bidirectional collision"
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

// ── #1145 (extended #1271): LLM ghost-duplicate merge into AST canonical ────

#[test]
fn ghost_duplicate_merged_into_ast_node() {
    // AST node uses a parent-qualified id and is stamped `_origin=ast`; the LLM
    // emits a bare-stem id with the same (basename, label). The ghost must be
    // removed and the edge that referenced it re-pointed at the AST node.
    let extraction = json!({
        "nodes": [
            {"id": "mod_get_pairs", "label": "get_pairs", "source_file": "src/bpe.py",
             "source_location": "L5", "_origin": "ast", "file_type": "code"},
            {"id": "get_pairs", "label": "get_pairs", "source_file": "src/bpe.py",
             "source_location": "L5", "file_type": "code"},
            {"id": "caller", "label": "caller", "source_file": "src/app.py", "_origin": "ast"},
        ],
        "edges": [
            {"source": "caller", "target": "get_pairs", "relation": "calls"},
        ],
    });
    let g = build_from_json(extraction, true, None).expect("build");
    assert!(g.contains_node("mod_get_pairs"), "AST canonical node kept");
    assert!(!g.contains_node("get_pairs"), "LLM ghost removed");
    assert!(
        g.edge_data("caller", "mod_get_pairs").is_some(),
        "edge re-pointed to the canonical node"
    );
}

#[test]
fn ghost_merge_keeps_distinct_symbols_in_different_files() {
    // Same label, different source files → distinct symbols, never merged.
    let extraction = json!({
        "nodes": [
            {"id": "a_render", "label": "render", "source_file": "a.py", "_origin": "ast"},
            {"id": "b_render", "label": "render", "source_file": "b.py", "_origin": "ast"},
        ],
        "edges": [],
    });
    let g = build_from_json(extraction, true, None).expect("build");
    assert!(g.contains_node("a_render"));
    assert!(g.contains_node("b_render"));
    assert_eq!(g.node_count(), 2);
}

// ── #1317: dedupe_nodes / dedupe_edges for the --no-cluster raw write path ──

#[test]
fn dedupe_edges_collapses_exact_parallels() {
    // #1317: --no-cluster / incremental update concatenate edge lists raw.
    let edges = [
        json!({"source": "a", "target": "b", "relation": "calls", "source_location": "L1"}),
        json!({"source": "a", "target": "b", "relation": "calls", "source_location": "L9"}),
        json!({"source": "a", "target": "b", "relation": "imports"}),
        json!({"source": "b", "target": "c", "relation": "calls"}),
    ];
    let out = dedupe_edges(&edges);
    let keys: Vec<(&str, &str, &str)> = out
        .iter()
        .map(|e| {
            (
                e.get("source").and_then(Value::as_str).unwrap_or(""),
                e.get("target").and_then(Value::as_str).unwrap_or(""),
                e.get("relation").and_then(Value::as_str).unwrap_or(""),
            )
        })
        .collect();
    assert_eq!(
        keys,
        vec![
            ("a", "b", "calls"),
            ("a", "b", "imports"),
            ("b", "c", "calls")
        ]
    );
    // First occurrence wins (keeps L1, not L9).
    assert_eq!(
        out[0].get("source_location").and_then(Value::as_str),
        Some("L1")
    );
}

#[test]
fn dedupe_edges_is_idempotent() {
    let edges = [
        json!({"source": "a", "target": "b", "relation": "calls"}),
        json!({"source": "a", "target": "b", "relation": "calls"}),
    ];
    let once = dedupe_edges(&edges);
    // Simulate a second `update` re-concatenating its edges.
    let mut combined = once.clone();
    combined.extend(edges.iter().cloned());
    let twice = dedupe_edges(&combined);
    assert_eq!(once.len(), 1);
    assert_eq!(twice.len(), 1);
}

#[test]
fn dedupe_nodes_collapses_by_id_last_wins() {
    // #1327: a shared module anchor is emitted once per importing file; the
    // --no-cluster raw writer must collapse same-id node dicts (#1317).
    let nodes = [
        json!({"id": "foundation", "label": "Foundation", "type": "module", "source_file": "A.swift"}),
        json!({"id": "akit", "label": "AKit", "file_type": "code"}),
        json!({"id": "foundation", "label": "Foundation", "type": "module", "source_file": "B.swift"}),
    ];
    let out = dedupe_nodes(&nodes);
    let ids: Vec<&str> = out
        .iter()
        .map(|n| n.get("id").and_then(Value::as_str).unwrap_or(""))
        .collect();
    assert_eq!(ids, vec!["foundation", "akit"]); // first-appearance order
    // Last writer wins on attributes.
    let foundation = out
        .iter()
        .find(|n| n.get("id").and_then(Value::as_str) == Some("foundation"))
        .expect("foundation node present");
    assert_eq!(
        foundation.get("source_file").and_then(Value::as_str),
        Some("B.swift")
    );
}

// ── #1279: edge source_file backfilled from endpoint node ───────────────────

#[test]
fn edge_missing_source_file_backfilled_from_node() {
    // #1279: a semantic/LLM edge lacking source_file must inherit it from its
    // source node rather than reach graph.json with no file reference.
    let extraction = json!({
        "nodes": [
            {"id": "n1", "label": "A", "file_type": "concept", "source_file": "docs/a.md"},
            {"id": "n2", "label": "B", "file_type": "concept", "source_file": "docs/b.md"},
        ],
        // No source_file on the edge (as LLM output sometimes omits it).
        "edges": [{"source": "n1", "target": "n2", "relation": "relates_to", "confidence": "INFERRED"}],
        "input_tokens": 0, "output_tokens": 0,
    });
    let g = build_from_json(extraction, false, None).expect("build");
    let sf = g
        .edge_data("n1", "n2")
        .and_then(|attrs| attrs.get("source_file"))
        .and_then(Value::as_str);
    assert_eq!(sf, Some("docs/a.md")); // backfilled from the source node
}

// ── #1257: ghost-merge skipped on ambiguous (basename, label) collision ─────

#[test]
fn ghost_merge_unique_located_node_still_merges() {
    // #1145: a semantic ghost collapses into the single AST node sharing its
    // (basename, label), and the edge re-points to the AST node.
    let ext = json!({
        "nodes": [
            {"id": "ast_render", "label": "render", "file_type": "code",
             "source_file": "src/app/index.ts", "source_location": "L10", "_origin": "ast"},
            {"id": "ghost_render", "label": "render", "file_type": "code",
             "source_file": "src/app/index.ts"},
            {"id": "caller", "label": "main", "file_type": "code",
             "source_file": "src/main.ts", "source_location": "L1", "_origin": "ast"},
        ],
        "edges": [{"source": "caller", "target": "ghost_render", "relation": "calls",
                   "confidence": "EXTRACTED", "source_file": "src/main.ts", "weight": 1.0}],
        "input_tokens": 0, "output_tokens": 0,
    });
    let g = build_from_json(ext, false, None).expect("build");
    assert!(!g.contains_node("ghost_render"), "ghost removed");
    assert!(
        g.edge_data("caller", "ast_render").is_some(),
        "edge re-pointed to the AST node"
    );
}

#[test]
fn ghost_merge_skipped_on_basename_collision() {
    // #1257: when two files with the same basename both define a symbol with the
    // same label, the (basename, label) key is ambiguous and the semantic ghost
    // must not be merged into an arbitrary one of them.
    let ext = json!({
        "nodes": [
            {"id": "a_render", "label": "render", "file_type": "code",
             "source_file": "src/a/index.ts", "source_location": "L10", "_origin": "ast"},
            {"id": "b_render", "label": "render", "file_type": "code",
             "source_file": "src/b/index.ts", "source_location": "L20", "_origin": "ast"},
            {"id": "ghost_render", "label": "render", "file_type": "code",
             "source_file": "src/a/index.ts"},
            {"id": "caller", "label": "main", "file_type": "code",
             "source_file": "src/main.ts", "source_location": "L1", "_origin": "ast"},
        ],
        "edges": [{"source": "caller", "target": "ghost_render", "relation": "calls",
                   "confidence": "EXTRACTED", "source_file": "src/main.ts", "weight": 1.0}],
        "input_tokens": 0, "output_tokens": 0,
    });
    let g = build_from_json(ext, false, None).expect("build");
    // The ghost survives: merging it into either a_render or b_render would
    // pick an arbitrary winner via iteration order over the node set.
    assert!(g.contains_node("ghost_render"));
    assert_eq!(g.node_count(), 4);
    assert!(g.edge_data("caller", "ghost_render").is_some());
    assert!(g.edge_data("caller", "a_render").is_none());
    assert!(g.edge_data("caller", "b_render").is_none());
}

// ── #1344: build_merge replaces a re-extracted file's stale contribution ────

#[test]
fn build_merge_replaces_changed_file_stale_edges() {
    // Re-extracting a CHANGED file must REPLACE its prior nodes/edges, not
    // accumulate them (#1344). The new-chunk source_file may be an absolute
    // win32 path while the stored graph keeps relative posix — both forms match.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canon").join("corpus");
    std::fs::create_dir(&root).expect("mkdir");
    let graph_path = tmp.path().join("graph.json");

    // First build: changed.md contributed A, B and edge A->B; keep.md unrelated.
    let stored = json!({
        "nodes": [
            {"id": "A", "label": "A", "file_type": "document", "source_file": "changed.md"},
            {"id": "B", "label": "B", "file_type": "document", "source_file": "changed.md"},
            {"id": "K", "label": "K", "file_type": "document", "source_file": "keep.md"},
        ],
        "edges": [
            {"source": "A", "target": "B", "relation": "references", "confidence": "EXTRACTED",
             "source_file": "changed.md", "weight": 1.0},
            {"source": "K", "target": "A", "relation": "references", "confidence": "EXTRACTED",
             "source_file": "keep.md", "weight": 1.0},
        ],
    });
    std::fs::write(&graph_path, serde_json::to_string(&stored).expect("ser")).expect("write");

    // changed.md edited: re-extraction now yields A, C and edge A->C (B dropped).
    // source_file arrives as an absolute win32-style path (as detect emits on Windows).
    let abs_changed = root.join("changed.md").to_string_lossy().replace('/', "\\");
    let new_chunk = json!({
        "nodes": [
            {"id": "A", "label": "A", "file_type": "document", "source_file": abs_changed.clone()},
            {"id": "C", "label": "C", "file_type": "document", "source_file": abs_changed.clone()},
        ],
        "edges": [
            {"source": "A", "target": "C", "relation": "references", "confidence": "EXTRACTED",
             "source_file": abs_changed.clone(), "weight": 1.0},
        ],
    });
    let g = build_merge(&[new_chunk], &graph_path, None, false, false, Some(&root))
        .expect("build_merge");

    let labels = node_labels(&g);
    let has_edge = |u: &str, v: &str| g.edge_data(u, v).is_some();

    // Stale contribution from the old version of changed.md is gone.
    assert!(
        !labels.contains("B"),
        "stale node from changed file's old version must be dropped"
    );
    assert!(!has_edge("A", "B"), "stale edge must be dropped");
    // Fresh contribution is present.
    assert!(labels.contains("C"), "re-extracted node must be present");
    assert!(has_edge("A", "C"), "re-extracted edge must be present");
    // An unchanged file is untouched.
    assert!(labels.contains("K"), "unchanged file's node must survive");
    assert!(has_edge("K", "A"), "unchanged file's edge must survive");
}

#[test]
fn build_merge_root_collapses_convention_drift() {
    // #1344: the caller must pass root so build_merge canonicalizes the new
    // chunk to the same relative base as the stored graph; only then does
    // re-extraction REPLACE the prior node (incl. stale nodes) for that file.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().canonicalize().expect("canon");
    let graph_dir = root.join("graphify-out");
    std::fs::create_dir_all(&graph_dir).expect("mkdir");
    let graph_path = graph_dir.join("graph.json");

    // Stored graph: nested project-relative convention + a STALE node for the
    // same file that the re-extraction no longer emits.
    let stored = json!({
        "nodes": [
            {"id": "wiki_overview_overview", "label": "Overview", "file_type": "document",
             "source_file": "docs/wiki/overview.md"},
            {"id": "wiki_overview_stale", "label": "Stale", "file_type": "document",
             "source_file": "docs/wiki/overview.md"},
        ],
        "edges": [],
    });
    let saved = serde_json::to_string(&stored).expect("ser");
    std::fs::write(&graph_path, &saved).expect("write");

    // BUG: --update drifted to a bare basename and no root was passed. Different
    // base -> source_file replace misses -> stale + duplicate both survive.
    let drift = json!({
        "nodes": [
            {"id": "overview_overview", "label": "Overview", "file_type": "document",
             "source_file": "overview.md"},
        ],
        "edges": [],
    });
    let g_bug = build_merge(&[drift], &graph_path, None, false, false, None).expect("build_merge");
    assert_eq!(
        g_bug.node_count(),
        3,
        "mismatched base must NOT replace -> stale+dup remain"
    );

    // FIX: caller passes root; the verbatim absolute path canonicalizes to the
    // stored relative base, so the re-extraction replaces the prior node.
    std::fs::write(&graph_path, &saved).expect("rewrite");
    let abs_overview = root
        .join("docs")
        .join("wiki")
        .join("overview.md")
        .to_string_lossy()
        .into_owned();
    let fixed = json!({
        "nodes": [
            {"id": "wiki_overview_overview", "label": "Overview", "file_type": "document",
             "source_file": abs_overview},
        ],
        "edges": [],
    });
    let g_ok =
        build_merge(&[fixed], &graph_path, None, false, false, Some(&root)).expect("build_merge");
    assert_eq!(
        g_ok.node_count(),
        1,
        "verbatim path + root must collapse to one node"
    );
    assert!(
        !g_ok.contains_node("docs_wiki_overview_stale"),
        "stale node for the re-extracted file must be dropped"
    );
    assert_eq!(
        g_ok.node_data("docs_wiki_overview_overview")
            .and_then(|a| a.get("source_file"))
            .and_then(Value::as_str),
        Some("docs/wiki/overview.md"),
        "new chunk must be canonicalized to the stored relative base"
    );
}

// ── #1504 migration: legacy-id detection + re-key source_file contract ─────────

#[test]
fn graph_has_legacy_ids_detects_old_scheme() {
    // The read-only-consumer nudge flags a pre-#1504 graph and leaves a canonical
    // one alone. Mirrors test_build.py::test_graph_has_legacy_ids_detects_old_scheme.
    let old = [
        json!({"id": "api_readme", "source_file": "docs/v1/api/README.md",
                      "type": "document", "source_location": "L1"}),
    ];
    let new = [
        json!({"id": "docs_v1_api_readme", "source_file": "docs/v1/api/README.md",
                      "type": "document", "source_location": "L1"}),
    ];
    assert!(graph_has_legacy_ids(&old, Some(".")));
    assert!(!graph_has_legacy_ids(&new, Some(".")));
    // sourceless / top-level file nodes don't false-positive.
    assert!(!graph_has_legacy_ids(
        &[json!({"id": "setup", "source_file": "setup.py", "source_location": "L1"})],
        Some("."),
    ));
    assert!(!graph_has_legacy_ids(
        &[json!({"id": "x", "label": "y"})],
        Some(".")
    ));
    // A package/dir-scoped SYMBOL id (Go's _make_id(pkg_dir, name) -> "sub_thing")
    // must NOT false-positive: it isn't file-level (no L1), so it's ignored even
    // though "sub_thing" coincides with the old file-stem form of pkg/sub/thing.go.
    let go_symbol = [json!({"id": "sub_thing", "source_file": "pkg/sub/thing.go",
                           "type": "code", "source_location": "L3"})];
    assert!(!graph_has_legacy_ids(&go_symbol, Some(".")));
}

#[test]
fn semantic_rekey_migrates_relative_leaves_absolute() {
    // Re-key contract (#1504): a relative source_file is migrated to the full-path
    // stem; an absolute one with no resolvable root is left untouched so its
    // on-disk path can't leak into IDs. Mirrors
    // test_build.py::test_semantic_rekey_relative_vs_absolute_source_file, exercised
    // through the observable build_from_json output.
    let rel = json!({
        "nodes": [{"id": "api_readme", "source_file": "docs/v1/api/README.md",
                   "file_type": "document"}],
        "edges": [],
    });
    let g = build_from_json(rel, false, Some(std::path::Path::new("."))).expect("build");
    assert!(g.contains_node("docs_v1_api_readme"));
    assert!(!g.contains_node("api_readme"));

    // A genuinely-absolute path (platform-native via canonicalize, so the test
    // exercises `Path::is_absolute` on Windows too) is left un-rekeyed: its id
    // can't be derived without leaking the temp prefix, so it stays as-is.
    let tmp = tempfile::tempdir().expect("tempdir");
    let abs_source = tmp
        .path()
        .canonicalize()
        .expect("canonicalize tmp")
        .join("docs")
        .join("v1")
        .join("api")
        .join("README.md")
        .to_string_lossy()
        .into_owned();
    let abs = json!({
        "nodes": [{"id": "api_readme", "source_file": abs_source,
                   "file_type": "document"}],
        "edges": [],
    });
    let g2 = build_from_json(abs, false, None).expect("build");
    assert!(g2.contains_node("api_readme"));
    assert!(!g2.contains_node("abs_docs_v1_api_readme"));
}

// ── #1536: corrupt graph.json on build_merge ─────────────────────────────────

#[test]
fn test_build_merge_corrupt_graph_raises_actionable_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = tmp.path().join("graph.json");
    std::fs::write(&gp, "{ not valid json").expect("write");
    let err = build_merge(&[], &gp, None, false, false, None).expect_err("corrupt graph errors");
    let msg = format!("{err}");
    assert!(msg.contains("Cannot read"), "msg: {msg}");
    assert!(
        msg.contains("incremental merge") || msg.contains("rebuild"),
        "msg: {msg}"
    );
}

#[test]
fn test_build_merge_valid_graph_still_loads() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = tmp.path().join("graph.json");
    std::fs::write(
        &gp,
        r#"{"nodes": [{"id": "a", "label": "A", "file_type": "code", "source_file": "a.py"}], "links": []}"#,
    )
    .expect("write");
    let new = json!({"nodes": [{"id": "b", "label": "B", "file_type": "code", "source_file": "b.py"}], "edges": []});
    let g = build_merge(&[new], &gp, None, false, false, None).expect("valid merge");
    assert!(
        g.node_count() >= 2,
        "both nodes present: {}",
        g.node_count()
    );
}

// ── #1753: deterministic ghost-node merge — distinct non-AST concepts survive ──

fn label_count(g: &Graph, label: &str) -> usize {
    g.nodes()
        .filter(|(_, a)| a.get("label").and_then(Value::as_str) == Some(label))
        .count()
}

#[test]
fn test_ghost_merge_non_ast_different_files_both_survive() {
    // Two non-AST concept nodes sharing (basename, label) but from DIFFERENT
    // files are distinct concepts — both must survive, not collapse (#1753).
    let extraction = json!({
        "nodes": [
            {"id": "a_update", "label": "update", "file_type": "concept", "source_file": "dir_a/update.md", "source_location": "L1"},
            {"id": "b_update", "label": "update", "file_type": "concept", "source_file": "dir_b/update.md", "source_location": "L1"},
        ],
        "edges": []
    });
    let g = build_from_json(extraction, false, None).expect("build");
    assert_eq!(
        label_count(&g, "update"),
        2,
        "both distinct concepts survive"
    );
}

#[test]
fn test_ghost_merge_non_ast_same_file_still_merges() {
    // A genuine same-file duplicate (identical source_file) still collapses.
    let extraction = json!({
        "nodes": [
            {"id": "u1", "label": "update", "file_type": "concept", "source_file": "dir_a/update.md", "source_location": "L1"},
            {"id": "u2", "label": "update", "file_type": "concept", "source_file": "dir_a/update.md", "source_location": "L2"},
        ],
        "edges": []
    });
    let g = build_from_json(extraction, false, None).expect("build");
    assert_eq!(
        label_count(&g, "update"),
        1,
        "same-file duplicate collapses"
    );
}

// ── #1749: cross-language imports/references guard ───────────────────────────

fn has_edge(g: &Graph, rel: &str, src_label: &str, tgt_label: &str) -> bool {
    let id_of = |label: &str| -> Option<String> {
        g.nodes()
            .find(|(_, a)| a.get("label").and_then(Value::as_str) == Some(label))
            .map(|(id, _)| id.clone())
    };
    let (Some(s), Some(t)) = (id_of(src_label), id_of(tgt_label)) else {
        return false;
    };
    g.edges().any(|e| {
        e.source == s
            && e.target == t
            && e.attrs.get("relation").and_then(Value::as_str) == Some(rel)
    })
}

#[test]
fn test_cross_language_imports_references_are_dropped() {
    // A Python `import time` must not bind to a same-named `time.ts` (#1749).
    let extraction = json!({
        "nodes": [
            {"id": "pa", "label": "a.py", "file_type": "code", "source_file": "a.py"},
            {"id": "tt", "label": "time.ts", "file_type": "code", "source_file": "src/time.ts"},
            {"id": "tb", "label": "b.ts", "file_type": "code", "source_file": "src/b.ts"},
        ],
        "edges": [
            {"source": "pa", "target": "tt", "relation": "imports_from", "confidence": "EXTRACTED", "source_file": "a.py"},
            {"source": "tb", "target": "tt", "relation": "imports_from", "confidence": "EXTRACTED", "source_file": "src/b.ts"},
        ]
    });
    let g = build_from_json(extraction, false, None).expect("build");
    assert!(
        !has_edge(&g, "imports_from", "a.py", "time.ts"),
        "py→ts import dropped"
    );
    assert!(
        has_edge(&g, "imports_from", "b.ts", "time.ts"),
        "ts→ts import kept"
    );
}

#[test]
fn test_cross_family_reference_to_unknown_ext_is_kept() {
    // A config/manifest (unknown ext) → code reference must survive: only BOTH
    // endpoints being known code languages of different families is a phantom.
    let extraction = json!({
        "nodes": [
            {"id": "pkg", "label": "package.json", "file_type": "config", "source_file": "package.json"},
            {"id": "app", "label": "app.ts", "file_type": "code", "source_file": "src/app.ts"},
        ],
        "edges": [
            {"source": "pkg", "target": "app", "relation": "references", "confidence": "EXTRACTED", "source_file": "package.json"}
        ]
    });
    let g = build_from_json(extraction, false, None).expect("build");
    assert!(
        has_edge(&g, "references", "package.json", "app.ts"),
        "config→code reference kept"
    );
}

// ── #1574: build_merge preserves unchanged-file hyperedges ───────────────────

#[test]
fn test_build_merge_preserves_unchanged_hyperedges() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = tmp.path().join("graph.json");
    std::fs::write(
        &gp,
        r#"{"nodes": [{"id": "d1", "label": "doc", "file_type": "document", "source_file": "docs/a.md"}],
            "links": [],
            "hyperedges": [{"id": "h1", "nodes": ["d1"], "source_file": "docs/a.md"}]}"#,
    )
    .expect("write");
    // Re-extract a DIFFERENT file; the unchanged file's hyperedge must survive.
    let new = json!({"nodes": [{"id": "c1", "label": "code", "file_type": "code", "source_file": "src/x.py"}], "edges": [], "hyperedges": []});
    let g = build_merge(&[new], &gp, None, false, false, None).expect("merge");
    let hyper = g
        .graph_attrs
        .get("hyperedges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        hyper
            .iter()
            .any(|h| h.get("id").and_then(Value::as_str) == Some("h1")),
        "unchanged-file hyperedge preserved, got {hyper:?}"
    );
}
