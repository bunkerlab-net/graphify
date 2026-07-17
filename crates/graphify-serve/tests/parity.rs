//! Parity tests against `graphify-py/tests/test_serve.py`.
//!
//! All test cases from the Python test suite are ported here. We use
//! `graphify_build::build_from_json` to construct `Graph` objects rather than
//! the Python `networkx` constructors.
#![allow(clippy::expect_used)]

use std::collections::HashMap;

use graphify_build::{Graph, GraphKind, build_from_json};
use graphify_prs::error::PrsError;
use graphify_prs::gh::GhClient;
use graphify_prs::git::GitClient;
use graphify_serve::graph::{
    bfs, communities_from_graph, compute_idf, dfs, filter_graph_by_context, find_node,
    infer_context_filters, load_graph, normalize_context_filters, pick_seeds, pick_seeds_diverse,
    query_graph_text, query_terms, resolve_context_filters, score_nodes, subgraph_to_text,
};
use graphify_serve::tools::{
    tool_get_pr_impact_with_clients, tool_list_prs_with_clients, tool_triage_prs_with_clients,
};
use serde_json::json;
use tempfile::tempdir;

// ── Test doubles for PR tool tests ────────────────────────────────────────────

/// One canned PR in the wire format that `gh pr list` returns.
const CANNED_PR_JSON: &str = r#"[{
    "number": 42,
    "title": "Add feature X",
    "headRefName": "feature/x",
    "baseRefName": "main",
    "author": {"login": "alice"},
    "isDraft": false,
    "reviewDecision": "APPROVED",
    "statusCheckRollup": [{"conclusion": "SUCCESS", "status": "COMPLETED"}],
    "updatedAt": "2025-01-01T00:00:00Z"
}]"#;

#[cfg(test)]
struct FakeGhClient {
    prs_json: &'static str,
    files: Vec<String>,
    default_branch: Option<String>,
}

impl GhClient for FakeGhClient {
    fn pr_list(&self, _repo: Option<&str>, _limit: usize) -> Result<Vec<u8>, PrsError> {
        Ok(self.prs_json.as_bytes().to_vec())
    }

    fn repo_default_branch(&self, _repo: Option<&str>) -> Option<String> {
        self.default_branch.clone()
    }

    fn pr_files(&self, _number: u64, _repo: Option<&str>) -> Vec<String> {
        self.files.clone()
    }
}

#[cfg(test)]
struct FakeGitClient;

impl GitClient for FakeGitClient {
    fn worktree_list_porcelain(&self) -> Option<String> {
        None
    }

    fn symbolic_ref_origin_head(&self) -> Option<String> {
        None
    }
}

// ── Test graph factory ────────────────────────────────────────────────────────

/// Mirrors Python `_make_graph()`.
fn make_graph() -> Graph {
    build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "extract", "source_file": "extract.py",
                 "source_location": "L10", "community": 0},
                {"id": "n2", "label": "cluster", "source_file": "cluster.py",
                 "source_location": "L5", "community": 0},
                {"id": "n3", "label": "build", "source_file": "build.py",
                 "source_location": "L1", "community": 1},
                {"id": "n4", "label": "report", "source_file": "report.py",
                 "source_location": "L1", "community": 1},
                {"id": "n5", "label": "isolated", "source_file": "other.py",
                 "source_location": "L1", "community": 2},
            ],
            "edges": [
                {"source": "n1", "target": "n2", "relation": "calls",
                 "confidence": "INFERRED", "context": "call"},
                {"source": "n2", "target": "n3", "relation": "imports",
                 "confidence": "EXTRACTED", "context": "import"},
                {"source": "n3", "target": "n4", "relation": "uses",
                 "confidence": "EXTRACTED"},
            ]
        }),
        // Use undirected to match Python nx.Graph in test_serve.py
        false,
        None,
    )
    .expect("make_graph")
}

/// Mirrors Python `_make_noisy_graph()`.
fn make_noisy_graph() -> Graph {
    let mut nodes = vec![];
    let mut edges = vec![];
    for i in 0..20_u64 {
        nodes.push(json!({
            "id": format!("err{i}"),
            "label": format!("error_handler_{i}"),
            "source_file": format!("err{i}.py"),
            "community": 0
        }));
        if i > 0 {
            edges.push(json!({
                "source": format!("err{}", i - 1),
                "target": format!("err{i}"),
                "relation": "calls",
                "confidence": "EXTRACTED"
            }));
        }
    }
    nodes.push(json!({
        "id": "fbs",
        "label": "FooBarService",
        "source_file": "service.py",
        "community": 1
    }));
    nodes.push(json!({
        "id": "fbs_dep",
        "label": "ServiceClient",
        "source_file": "client.py",
        "community": 1
    }));
    edges.push(json!({
        "source": "fbs",
        "target": "fbs_dep",
        "relation": "uses",
        "confidence": "EXTRACTED"
    }));
    build_from_json(json!({"nodes": nodes, "edges": edges}), false, None).expect("make_noisy_graph")
}

// ── _communities_from_graph ───────────────────────────────────────────────────

#[test]
fn test_communities_from_graph_basic() {
    let g = make_graph();
    let communities = communities_from_graph(&g);
    assert!(communities.contains_key(&0));
    assert!(communities.contains_key(&1));
    assert!(communities[&0].contains(&"n1".to_string()));
    assert!(communities[&0].contains(&"n2".to_string()));
    assert!(communities[&1].contains(&"n3".to_string()));
}

#[test]
fn test_communities_from_graph_no_community_attr() {
    let g = build_from_json(
        json!({"nodes": [{"id": "a", "label": "foo"}], "edges": []}),
        false,
        None,
    )
    .expect("graph");
    let communities = communities_from_graph(&g);
    assert!(communities.is_empty());
}

#[test]
fn test_communities_from_graph_isolated() {
    let g = make_graph();
    let communities = communities_from_graph(&g);
    assert!(communities.contains_key(&2));
    assert!(communities[&2].contains(&"n5".to_string()));
}

// ── _score_nodes ─────────────────────────────────────────────────────────────

#[test]
fn test_score_nodes_exact_label_match() {
    let g = make_graph();
    let mut cache = HashMap::new();
    let scored = score_nodes(&g, &["extract"], &mut cache);
    assert!(!scored.is_empty());
    let nids: Vec<&str> = scored.iter().map(|(_, nid)| nid.as_str()).collect();
    assert!(nids.contains(&"n1"));
    assert_eq!(scored[0].1, "n1", "highest score should be n1");
}

#[test]
fn test_score_nodes_no_match() {
    let g = make_graph();
    let mut cache = HashMap::new();
    let scored = score_nodes(&g, &["xyzzy"], &mut cache);
    assert!(scored.is_empty());
}

#[test]
fn test_score_nodes_source_file_partial() {
    let g = make_graph();
    let mut cache = HashMap::new();
    // "cluster.py" contains "cluster" — should score for source match
    let scored = score_nodes(&g, &["cluster"], &mut cache);
    let nids: Vec<&str> = scored.iter().map(|(_, nid)| nid.as_str()).collect();
    assert!(nids.contains(&"n2"));
}

#[test]
fn test_score_nodes_ignores_trailing_punctuation() {
    let g = make_graph();
    let mut cache = HashMap::new();
    let scored = score_nodes(&g, &["extract?"], &mut cache);
    assert!(!scored.is_empty(), "expected at least one match");
    assert_eq!(scored[0].1, "n1");
}

#[test]
fn test_score_nodes_multiword_exact_label_outranks_superset() {
    // A multi-word query equal to a whole label must resolve uniquely and
    // strictly outrank a superset/decoy that shares the same token set
    // (regression for the `graphify path` "No path found" bug). norm_label keeps
    // the ':' punctuation; the exact node wins via the label's tokenized form.
    let g = build_from_json(
        json!({
            "nodes": [
                {"id": "exact", "label": "UOCE: Dehumidifier Driver",
                 "source_file": "uoce_dehumidifier.yaml", "community": 0},
                {"id": "super", "label": "UOCE: Dehumidifier Driver State Machine",
                 "source_file": "uoce_dehumidifier.yaml", "community": 0},
                {"id": "decoy", "label": "Dehumidifier Driver Helper",
                 "source_file": "uoce_dehumidifier.yaml", "community": 0},
            ],
            "edges": []
        }),
        false,
        None,
    )
    .expect("graph");
    let mut cache = HashMap::new();
    // CLI resolves endpoints as [t.lower() for t in label.split()].
    let scored = score_nodes(&g, &["uoce:", "dehumidifier", "driver"], &mut cache);
    assert_eq!(scored[0].1, "exact");
    assert!(
        scored[0].0 > scored[1].0,
        "exact label must strictly outrank superset/token-bag matches"
    );
}

#[test]
fn test_score_nodes_coverage_lone_generic_exact_hit_loses_to_multi_term_match() {
    // #1602: in a multi-term query, a lone generic term that exactly equals a
    // short leaf label ("list" == a list() leaf) must NOT bury a node matching
    // several of the query's terms. Per-term exact/prefix tiers are scaled by
    // squared term coverage. Leaves sit in the target's own directory to pin
    // that source-path hits do NOT count toward coverage.
    let mut nodes = vec![
        json!({"id": "target", "label": "ClientLive.Index", "source_file": "lib/clients_live/index.ex", "community": 0}),
        json!({"id": "form", "label": "ClientLive.Form", "source_file": "lib/clients_live/form.ex", "community": 0}),
        json!({"id": "show", "label": "ClientLive.Show", "source_file": "lib/clients_live/show.ex", "community": 0}),
    ];
    for i in 0..3 {
        nodes.push(json!({"id": format!("leaf{i}"), "label": "list()", "source_file": format!("lib/clients_live/helpers{i}.ex"), "community": 0}));
    }
    for i in 0..24 {
        nodes.push(json!({"id": format!("filler{i}"), "label": format!("shopping list {i}"), "source_file": format!("lib/filler{i}.ex"), "community": 0}));
    }
    let g = build_from_json(json!({"nodes": nodes, "edges": []}), false, None).expect("graph");
    let mut cache = HashMap::new();
    let scored = score_nodes(
        &g,
        &["clientlive", "index", "clients", "list", "columns"],
        &mut cache,
    );
    let by_id: HashMap<&str, f64> = scored.iter().map(|(s, n)| (n.as_str(), *s)).collect();
    assert_eq!(scored[0].1, "target");
    assert!(
        by_id["target"] > by_id["leaf0"],
        "a 1-of-5-terms exact collision must not outrank the node matching 3 of 5 terms"
    );
}

#[test]
fn test_score_nodes_coverage_full_coverage_query_is_unchanged() {
    // A single-term (coverage == 1) identifier lookup keeps the exact tier's full
    // magnitude, so #1602 dampening never touches it. Full-query exact tier (10x)
    // + per-term exact tier + source hit ("extract" in "extract.py"), undampened.
    let g = make_graph();
    let mut cache = HashMap::new();
    let scored = score_nodes(&g, &["extract"], &mut cache);
    let w = *compute_idf(&g, &["extract"], &mut cache)
        .get("extract")
        .expect("idf for extract");
    // EXACT=1000, SOURCE=0.5 (see graph.rs constants).
    let expected = (1000.0 * 10.0 + 1000.0 + 0.5) * w;
    assert_eq!(scored[0].1, "n1");
    assert!(
        (scored[0].0 - expected).abs() <= expected.abs() * 1e-9,
        "single-term exact score {} must equal the undampened {expected}",
        scored[0].0
    );
}

#[test]
fn test_find_node_ignores_trailing_punctuation() {
    let g = make_graph();
    assert_eq!(find_node(&g, "extract?"), vec!["n1".to_string()]);
}

#[test]
fn test_find_node_matches_full_punctuated_unicode_label() {
    // Mirrors graphify-py test_serve.py::test_find_node_matches_full_punctuated_unicode_label.
    let g = build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "Skill /auditar — Auditoría inquisitiva de enlaces"}
            ],
            "edges": []
        }),
        false,
        None,
    )
    .expect("build graph");
    assert_eq!(
        find_node(&g, "Skill /auditar — Auditoría inquisitiva de enlaces"),
        vec!["n1".to_string()]
    );
}

#[test]
fn test_find_node_matches_punctuated_file_label_exactly() {
    // #1704: an exactly-typed punctuated file label must resolve through explain,
    // just like it does through path/query. Built directly so the explicit
    // norm_label is not rewritten by build_from_json canonicalization.
    let mut g = Graph::new(GraphKind::Graph);
    let mk = |label: &str, norm: &str, src: &str| {
        let mut a: indexmap::IndexMap<String, serde_json::Value> = indexmap::IndexMap::new();
        a.insert("label".to_string(), json!(label));
        a.insert("norm_label".to_string(), json!(norm));
        a.insert("source_file".to_string(), json!(src));
        a.insert("source_location".to_string(), json!("L1"));
        a
    };
    g.add_node(
        "f1",
        mk("blockStream.ts", "blockstream.ts", "lib/blockStream.ts"),
    );
    g.add_node(
        "f2",
        mk(
            "blockStream.test.ts",
            "blockstream.test.ts",
            "lib/blockStream.test.ts",
        ),
    );
    assert_eq!(find_node(&g, "blockStream.ts"), vec!["f1".to_string()]);
    assert_eq!(find_node(&g, "blockStream.test.ts"), vec!["f2".to_string()]);
}

#[test]
fn test_find_node_resolves_when_label_and_norm_label_diverge() {
    // #1704: when label ("BlockStream") and norm_label ("blockstream.ts") diverge,
    // the punctuation-preserving norm_query resolves the exactly-typed label.
    let mut g = Graph::new(GraphKind::Graph);
    let mut a: indexmap::IndexMap<String, serde_json::Value> = indexmap::IndexMap::new();
    a.insert("label".to_string(), json!("BlockStream"));
    a.insert("norm_label".to_string(), json!("blockstream.ts"));
    a.insert("source_file".to_string(), json!("lib/x.ts"));
    a.insert("source_location".to_string(), json!("L1"));
    g.add_node("n1", a);
    assert_eq!(find_node(&g, "blockStream.ts"), vec!["n1".to_string()]);
}

#[test]
fn test_find_node_source_file_path_prefers_file_level_node() {
    // #1503: a source-file path query floats the L1 file node ahead of the
    // symbols that share the file. `build_from_json` re-keys non-AST nodes to
    // their full repo-relative path id (#1504): example_route ->
    // app_api_example_route.
    let g = build_from_json(
        json!({
            "nodes": [
                {"id": "example_route_get", "label": "GET()",
                 "source_file": "app/api/example/route.ts", "source_location": "L42"},
                {"id": "example_route", "label": "route.ts",
                 "source_file": "app/api/example/route.ts", "source_location": "L1"},
            ],
            "edges": [],
        }),
        false,
        None,
    )
    .expect("make graph");
    let matches = find_node(&g, "app/api/example/route.ts");
    assert_eq!(
        matches.first().map(String::as_str),
        Some("app_api_example_route")
    );
    assert!(matches.iter().any(|m| m == "app_api_example_route_get"));
}

#[test]
fn test_find_node_source_file_path_backslash_prefers_file_level_node() {
    // #1503: a Windows-style backslash path query must behave like its
    // forward-slash twin — the basename is derived from slash-normalized
    // separators, so the L1 file node still floats ahead of its symbols.
    let g = build_from_json(
        json!({
            "nodes": [
                {"id": "example_route_get", "label": "GET()",
                 "source_file": "app/api/example/route.ts", "source_location": "L42"},
                {"id": "example_route", "label": "route.ts",
                 "source_file": "app/api/example/route.ts", "source_location": "L1"},
            ],
            "edges": [],
        }),
        false,
        None,
    )
    .expect("make graph");
    let matches = find_node(&g, "app\\api\\example\\route.ts");
    assert_eq!(
        matches.first().map(String::as_str),
        Some("app_api_example_route")
    );
    assert!(matches.iter().any(|m| m == "app_api_example_route_get"));
}

#[test]
fn test_query_terms_strips_search_punctuation() {
    // "what" is a question stopword (dropped); punctuation still stripped from "extract?".
    assert_eq!(
        query_terms("what calls extract?"),
        vec!["calls".to_string(), "extract".to_string()]
    );
}

#[test]
fn test_query_terms_drops_question_stopwords() {
    // Natural-language question/filler words are dropped so content words drive
    // seeding: "how does the frontier cache work" → content terms only (#6e97088).
    assert_eq!(
        query_terms("how does the frontier cache work"),
        vec!["frontier".to_string(), "cache".to_string()]
    );
}

#[test]
fn test_query_terms_all_stopwords_falls_back_to_unfiltered() {
    // An all-stopword query keeps its terms rather than seeding on nothing
    // ("it" is dropped as a short English token before the stopword filter).
    assert_eq!(
        query_terms("how does it work"),
        vec!["how".to_string(), "does".to_string(), "work".to_string()]
    );
}

#[test]
fn test_query_terms_drops_german_question_stopwords() {
    // #1900: German full-sentence queries reduce to the content noun. In a
    // mostly-English corpus "wie"/"funktioniert" are rare, get high IDF weight,
    // and out-seed the actual keyword unless dropped here.
    assert_eq!(
        query_terms("Wie funktioniert die Authentifizierung?"),
        vec!["authentifizierung".to_string()]
    );
}

#[test]
fn test_query_terms_all_german_stopwords_falls_back_to_unfiltered() {
    // The all-stopword fallback applies to German fillers too.
    assert_eq!(
        query_terms("wie funktioniert das"),
        vec![
            "wie".to_string(),
            "funktioniert".to_string(),
            "das".to_string()
        ]
    );
}

#[test]
fn test_pick_seeds_german_query_seeds_content_node_not_heading_noise() {
    // End-to-end for #1900: a German question over a graph with German
    // heading-noise nodes must seed on the content noun, not on nodes that
    // happen to contain 'die'/'wie'/'wird'.
    let mut g = Graph::new(GraphKind::DiGraph);
    let mk = |label: &str, src: &str| {
        indexmap::IndexMap::<String, serde_json::Value>::from([
            ("label".to_string(), json!(label)),
            ("source_file".to_string(), json!(src)),
        ])
    };
    g.add_node("cfg", mk("Die Konfiguration", "docs/konfiguration.md"));
    g.add_node("sec", mk("Wie wird gesichert", "docs/sicherheit.md"));
    g.add_node("auth", mk("Authentifizierung", "src/auth.py"));
    g.add_node("helper", mk("login_helper", "src/auth.py"));
    g.add_edge("helper", "auth", indexmap::IndexMap::new());

    let terms = query_terms("Wie funktioniert die Authentifizierung?");
    let term_refs: Vec<&str> = terms.iter().map(String::as_str).collect();
    let mut cache = HashMap::new();
    let scored = score_nodes(&g, &term_refs, &mut cache);
    let seeds = pick_seeds_diverse(&scored, 3, 0.2, &g, &term_refs, &mut cache);
    assert!(seeds.contains(&"auth".to_string()), "seeds: {seeds:?}");
    assert!(!seeds.contains(&"cfg".to_string()), "seeds: {seeds:?}");
    assert!(!seeds.contains(&"sec".to_string()), "seeds: {seeds:?}");
}

// ── _normalize_context_filters alias resolution ──────────────────────────────

#[test]
fn test_normalize_context_filters_resolves_aliases() {
    let cases = [
        ("param", "parameter_type"),
        ("params", "parameter_type"),
        ("parameter", "parameter_type"),
        ("parameters", "parameter_type"),
        ("argument", "parameter_type"),
        ("arguments", "parameter_type"),
        ("arg", "parameter_type"),
        ("args", "parameter_type"),
        ("return", "return_type"),
        ("returns", "return_type"),
        ("returned", "return_type"),
        ("generic", "generic_arg"),
        ("generics", "generic_arg"),
        ("template", "generic_arg"),
        ("templates", "generic_arg"),
        ("annotation", "attribute"),
        ("annotations", "attribute"),
        ("decorator", "attribute"),
        ("decorators", "attribute"),
        ("calls", "call"),
        ("called", "call"),
        ("invoke", "call"),
        ("invocation", "call"),
        ("fields", "field"),
        ("property", "field"),
        ("properties", "field"),
        ("member", "field"),
        ("members", "field"),
        ("imports", "import"),
        ("imported", "import"),
        ("module", "import"),
        ("modules", "import"),
        ("exports", "export"),
        ("exported", "export"),
    ];
    for (input, expected) in cases {
        assert_eq!(
            normalize_context_filters(&[input.to_string()]),
            vec![expected.to_string()],
            "alias {input:?} should resolve to {expected:?}"
        );
    }
}

#[test]
fn test_normalize_context_filters_passes_through_canonical() {
    assert_eq!(
        normalize_context_filters(&["parameter_type".to_string()]),
        vec!["parameter_type".to_string()]
    );
    assert_eq!(
        normalize_context_filters(&["field".to_string()]),
        vec!["field".to_string()]
    );
}

#[test]
fn test_normalize_context_filters_is_case_insensitive() {
    // Mixed casing of the same alias should fold to a single canonical entry.
    assert_eq!(
        normalize_context_filters(&["PARAM".to_string(), "param".to_string()]),
        vec!["parameter_type".to_string()]
    );
}

#[test]
fn test_normalize_context_filters_deduplicates_aliases() {
    // Three different surface forms collapse to the same canonical name and
    // appear only once in the result.
    assert_eq!(
        normalize_context_filters(&[
            "param".to_string(),
            "parameter".to_string(),
            "arg".to_string(),
        ]),
        vec!["parameter_type".to_string()]
    );
}

#[test]
fn test_normalize_context_filters_trims_whitespace() {
    assert_eq!(
        normalize_context_filters(&["  return  ".to_string()]),
        vec!["return_type".to_string()]
    );
}

#[test]
fn test_normalize_context_filters_skips_empty_and_whitespace_only() {
    // Empty strings and whitespace-only strings collapse to no entries — the
    // canonical-name list must not be polluted by `--context ""` or
    // `--context "   "`.
    assert_eq!(
        normalize_context_filters(&[String::new(), "  ".to_string()]),
        Vec::<String>::new()
    );
}

// ── _infer_context_filters ────────────────────────────────────────────────────

#[test]
fn test_infer_context_filters_for_calls_question() {
    assert_eq!(
        infer_context_filters("who calls extract"),
        vec!["call".to_string()]
    );
}

// ── _resolve_context_filters ──────────────────────────────────────────────────

#[test]
fn test_resolve_context_filters_explicit_overrides_heuristic() {
    let explicit = vec!["field".to_string()];
    let (filters, source) = resolve_context_filters("who calls extract", Some(&explicit));
    assert_eq!(filters, vec!["field".to_string()]);
    assert_eq!(source, Some("explicit".to_string()));
}

// ── _bfs ─────────────────────────────────────────────────────────────────────

#[test]
fn test_bfs_depth_1() {
    let g = make_graph();
    let (visited, _edges) = bfs(&g, &["n1".to_string()], 1);
    assert!(visited.contains("n1"));
    assert!(visited.contains("n2")); // direct neighbor
    assert!(!visited.contains("n3")); // 2 hops away
}

#[test]
fn test_bfs_depth_2() {
    let g = make_graph();
    let (visited, _edges) = bfs(&g, &["n1".to_string()], 2);
    assert!(visited.contains("n3")); // n1 -> n2 -> n3
}

#[test]
fn test_bfs_disconnected() {
    let g = make_graph();
    let (visited, _edges) = bfs(&g, &["n5".to_string()], 3);
    // isolated node — only itself
    assert_eq!(visited.len(), 1);
    assert!(visited.contains("n5"));
}

#[test]
fn test_bfs_returns_edges() {
    let g = make_graph();
    let (_, edges) = bfs(&g, &["n1".to_string()], 1);
    assert!(!edges.is_empty());
    assert!(edges.iter().any(|(u, v)| u == "n1" || v == "n1"));
}

// ── _filter_graph_by_context ──────────────────────────────────────────────────

#[test]
fn test_filter_graph_by_context_limits_traversal() {
    let g = make_graph();
    let filters = vec!["call".to_string()];
    let filtered = filter_graph_by_context(&g, Some(&filters));
    let (visited, edges) = bfs(&filtered, &["n1".to_string()], 2);
    assert!(visited.contains("n2"));
    assert!(!visited.contains("n3"));
    assert_eq!(edges, vec![("n1".to_string(), "n2".to_string())]);
}

// ── _dfs ─────────────────────────────────────────────────────────────────────

#[test]
fn test_dfs_depth_1() {
    let g = make_graph();
    let (visited, _edges) = dfs(&g, &["n1".to_string()], 1);
    assert!(visited.contains("n1"));
    assert!(visited.contains("n2"));
    assert!(!visited.contains("n3"));
}

#[test]
fn test_dfs_full_chain() {
    let g = make_graph();
    let (visited, _edges) = dfs(&g, &["n1".to_string()], 5);
    for n in ["n1", "n2", "n3", "n4"] {
        assert!(visited.contains(n), "expected {n} in visited");
    }
}

// ── _subgraph_to_text ─────────────────────────────────────────────────────────

#[test]
fn test_subgraph_to_text_contains_labels() {
    let g = make_graph();
    let nodes: std::collections::HashSet<String> =
        ["n1".to_string(), "n2".to_string()].into_iter().collect();
    let text = subgraph_to_text(
        &g,
        &nodes,
        &[("n1".to_string(), "n2".to_string())],
        2000,
        None,
    );
    assert!(text.contains("extract"));
    assert!(text.contains("cluster"));
}

#[test]
fn test_subgraph_to_text_prefers_community_name() {
    // A node carrying `community_name` renders the human label; a node without
    // it falls back to the numeric `community` id. Mirrors graphify-py
    // `str(d.get('community_name') or d.get('community', ''))` (#1305).
    let g = build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "extract", "source_file": "extract.py",
                 "source_location": "L10", "community": 7, "community_name": "Auth Layer"},
                {"id": "n2", "label": "cluster", "source_file": "cluster.py",
                 "source_location": "L5", "community": 7},
            ],
            "edges": [
                {"source": "n1", "target": "n2", "relation": "calls",
                 "confidence": "INFERRED"}
            ]
        }),
        false,
        None,
    )
    .expect("build graph");
    let nodes: std::collections::HashSet<String> =
        ["n1".to_string(), "n2".to_string()].into_iter().collect();
    let text = subgraph_to_text(&g, &nodes, &[], 2000, None);
    // community_name wins for n1 (human label, not the numeric cid).
    assert!(
        text.contains("NODE extract [src=extract.py loc=L10 community=Auth Layer]"),
        "named community not rendered: {text}"
    );
    // n2 has no community_name -> falls back to the numeric community id.
    assert!(
        text.contains("NODE cluster [src=cluster.py loc=L5 community=7]"),
        "numeric community fallback missing: {text}"
    );
}

#[test]
fn test_subgraph_to_text_string_community_renders_unquoted() {
    // A `community` stored as a JSON string must render like Python's
    // `str("7")` -> `7`, not the quoted `"7"` that `Value::to_string` emits.
    let g = build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "extract", "source_file": "extract.py",
                 "source_location": "L10", "community": "7"},
            ],
            "edges": []
        }),
        false,
        None,
    )
    .expect("build graph");
    let nodes: std::collections::HashSet<String> = ["n1".to_string()].into_iter().collect();
    let text = subgraph_to_text(&g, &nodes, &[], 2000, None);
    assert!(
        text.contains("NODE extract [src=extract.py loc=L10 community=7]"),
        "string community should render unquoted: {text}"
    );
    assert!(
        !text.contains("community=\"7\""),
        "string community must not be JSON-quoted: {text}"
    );
}

#[test]
fn test_subgraph_to_text_truncates() {
    let g = make_graph();
    let nodes: std::collections::HashSet<String> = ["n1", "n2", "n3", "n4"]
        .iter()
        .map(|&s| s.to_string())
        .collect();
    // Very small budget forces truncation.
    let text = subgraph_to_text(&g, &nodes, &[("n1".to_string(), "n2".to_string())], 1, None);
    assert!(text.contains("truncated"));
}

#[test]
fn test_subgraph_to_text_edge_included() {
    let g = make_graph();
    let nodes: std::collections::HashSet<String> =
        ["n1".to_string(), "n2".to_string()].into_iter().collect();
    let text = subgraph_to_text(
        &g,
        &nodes,
        &[("n1".to_string(), "n2".to_string())],
        2000,
        None,
    );
    assert!(text.contains("EDGE"));
    assert!(text.contains("calls"));
}

#[test]
fn test_subgraph_to_text_includes_edge_context() {
    let g = make_graph();
    let nodes: std::collections::HashSet<String> =
        ["n1".to_string(), "n2".to_string()].into_iter().collect();
    let text = subgraph_to_text(
        &g,
        &nodes,
        &[("n1".to_string(), "n2".to_string())],
        2000,
        None,
    );
    assert!(text.contains("context=call"));
}

// ── _query_graph_text ─────────────────────────────────────────────────────────

#[test]
fn test_query_graph_text_explicit_context_filter_changes_traversal() {
    let g = make_graph();
    let mut cache = HashMap::new();
    let filters = vec!["call".to_string()];
    let text = query_graph_text(&g, "extract", "bfs", 2, 2000, Some(&filters), &mut cache);
    assert!(text.contains("Context: call (explicit)"));
    assert!(text.contains("cluster"));
    assert!(!text.contains("build"));
}

#[test]
fn test_query_graph_text_heuristic_context_filter_changes_traversal() {
    let g = make_graph();
    let mut cache = HashMap::new();
    let text = query_graph_text(&g, "who calls extract", "bfs", 2, 2000, None, &mut cache);
    assert!(text.contains("Context: call (heuristic)"));
    assert!(text.contains("cluster"));
    assert!(!text.contains("build"));
}

// ── _load_graph ───────────────────────────────────────────────────────────────

#[test]
fn test_load_graph_roundtrip() {
    let tmp = tempdir().expect("tempdir");
    let p = tmp.path().join("graph.json");

    // Write a minimal node-link JSON.
    let data = json!({
        "directed": true,
        "nodes": [
            {"id": "n1", "label": "a"},
            {"id": "n2", "label": "b"},
            {"id": "n3", "label": "c"},
            {"id": "n4", "label": "d"},
            {"id": "n5", "label": "e"}
        ],
        "links": [
            {"source": "n1", "target": "n2"},
            {"source": "n2", "target": "n3"},
            {"source": "n3", "target": "n4"}
        ]
    });
    std::fs::write(&p, serde_json::to_string(&data).expect("json")).expect("write");
    let g = load_graph(p.to_str().expect("str")).expect("load");
    assert_eq!(g.node_count(), 5);
    assert_eq!(g.edge_count(), 3);
}

#[test]
fn test_load_graph_missing_file() {
    let tmp = tempdir().expect("tempdir");
    let p = tmp.path().join("graphify-out").join("nonexistent.json");
    // Should return Err, not panic.
    assert!(load_graph(p.to_str().expect("str")).is_err());
}

// ── Hot-reload (issue #874) ───────────────────────────────────────────────────

fn write_graph(path: &std::path::Path, node_ids: &[&str]) {
    let nodes: Vec<_> = node_ids
        .iter()
        .map(|id| json!({"id": id, "label": id, "community": 0}))
        .collect();
    let data = json!({"directed": true, "nodes": nodes, "links": []});
    std::fs::write(path, serde_json::to_string(&data).expect("json")).expect("write");
}

#[test]
fn test_maybe_reload_reloads_and_clears_idf_cache() {
    // A graph-file change triggers a reload that also clears the IDF cache (its
    // per-term weights are stale once the graph is replaced); an unchanged file
    // is a no-op that keeps the cache.
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graphify-out");
    std::fs::create_dir_all(&out).expect("mkdir");
    let path = out.join("graph.json");
    let path_s = path.to_str().expect("str").to_string();

    write_graph(&path, &["alpha", "beta"]);
    let mut graph = load_graph(&path_s).expect("load");
    let mut communities = graphify_serve::graph::communities_from_graph(&graph);
    let meta = std::fs::metadata(&path).expect("stat");
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| {
            u64::from(d.subsec_nanos()) + d.as_secs() * 1_000_000_000
        });
    let mut reload_state = graphify_serve::ReloadState {
        mtime_ns,
        size: meta.len(),
    };
    // Prime the IDF cache the way a query would.
    let mut idf_cache = HashMap::new();
    idf_cache.insert("alpha".to_string(), 1.23_f64);

    // Unchanged file: no reload, cache retained.
    assert!(
        !graphify_serve::tools::maybe_reload(
            &path_s,
            &mut graph,
            &mut communities,
            &mut reload_state,
            &mut idf_cache
        ),
        "an unchanged file must not reload"
    );
    assert!(idf_cache.contains_key("alpha"), "no reload keeps the cache");

    // Change the file: reload fires and clears the stale cache.
    std::thread::sleep(std::time::Duration::from_millis(10));
    write_graph(&path, &["alpha", "beta", "gamma"]);
    assert!(
        graphify_serve::tools::maybe_reload(
            &path_s,
            &mut graph,
            &mut communities,
            &mut reload_state,
            &mut idf_cache
        ),
        "a changed file must reload"
    );
    assert!(
        idf_cache.is_empty(),
        "a successful reload MUST clear the stale IDF cache"
    );
    let ids: Vec<_> = graph.nodes().map(|(id, _)| id.clone()).collect();
    assert!(
        ids.contains(&"gamma".to_string()),
        "the reloaded graph has the new node"
    );
}

#[test]
fn test_load_graph_cache_key_changes_with_content() {
    let tmp = tempdir().expect("tempdir");
    let out = tmp.path().join("graphify-out");
    std::fs::create_dir_all(&out).expect("mkdir");
    let path = out.join("graph.json");

    write_graph(&path, &["a"]);
    let m1 = std::fs::metadata(&path).expect("stat1");
    let key1 = (m1.modified().expect("mtime1"), m1.len());

    std::thread::sleep(std::time::Duration::from_millis(10));
    write_graph(&path, &["a", "b"]);
    let m2 = std::fs::metadata(&path).expect("stat2");
    let key2 = (m2.modified().expect("mtime2"), m2.len());

    assert_ne!(key1, key2, "stat key must change when file content changes");
}

// ── IDF weighting tests (issue #897) ─────────────────────────────────────────

#[test]
fn test_idf_downweights_common_terms() {
    let g = make_noisy_graph();
    let mut cache = HashMap::new();
    // "foobarservice" matches 1 node; "error" matches 20 → IDF should make fbs rank first.
    let scored = score_nodes(&g, &["foobarservice", "error"], &mut cache);
    assert!(!scored.is_empty(), "should have results");
    assert_eq!(
        scored[0].1, "fbs",
        "FooBarService should rank first, got {}",
        scored[0].1
    );
}

#[test]
fn test_idf_cached_on_graph() {
    // Calling score_nodes should populate the IDF cache.
    let g = make_graph();
    let mut cache = HashMap::new();
    let _ = score_nodes(&g, &["extract"], &mut cache);
    assert!(
        cache.contains_key("extract"),
        "IDF cache should contain 'extract'"
    );
}

#[test]
fn test_idf_new_graph_starts_fresh() {
    let g1 = make_graph();
    let g2 = make_graph();
    let mut cache1 = HashMap::new();
    let mut cache2 = HashMap::new();
    let _ = score_nodes(&g1, &["extract"], &mut cache1);
    // g2 has its own separate cache — not shared.
    assert!(!cache2.contains_key("extract"));
    // After scoring g2, cache2 should be populated independently.
    let _ = score_nodes(&g2, &["extract"], &mut cache2);
    assert!(cache2.contains_key("extract"));
    let _ = g2; // suppress unused warning
}

#[test]
fn test_idf_rare_term_gets_high_weight() {
    let g = make_graph(); // 5 nodes
    let mut cache = HashMap::new();
    let idf = compute_idf(&g, &["extract"], &mut cache);
    // extract matches only n1: IDF = ln(1 + 5/2) ≈ 1.25
    assert!(idf["extract"] > 1.0, "rare term IDF should be > 1.0");
}

#[test]
fn test_idf_common_term_gets_low_weight() {
    // 'handle' in every node label → very low IDF.
    let mut nodes = vec![];
    for i in 0..20_u64 {
        nodes.push(json!({
            "id": format!("n{i}"),
            "label": format!("handle_{i}"),
            "source_file": format!("f{i}.py")
        }));
    }
    let g = build_from_json(json!({"nodes": nodes, "edges": []}), false, None).expect("graph");
    let mut cache = HashMap::new();
    let idf = compute_idf(&g, &["handle"], &mut cache);
    assert!(idf["handle"] < 1.0, "common term IDF should be < 1.0");
}

// ── _pick_seeds (issue #897) ──────────────────────────────────────────────────

#[test]
fn test_pick_seeds_dominant_identifier_gives_one_seed() {
    let scored = vec![
        (1000.0_f64, "fbs".to_string()),
        (1.0, "err1".to_string()),
        (0.9, "err2".to_string()),
    ];
    let seeds = pick_seeds(&scored, 3, 0.2);
    assert_eq!(seeds, vec!["fbs".to_string()]);
}

#[test]
fn test_pick_seeds_close_scores_keeps_multiple() {
    let scored = vec![
        (10.0_f64, "a".to_string()),
        (9.0, "b".to_string()),
        (8.5, "c".to_string()),
    ];
    let seeds = pick_seeds(&scored, 3, 0.2);
    assert_eq!(seeds.len(), 3);
}

#[test]
fn test_pick_seeds_empty() {
    let seeds = pick_seeds(&[], 3, 0.2);
    assert!(seeds.is_empty());
}

#[test]
fn test_pick_seeds_single() {
    let scored = vec![(5.0_f64, "x".to_string())];
    let seeds = pick_seeds(&scored, 3, 0.2);
    assert_eq!(seeds, vec!["x".to_string()]);
}

#[test]
fn test_pick_seeds_respects_max_k() {
    let scored: Vec<(f64, String)> = (0..10).map(|i| (10.0, format!("n{i}"))).collect();
    let seeds = pick_seeds(&scored, 3, 0.2);
    assert_eq!(seeds.len(), 3);
}

#[test]
fn test_pick_seeds_without_diversity_args_is_unchanged() {
    // pick_seeds (no graph/terms) keeps the pre-#1445 gap-cutoff behavior.
    let scored = vec![
        (1000.0_f64, "fbs".to_string()),
        (1.0, "err1".to_string()),
        (0.9, "err2".to_string()),
    ];
    assert_eq!(pick_seeds(&scored, 3, 0.2), vec!["fbs".to_string()]);
}

#[test]
fn test_pick_seeds_diversity_recovers_starved_term() {
    // #1445: one term's incidental EXACT match ("unrelated") outscores the
    // substring match on the relevant term ("widget" → rate_limit_widget) by
    // ~1000x, so the 20%-gap cutoff drops the relevant node. pick_seeds_diverse
    // guarantees a per-term seed, recovering it. Directed fixture mirrors Python.
    let mut g = Graph::new(GraphKind::DiGraph);
    let mk = |label: &str, src: &str| {
        let mut a: indexmap::IndexMap<String, serde_json::Value> = indexmap::IndexMap::new();
        a.insert("label".to_string(), json!(label));
        a.insert("source_file".to_string(), json!(src));
        a
    };
    g.add_node("noise", mk("unrelated", "design_tokens.json"));
    g.add_node("target", mk("rate_limit_widget", "src/widget.py"));
    g.add_node("other", mk("something_else", "src/other.py"));
    g.add_edge("other", "target", indexmap::IndexMap::new());

    let mut cache = HashMap::new();
    let terms = ["unrelated", "widget"];
    let scored = score_nodes(&g, &terms, &mut cache);
    // Premise: without diversity, only the exact match survives the gap cutoff.
    assert_eq!(pick_seeds(&scored, 3, 0.2), vec!["noise".to_string()]);
    let after = pick_seeds_diverse(&scored, 3, 0.2, &g, &terms, &mut cache);
    assert!(
        after.contains(&"noise".to_string()),
        "exact match kept: {after:?}"
    );
    assert!(
        after.contains(&"target".to_string()),
        "starved term recovered: {after:?}"
    );
}

#[test]
fn test_pick_seeds_dedups_homonymous_generic_labels() {
    // #1766: many nodes sharing one generic label (framework `GET` handlers)
    // contribute at most ONE seed, not consume every slot. A distinct, relevant
    // label still gets its own seed.
    let mut g = Graph::new(GraphKind::DiGraph);
    let mk = |label: &str, src: &str| {
        let mut a: indexmap::IndexMap<String, serde_json::Value> = indexmap::IndexMap::new();
        a.insert("label".to_string(), json!(label));
        a.insert("source_file".to_string(), json!(src));
        a
    };
    let mut scored: Vec<(f64, String)> = Vec::new();
    for i in 0..5 {
        g.add_node(&format!("get{i}"), mk("GET", &format!("routes/r{i}.py")));
        scored.push((1000.0, format!("get{i}")));
    }
    g.add_node("um", mk("users_model", "models/users.py"));
    scored.push((900.0, "um".to_string()));

    let mut cache = HashMap::new();
    let seeds = pick_seeds_diverse(&scored, 3, 0.2, &g, &[], &mut cache);
    let get_seeds: Vec<&String> = seeds.iter().filter(|s| s.starts_with("get")).collect();
    assert_eq!(
        get_seeds.len(),
        1,
        "expected one GET representative, got {get_seeds:?}"
    );
    assert!(
        seeds.contains(&"um".to_string()),
        "distinct label starved out: {seeds:?}"
    );
}

#[test]
fn test_pick_seeds_dedup_key_is_case_and_diacritic_normalized() {
    // #1766: `GET`/`Get`/`get` are the same generic label and dedup together.
    let mut g = Graph::new(GraphKind::DiGraph);
    let mk = |label: &str, src: &str| {
        let mut a: indexmap::IndexMap<String, serde_json::Value> = indexmap::IndexMap::new();
        a.insert("label".to_string(), json!(label));
        a.insert("source_file".to_string(), json!(src));
        a
    };
    g.add_node("a", mk("GET", "a.py"));
    g.add_node("b", mk("Get", "b.py"));
    g.add_node("c", mk("get", "c.py"));
    let scored = vec![
        (1000.0_f64, "a".to_string()),
        (990.0, "b".to_string()),
        (980.0, "c".to_string()),
    ];
    let mut cache = HashMap::new();
    let seeds = pick_seeds_diverse(&scored, 3, 0.2, &g, &[], &mut cache);
    assert_eq!(
        seeds.len(),
        1,
        "case-variant duplicates not collapsed: {seeds:?}"
    );
}

#[test]
fn test_pick_seeds_per_term_guarantee_does_not_reintroduce_generic_dupe() {
    // #1766: the per-term guarantee loop honors the same per-label cap, so it
    // can't add a second `GET` after dedup already seeded one.
    let mut g = Graph::new(GraphKind::DiGraph);
    let mk = |label: &str, src: &str| {
        let mut a: indexmap::IndexMap<String, serde_json::Value> = indexmap::IndexMap::new();
        a.insert("label".to_string(), json!(label));
        a.insert("source_file".to_string(), json!(src));
        a
    };
    for i in 0..3 {
        g.add_node(&format!("get{i}"), mk("GET", &format!("r{i}.py")));
    }
    g.add_node("um", mk("users_model", "users.py"));
    g.add_edge("um", "get0", indexmap::IndexMap::new());

    let mut cache = HashMap::new();
    let terms = ["get", "users"];
    let scored = score_nodes(&g, &terms, &mut cache);
    let seeds = pick_seeds_diverse(&scored, 3, 0.2, &g, &terms, &mut cache);
    let get_seeds: Vec<&String> = seeds.iter().filter(|s| s.starts_with("get")).collect();
    assert_eq!(
        get_seeds.len(),
        1,
        "per-term guarantee reintroduced a GET dupe: {seeds:?}"
    );
}

#[test]
fn test_score_nodes_scores_identical_labels_equally() {
    // #1766 followup: the per-label multiplicity penalty must NOT leak into
    // score_nodes (shared by path/explain endpoint resolution). Two nodes with
    // the SAME label receive the SAME score — the fix lives in seed selection.
    let mut g = Graph::new(GraphKind::DiGraph);
    let mk = |label: &str, src: &str| {
        let mut a: indexmap::IndexMap<String, serde_json::Value> = indexmap::IndexMap::new();
        a.insert("label".to_string(), json!(label));
        a.insert("source_file".to_string(), json!(src));
        a
    };
    g.add_node("g1", mk("GET", "a.py"));
    g.add_node("g2", mk("GET", "b.py"));
    g.add_node("g3", mk("GET", "c.py"));
    let mut cache = HashMap::new();
    let by_id: std::collections::HashMap<String, f64> = score_nodes(&g, &["get"], &mut cache)
        .into_iter()
        .map(|(s, nid)| (nid, s))
        .collect();
    assert_eq!(
        by_id["g1"].total_cmp(&by_id["g2"]),
        std::cmp::Ordering::Equal,
        "identical labels scored differently: {by_id:?}"
    );
    assert_eq!(
        by_id["g2"].total_cmp(&by_id["g3"]),
        std::cmp::Ordering::Equal,
        "identical labels scored differently: {by_id:?}"
    );
}

// ── Truncation hint (issue #897) ──────────────────────────────────────────────

#[test]
fn test_subgraph_to_text_truncation_hint_is_actionable() {
    let g = make_graph();
    let nodes: std::collections::HashSet<String> = ["n1", "n2", "n3", "n4"]
        .iter()
        .map(|&s| s.to_string())
        .collect();
    let text = subgraph_to_text(&g, &nodes, &[("n1".to_string(), "n2".to_string())], 1, None);
    assert!(text.contains("truncated"));
    assert!(
        text.contains("get_node") || text.contains("context_filter"),
        "truncation hint should tell user what to do"
    );
}

// ── Integration: identifier + noise (issue #897) ──────────────────────────────

#[test]
fn test_query_seeds_from_identifier_not_noise() {
    let g = make_noisy_graph();
    let mut cache = HashMap::new();
    let text = query_graph_text(
        &g,
        "FooBarService error handling",
        "bfs",
        2,
        2000,
        None,
        &mut cache,
    );
    assert!(
        text.contains("FooBarService"),
        "FooBarService should appear in results"
    );
    assert!(
        text.contains("ServiceClient"),
        "ServiceClient should appear as neighbor"
    );
}

// ── PR tool tests ─────────────────────────────────────────────────────────────

fn make_fake_gh() -> FakeGhClient {
    FakeGhClient {
        prs_json: CANNED_PR_JSON,
        files: vec!["src/feature_x.rs".to_string()],
        default_branch: Some("main".to_string()),
    }
}

#[test]
fn test_tool_list_prs_returns_pr_descriptors() {
    let gh = make_fake_gh();
    let result =
        tool_list_prs_with_clients(&json!({}), &gh, &FakeGitClient).expect("test invariant");
    let prs = result["prs"].as_array().expect("array field");
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0]["number"], 42);
    assert_eq!(prs[0]["title"], "Add feature X");
    assert_eq!(prs[0]["author"], "alice");
}

#[test]
fn test_tool_list_prs_includes_count() {
    let gh = make_fake_gh();
    let result =
        tool_list_prs_with_clients(&json!({}), &gh, &FakeGitClient).expect("test invariant");
    assert_eq!(result["count"], 1);
}

#[test]
fn test_tool_list_prs_handles_empty() {
    let gh = FakeGhClient {
        prs_json: "[]",
        files: vec![],
        default_branch: Some("main".to_string()),
    };
    let result =
        tool_list_prs_with_clients(&json!({}), &gh, &FakeGitClient).expect("test invariant");
    let prs = result["prs"].as_array().expect("array field");
    assert!(prs.is_empty());
    assert_eq!(result["count"], 0);
}

/// Minimal graph with one node whose `source_file` matches the PR's changed file.
fn make_impact_graph() -> Graph {
    build_from_json(
        json!({
            "nodes": [
                {"id": "n1", "label": "feature_x", "source_file": "src/feature_x.rs", "community": 0}
            ],
            "edges": []
        }),
        true,
        None,
    )
    .expect("make_impact_graph")
}

#[test]
fn test_tool_get_pr_impact_lists_affected_nodes() {
    let gh = make_fake_gh();
    let graph = make_impact_graph();
    let args = json!({"pr_number": 42});
    let result = tool_get_pr_impact_with_clients(&graph, &args, &gh).expect("test invariant");
    assert!(
        result["affected_nodes"].as_u64().expect("u64 field") > 0,
        "must report affected nodes when file matches"
    );
}

#[test]
fn test_tool_get_pr_impact_empty_when_no_match() {
    let gh = FakeGhClient {
        prs_json: CANNED_PR_JSON,
        files: vec!["other/unrelated.rs".to_string()],
        default_branch: Some("main".to_string()),
    };
    let graph = make_impact_graph();
    let args = json!({"pr_number": 42});
    let result = tool_get_pr_impact_with_clients(&graph, &args, &gh).expect("test invariant");
    assert_eq!(
        result["affected_nodes"].as_u64().expect("u64 field"),
        0,
        "no overlap → zero affected nodes"
    );
}

#[test]
fn test_tool_triage_prs_returns_structured_output() {
    let gh = make_fake_gh();
    let result =
        tool_triage_prs_with_clients(&json!({}), &gh, &FakeGitClient).expect("test invariant");
    assert!(result.is_array(), "triage output must be a JSON array");
}

#[test]
fn test_tool_triage_prs_respects_limit() {
    // Only 1 PR in canned data; limit=1 should not change anything, but the
    // field must be respected (no more than `limit` items returned).
    let gh = make_fake_gh();
    let args = json!({"limit": 1});
    let result = tool_triage_prs_with_clients(&args, &gh, &FakeGitClient).expect("test invariant");
    let items = result.as_array().expect("array field");
    assert!(
        items.len() <= 1,
        "limit=1 must cap the result length; got {}",
        items.len()
    );
}

// ---------------------------------------------------------------------------
// query_terms: keep short non-English tokens (#964)
// ---------------------------------------------------------------------------

#[test]
fn query_terms_filters_only_short_english_terms() {
    assert_eq!(
        query_terms("the quick brown"),
        vec!["quick", "brown"] // "the" is a question/filler stopword, dropped
    );
    let r = query_terms("an ai bot");
    assert_eq!(r, vec!["bot"]);
}

#[test]
fn query_terms_keeps_short_non_english_terms() {
    let r = query_terms("認証");
    assert_eq!(r, vec!["認証"]);
}

#[test]
fn query_terms_lowercases() {
    let r = query_terms("AuthN AuthZ");
    assert_eq!(r, vec!["authn", "authz"]);
}

// ---------------------------------------------------------------------------
// query_terms: Chinese segmentation (#1026)
//
// Ports graphify-py `tests/test_serve.py` Chinese segmentation cases that
// exercise the fallback path (the Rust port ships without `jieba`, so
// bigram fallback is the only segmentation path; tests that assert
// dictionary-quality cuts like `["包", "管理器"]` from `"包管理器"` would
// require jieba and are intentionally not ported).
// ---------------------------------------------------------------------------

#[test]
fn query_terms_chinese_mixed_falls_back_to_bigrams() {
    // Mixed Chinese + English input: the English terms come through as-is
    // (short stopwords dropped), Chinese sub-strings split into character
    // bigrams plus the original term. "前端" (2 chars) yields itself.
    let r = query_terms("前端 router 路由配置");
    assert!(r.iter().any(|t| t == "前端"));
    assert!(r.iter().any(|t| t == "router"));
    assert!(r.iter().any(|t| t == "路由"));
    assert!(r.iter().any(|t| t == "配置"));
}

#[test]
fn query_terms_non_chinese_scripts_are_not_segmented() {
    // Hiragana, Katakana, and Hangul live outside the CJK Unified
    // Ideographs range (U+4E00–U+9FFF) the segmenter keys on — they
    // pass through as a single search term.
    let r = query_terms("かなカナ한글");
    assert_eq!(r, vec!["かなカナ한글"]);
}

#[test]
fn query_terms_chinese_includes_original_term() {
    // The original 4-char string should still appear in the term list
    // alongside the bigrams so an exact-substring match against an
    // indexed label still resolves.
    let r = query_terms("页面路由");
    assert!(r.iter().any(|t| t == "页面"));
    assert!(r.iter().any(|t| t == "路由"));
    assert!(r.iter().any(|t| t == "页面路由"));
}

#[test]
fn query_terms_chinese_mixed_script_does_not_bigram_across_scripts() {
    // A mixed-script token like "a前b" must not produce noisy
    // cross-script bigrams ("a前", "前b") — only same-script bigrams
    // are emitted, and the unsegmented original term is preserved.
    // Divergence note: graphify-py's `_segment_chinese` bigram fallback
    // walks raw character pairs without script awareness; the Rust
    // port tightens this since the bigram path is its only segmenter.
    let r = query_terms("a前b");
    assert!(!r.iter().any(|t| t == "a前"));
    assert!(!r.iter().any(|t| t == "前b"));
    assert!(r.iter().any(|t| t == "a前b"));
}

// ---------------------------------------------------------------------------
// load_graph: reject oversized files
// ---------------------------------------------------------------------------

#[test]
fn test_load_graph_accepts_under_cap() {
    // Smoke test of the happy path: a tiny well-formed graph round-trips
    // through the size-cap-guarded loader. Boundary testing with a tiny
    // cap lives in graphify-security's parity suite where the
    // `_with(cap)` variant lets us trigger the error explicitly.
    let dir = tempdir().expect("tempdir");
    let graph_path = dir.path().join("graph.json");
    // Canonical NetworkX `node_link_data` shape — `links` not `edges`,
    // plus the `directed`/`multigraph` flags the loader inspects. Using
    // the same shape as `test_load_graph_roundtrip` so this test exercises
    // the real parse path rather than a degenerate minimal payload.
    std::fs::write(
        &graph_path,
        br#"{"directed": true, "multigraph": false, "nodes": [], "links": []}"#,
    )
    .expect("write");
    let result = load_graph(graph_path.to_str().expect("utf-8"));
    assert!(result.is_ok(), "small graph should load: {result:?}");
}

// --- #1441 work-memory overlay: query-text learning suffix --------------------

#[test]
fn test_subgraph_to_text_annotates_node_with_learning_status() {
    let mut g = make_graph();
    g.graph_attrs.insert(
        "_learning_overlay".to_string(),
        json!({ "n1": { "status": "preferred" } }),
    );
    let nodes: std::collections::HashSet<String> =
        ["n1".to_string(), "n2".to_string()].into_iter().collect();
    let text = subgraph_to_text(&g, &nodes, &[], 1000, None);
    let extract_line = text
        .lines()
        .find(|l| l.starts_with("NODE extract "))
        .expect("extract node line");
    assert!(
        extract_line.contains("learning=preferred]"),
        "{extract_line}"
    );
    let cluster_line = text
        .lines()
        .find(|l| l.starts_with("NODE cluster "))
        .expect("cluster node line");
    assert!(!cluster_line.contains("learning="), "{cluster_line}");
}

#[test]
fn test_subgraph_to_text_marks_stale_status() {
    let mut g = make_graph();
    g.graph_attrs.insert(
        "_learning_overlay".to_string(),
        json!({ "n1": { "status": "contested", "stale": true } }),
    );
    let nodes: std::collections::HashSet<String> = ["n1".to_string()].into_iter().collect();
    let text = subgraph_to_text(&g, &nodes, &[], 1000, None);
    assert!(text.contains("learning=contested:stale]"), "{text}");
}
