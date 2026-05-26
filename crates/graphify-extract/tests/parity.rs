//! Parity tests for `graphify-extract`.
//!
//! 1:1 ports of `graphify-py/tests/test_extract.py` and
//! `graphify-py/tests/test_astro_extraction.py`.

#![allow(clippy::expect_used)]

use std::path::{Path, PathBuf};

use graphify_extract::{
    extract, extract_astro, extract_bash, extract_js, extract_json, extract_python, file_stem,
    make_id, make_id1,
};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

// ── make_id ───────────────────────────────────────────────────────────────────

#[test]
fn make_id_strips_dots_and_underscores() {
    assert_eq!(make_id1("_auth"), "auth");
    assert_eq!(make_id(&[".httpx._client"]), "httpx_client");
}

#[test]
fn make_id_consistent() {
    assert_eq!(make_id(&["foo", "Bar"]), make_id(&["foo", "Bar"]));
}

#[test]
fn make_id_no_leading_trailing_underscores() {
    let result = make_id1("__init__");
    assert!(!result.starts_with('_'));
    assert!(!result.ends_with('_'));
}

// ── extract_python ────────────────────────────────────────────────────────────

#[test]
fn extract_python_finds_class() {
    let result = extract_python(&fixtures().join("sample.py"));
    let labels: Vec<_> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"Transformer"), "labels: {labels:?}");
}

#[test]
fn extract_python_finds_methods() {
    let result = extract_python(&fixtures().join("sample.py"));
    let labels: Vec<_> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(
        labels
            .iter()
            .any(|l| l.contains("__init__") || l.contains("forward")),
        "labels: {labels:?}"
    );
}

#[test]
fn extract_python_no_dangling_edges() {
    let result = extract_python(&fixtures().join("sample.py"));
    let node_ids: std::collections::HashSet<&str> =
        result.nodes.iter().map(|n| n.id.as_str()).collect();
    for edge in &result.edges {
        assert!(
            node_ids.contains(edge.source.as_str()),
            "Dangling source: {}",
            edge.source
        );
    }
}

#[test]
fn structural_edges_are_extracted() {
    let result = extract_python(&fixtures().join("sample.py"));
    let structural = ["contains", "method", "inherits", "imports", "imports_from"];
    for edge in &result.edges {
        if structural.contains(&edge.relation.as_str()) {
            assert_eq!(edge.confidence, "EXTRACTED", "Expected EXTRACTED: {edge:?}");
        }
    }
}

#[test]
fn extract_merges_multiple_files() {
    let files: Vec<PathBuf> = fixtures()
        .read_dir()
        .expect("read fixtures dir")
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("py"))
        .collect();
    assert!(!files.is_empty());
    let result = extract(&files, None);
    assert!(!result.nodes.is_empty());
    assert_eq!(result.input_tokens, 0);
}

#[test]
fn no_dangling_edges_on_extract() {
    let files: Vec<PathBuf> = fixtures()
        .read_dir()
        .expect("read fixtures dir")
        .filter_map(std::result::Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("py"))
        .collect();
    let result = extract(&files, None);
    let node_ids: std::collections::HashSet<String> = result
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(|v| v.as_str()).map(str::to_string))
        .collect();
    let internal_relations = ["contains", "method", "inherits", "calls"];
    for edge in &result.edges {
        let relation = edge.get("relation").and_then(|v| v.as_str()).unwrap_or("");
        if internal_relations.contains(&relation) {
            let src = edge.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let tgt = edge.get("target").and_then(|v| v.as_str()).unwrap_or("");
            assert!(node_ids.contains(src), "Dangling source: {edge:?}");
            assert!(node_ids.contains(tgt), "Dangling target: {edge:?}");
        }
    }
}

// ── Call-graph (sample_calls.py) ──────────────────────────────────────────────

#[test]
fn calls_edges_emitted() {
    let result = extract_python(&fixtures().join("sample_calls.py"));
    let calls: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .collect();
    assert!(!calls.is_empty(), "Expected at least one calls edge");
}

#[test]
fn calls_edges_are_extracted() {
    let result = extract_python(&fixtures().join("sample_calls.py"));
    for edge in result.edges.iter().filter(|e| e.relation == "calls") {
        assert_eq!(edge.confidence, "EXTRACTED");
        assert!((edge.weight - 1.0).abs() < f64::EPSILON);
    }
}

#[test]
fn python_call_edges_have_call_context() {
    let result = extract_python(&fixtures().join("sample_calls.py"));
    let call_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .collect();
    assert!(!call_edges.is_empty());
    for edge in &call_edges {
        assert_eq!(edge.context.as_deref(), Some("call"), "edge: {edge:?}");
    }
}

#[test]
fn calls_no_self_loops() {
    let result = extract_python(&fixtures().join("sample_calls.py"));
    for edge in result.edges.iter().filter(|e| e.relation == "calls") {
        assert_ne!(edge.source, edge.target, "Self-loop: {edge:?}");
    }
}

#[test]
fn run_analysis_calls_compute_score() {
    let result = extract_python(&fixtures().join("sample_calls.py"));
    let calls: std::collections::HashSet<(&str, &str)> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .map(|e| (e.source.as_str(), e.target.as_str()))
        .collect();
    let node_by_label: std::collections::HashMap<&str, &str> = result
        .nodes
        .iter()
        .map(|n| (n.label.as_str(), n.id.as_str()))
        .collect();
    let src = node_by_label.get("run_analysis()").copied();
    let tgt = node_by_label.get("compute_score()").copied();
    assert!(src.is_some(), "run_analysis node not found");
    assert!(tgt.is_some(), "compute_score node not found");
    assert!(
        calls.contains(&(src.expect("test invariant"), tgt.expect("test invariant"))),
        "run_analysis -> compute_score not found in {calls:?}"
    );
}

#[test]
fn run_analysis_calls_normalize() {
    let result = extract_python(&fixtures().join("sample_calls.py"));
    let calls: std::collections::HashSet<(&str, &str)> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .map(|e| (e.source.as_str(), e.target.as_str()))
        .collect();
    let node_by_label: std::collections::HashMap<&str, &str> = result
        .nodes
        .iter()
        .map(|n| (n.label.as_str(), n.id.as_str()))
        .collect();
    let src = node_by_label.get("run_analysis()").copied();
    let tgt = node_by_label.get("normalize()").copied();
    assert!(src.is_some() && tgt.is_some());
    assert!(calls.contains(&(src.expect("test invariant"), tgt.expect("test invariant"))));
}

#[test]
fn method_calls_module_function() {
    let result = extract_python(&fixtures().join("sample_calls.py"));
    let calls: std::collections::HashSet<(&str, &str)> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .map(|e| (e.source.as_str(), e.target.as_str()))
        .collect();
    let node_by_label: std::collections::HashMap<&str, &str> = result
        .nodes
        .iter()
        .map(|n| (n.label.as_str(), n.id.as_str()))
        .collect();
    let src = node_by_label.get(".process()").copied();
    let tgt = node_by_label.get("run_analysis()").copied();
    assert!(src.is_some() && tgt.is_some());
    assert!(calls.contains(&(src.expect("test invariant"), tgt.expect("test invariant"))));
}

#[test]
fn calls_deduplication() {
    let result = extract_python(&fixtures().join("sample_calls.py"));
    let call_pairs: Vec<(&str, &str)> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .map(|e| (e.source.as_str(), e.target.as_str()))
        .collect();
    let unique: std::collections::HashSet<_> = call_pairs.iter().copied().collect();
    assert_eq!(
        call_pairs.len(),
        unique.len(),
        "Duplicate calls edges found"
    );
}

#[test]
fn cross_file_calls_skip_ambiguous_duplicate_labels() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let caller = tmp.path().join("caller.py");
    let helper_a = tmp.path().join("a.py");
    let helper_b = tmp.path().join("b.py");
    std::fs::write(&caller, "def run():\n    log()\n").expect("test invariant");
    std::fs::write(&helper_a, "def log():\n    return 'a'\n").expect("test invariant");
    std::fs::write(&helper_b, "def log():\n    return 'b'\n").expect("test invariant");

    let result = extract(&[caller, helper_a, helper_b], Some(tmp.path()));
    let nodes: std::collections::HashMap<String, String> = result
        .nodes
        .iter()
        .filter_map(|n| {
            let id = n.get("id").and_then(|v| v.as_str())?.to_string();
            let label = n.get("label").and_then(|v| v.as_str())?.to_string();
            Some((id, label))
        })
        .collect();
    let inferred_calls: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("calls")
                && e.get("confidence").and_then(|v| v.as_str()) == Some("INFERRED")
        })
        .collect();
    // run() must not have an INFERRED call to log() since the name is ambiguous
    let ambiguous = inferred_calls.iter().any(|e| {
        let src_id = e.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let tgt_id = e.get("target").and_then(|v| v.as_str()).unwrap_or("");
        nodes.get(src_id).is_some_and(|l| l == "run()")
            && nodes.get(tgt_id).is_some_and(|l| l == "log()")
    });
    assert!(
        !ambiguous,
        "Ambiguous duplicate-label call should be skipped"
    );
}

// ── JS / TSX ──────────────────────────────────────────────────────────────────

#[test]
fn extract_js_destructured_require_imports_from() {
    let result = extract_js(&fixtures().join("cjs_require.js"));
    let imports_from: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports_from")
        .collect();
    let targets: Vec<_> = imports_from.iter().map(|e| e.target.as_str()).collect();
    assert!(
        targets.iter().any(|t| t.contains("foundation")),
        "No foundation import_from: {targets:?}"
    );
    assert!(
        targets.iter().any(|t| t.contains("utils")),
        "No utils import_from: {targets:?}"
    );
    assert!(
        targets.iter().any(|t| t.contains("helpers")),
        "No helpers import_from: {targets:?}"
    );
    for e in &imports_from {
        assert_eq!(e.confidence, "EXTRACTED");
    }
}

#[test]
fn extract_js_destructured_require_named_symbols() {
    let result = extract_js(&fixtures().join("cjs_require.js"));
    let sym_targets: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports")
        .map(|e| e.target.as_str())
        .collect();
    let foundation_stem = file_stem(&fixtures().join("foundation.js"));
    assert!(
        sym_targets.contains(&make_id(&[&foundation_stem, "loadFoundation"]).as_str()),
        "targets: {sym_targets:?}"
    );
    assert!(
        sym_targets.contains(&make_id(&[&foundation_stem, "validateConfig"]).as_str()),
        "targets: {sym_targets:?}"
    );
}

#[test]
fn extract_js_member_require_emits_property_symbol() {
    let result = extract_js(&fixtures().join("cjs_require.js"));
    let sym_targets: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports")
        .map(|e| e.target.as_str())
        .collect();
    let helpers_stem = file_stem(&fixtures().join("helpers.js"));
    assert!(
        sym_targets.contains(&make_id(&[&helpers_stem, "helperFn"]).as_str()),
        "targets: {sym_targets:?}"
    );
}

#[test]
fn extract_js_arrow_function_still_extracted() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let arrow_fixture = tmp.path().join("_arrow_only.js");
    std::fs::write(&arrow_fixture, "const greet = () => console.log('hi');\n")
        .expect("test invariant");
    let result = extract_js(&arrow_fixture);
    let labels: Vec<_> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains(&"greet()"), "labels: {labels:?}");
}

#[test]
fn cross_file_call_promoted_to_extracted_with_import_evidence() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let caller_path = tmp.path().join("caller.js");
    let callee_path = tmp.path().join("lib.js");
    std::fs::write(
        &caller_path,
        "const { doWork } = require('./lib');\nfunction run() { doWork(); }\n",
    )
    .expect("test invariant");
    std::fs::write(
        &callee_path,
        "function doWork() { return 1; }\nmodule.exports = { doWork };\n",
    )
    .expect("test invariant");
    let result = extract(&[caller_path, callee_path], Some(tmp.path()));
    let nodes: std::collections::HashMap<String, String> = result
        .nodes
        .iter()
        .filter_map(|n| {
            let id = n.get("id").and_then(|v| v.as_str())?.to_string();
            let label = n.get("label").and_then(|v| v.as_str())?.to_string();
            Some((id, label))
        })
        .collect();
    let call_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("calls") && {
                let src = e.get("source").and_then(|v| v.as_str()).unwrap_or("");
                let tgt = e.get("target").and_then(|v| v.as_str()).unwrap_or("");
                nodes.get(src).is_some_and(|l| l == "run()")
                    && nodes.get(tgt).is_some_and(|l| l == "doWork()")
            }
        })
        .collect();
    assert_eq!(
        call_edges.len(),
        1,
        "Expected exactly one run->doWork calls edge"
    );
    let confidence = call_edges[0]
        .get("confidence")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(confidence, "EXTRACTED");
    let score = call_edges[0]
        .get("confidence_score")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    assert!(
        (score - 1.0).abs() < f64::EPSILON,
        "confidence_score: {score}"
    );
}

#[test]
fn cross_file_call_remains_inferred_without_import_evidence() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let caller_path = tmp.path().join("caller.js");
    let callee_path = tmp.path().join("lib.js");
    std::fs::write(&caller_path, "function run() { doUnique(); }\n").expect("test invariant");
    std::fs::write(
        &callee_path,
        "function doUnique() { return 1; }\nmodule.exports = { doUnique };\n",
    )
    .expect("test invariant");
    let result = extract(&[caller_path, callee_path], Some(tmp.path()));
    let nodes: std::collections::HashMap<String, String> = result
        .nodes
        .iter()
        .filter_map(|n| {
            let id = n.get("id").and_then(|v| v.as_str())?.to_string();
            let label = n.get("label").and_then(|v| v.as_str())?.to_string();
            Some((id, label))
        })
        .collect();
    let call_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("calls") && {
                let src = e.get("source").and_then(|v| v.as_str()).unwrap_or("");
                let tgt = e.get("target").and_then(|v| v.as_str()).unwrap_or("");
                nodes.get(src).is_some_and(|l| l == "run()")
                    && nodes.get(tgt).is_some_and(|l| l == "doUnique()")
            }
        })
        .collect();
    assert_eq!(call_edges.len(), 1);
    let confidence = call_edges[0]
        .get("confidence")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert_eq!(confidence, "INFERRED");
}

// ── TSX ───────────────────────────────────────────────────────────────────────

#[test]
fn extract_tsx_finds_helpers_and_component() {
    let result = extract_js(&fixtures().join("sample.tsx"));
    let labels: Vec<_> = result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(
        labels.iter().any(|l| l.contains("fmtDate")),
        "fmtDate missing from {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.contains("fmtCount")),
        "fmtCount missing from {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l.contains("App")),
        "App missing from {labels:?}"
    );
}

#[test]
fn extract_tsx_jsx_expression_calls_resolve() {
    let result = extract_js(&fixtures().join("sample.tsx"));
    let nodes_by_id: std::collections::HashMap<&str, &str> = result
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), n.label.as_str()))
        .collect();
    let call_targets: std::collections::HashSet<&str> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .filter_map(|e| nodes_by_id.get(e.target.as_str()).copied())
        .collect();
    assert!(
        call_targets.contains("fmtDate()"),
        "fmtDate() call not captured. Targets: {call_targets:?}"
    );
    assert!(
        call_targets.contains("fmtCount()"),
        "fmtCount() call not captured. Targets: {call_targets:?}"
    );
}

// ── Bash extractor ────────────────────────────────────────────────────────────

#[test]
fn extract_bash_finds_functions() {
    let result = extract_bash(&fixtures().join("sample.sh"));
    assert!(result.error.is_none());
    let labels: std::collections::HashSet<&str> =
        result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains("build()"), "labels: {labels:?}");
    assert!(labels.contains("test_suite()"), "labels: {labels:?}");
    assert!(labels.contains("deploy()"), "labels: {labels:?}");
}

#[test]
fn extract_bash_emits_defines_edges() {
    let result = extract_bash(&fixtures().join("sample.sh"));
    let relations: std::collections::HashSet<&str> =
        result.edges.iter().map(|e| e.relation.as_str()).collect();
    assert!(relations.contains("defines"), "relations: {relations:?}");
}

#[test]
fn extract_bash_emits_calls_edges() {
    let result = extract_bash(&fixtures().join("sample.sh"));
    let calls: Vec<(&str, &str)> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .map(|e| (e.source.as_str(), e.target.as_str()))
        .collect();
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("deploy") && t.contains("build"))
    );
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("deploy") && t.contains("test_suite"))
    );
    assert!(
        calls
            .iter()
            .any(|(s, t)| s.contains("test_suite") && t.contains("build"))
    );
}

#[test]
fn extract_bash_calls_have_extracted_confidence() {
    let result = extract_bash(&fixtures().join("sample.sh"));
    for edge in result.edges.iter().filter(|e| e.relation == "calls") {
        assert_eq!(edge.confidence, "EXTRACTED");
        assert_eq!(edge.context.as_deref(), Some("call"));
    }
}

#[test]
fn extract_bash_emits_source_imports_from() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let helpers = tmp.path().join("helpers.sh");
    let script = tmp.path().join("deploy.sh");
    std::fs::write(&helpers, "# helper\n").expect("write fixture");
    std::fs::write(
        &script,
        "#!/bin/bash\nsource ./helpers.sh\nfoo() { echo hi; }\n",
    )
    .expect("test invariant");
    let result = extract_bash(&script);
    let import_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports_from")
        .collect();
    assert!(
        !import_edges.is_empty(),
        "Expected at least one imports_from edge"
    );
    assert_eq!(import_edges[0].context.as_deref(), Some("import"));
}

#[test]
fn extract_bash_creates_entrypoint_node() {
    // Every script gets a `<file>__entry` entrypoint node attached to the
    // file via `contains`. Mirrors the change in graphify-py `extract_bash`.
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = tmp.path().join("entry.sh");
    std::fs::write(&script, "#!/bin/bash\necho top\n").expect("write fixture");
    let result = extract_bash(&script);
    let entry_node = result
        .nodes
        .iter()
        .find(|n| n.id.ends_with("__entry"))
        .expect("bash entrypoint node should be present");
    assert_eq!(entry_node.label, "__entry__");
    let file_to_entry: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "contains" && e.target == entry_node.id)
        .collect();
    assert_eq!(
        file_to_entry.len(),
        1,
        "expected exactly one file->entry contains edge"
    );
}

#[test]
fn extract_bash_entrypoint_no_collision_with_function_named_script() {
    // The entrypoint ID is `<file_nid>__entry`, which must be distinct from
    // any function named `script` even when the file is `script.sh`.
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = tmp.path().join("script.sh");
    std::fs::write(&script, "#!/bin/bash\nscript() { echo s; }\n").expect("test invariant");
    let result = extract_bash(&script);
    let mut ids: Vec<&str> = result.nodes.iter().map(|n| n.id.as_str()).collect();
    ids.sort_unstable();
    let mut deduped = ids.clone();
    deduped.dedup();
    assert_eq!(ids, deduped, "ids should be unique: {ids:?}");
}

#[test]
fn extract_bash_rejects_command_substitution_as_call() {
    // `$(build)` inside a function body is shell expansion, not a real call.
    // The expansion-parent filter must skip it so no false `calls` edge is
    // emitted (#993).
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = tmp.path().join("expand.sh");
    std::fs::write(
        &script,
        "#!/bin/bash\nbuild() { echo b; }\ndeploy() { x=$(build); }\n",
    )
    .expect("test invariant");
    let result = extract_bash(&script);
    let calls: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .collect();
    assert!(
        calls.is_empty(),
        "command_substitution must not produce calls edges; got: {calls:?}"
    );
}

#[test]
fn extract_bash_process_substitution_not_recorded() {
    // `<(helper)` is process substitution — same expansion-parent filter
    // must skip it.
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = tmp.path().join("proc.sh");
    std::fs::write(
        &script,
        "#!/bin/bash\nhelper() { echo h; }\nrun() { diff <(helper) /dev/null; }\n",
    )
    .expect("test invariant");
    let result = extract_bash(&script);
    let calls: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .collect();
    assert!(
        calls.is_empty(),
        "process_substitution must not produce calls edges; got: {calls:?}"
    );
}

#[test]
fn extract_js_barrel_reexport_emits_re_exports_edges() {
    // `export { Foo, Bar } from './mod'` is a barrel re-export — graphify
    // emits one `re_exports` edge per specifier (with context="re-export")
    // and one `imports_from` edge to the source module.
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        tmp.path().join("mod.ts"),
        "export const Foo = 1;\nexport const Bar = 2;\n",
    )
    .expect("test invariant");
    let barrel = tmp.path().join("index.ts");
    std::fs::write(&barrel, "export { Foo, Bar } from './mod';\n").expect("write fixture");
    let result = extract_js(&barrel);
    let re_exports: Vec<&graphify_extract::types::Edge> = result
        .edges
        .iter()
        .filter(|e| e.relation == "re_exports")
        .collect();
    assert_eq!(
        re_exports.len(),
        2,
        "expected 2 re_exports edges (Foo, Bar): {:?}",
        result.edges
    );
    for e in &re_exports {
        assert_eq!(e.context.as_deref(), Some("re-export"));
        assert_eq!(e.confidence, "EXTRACTED");
    }
    let imports_from: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports_from")
        .collect();
    assert_eq!(
        imports_from.len(),
        1,
        "expected exactly one imports_from to './mod'"
    );
}

#[test]
fn extract_js_resolves_pnpm_workspace_package() {
    // Set up a minimal pnpm workspace and verify a bare `@scope/pkg`
    // import resolves to the package's entry-point file inside the
    // workspace rather than degrading to a bare-name hash.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    std::fs::write(
        root.join("pnpm-workspace.yaml"),
        "packages:\n  - 'packages/*'\n",
    )
    .expect("test invariant");
    std::fs::create_dir_all(root.join("packages/utils/src")).expect("test invariant");
    std::fs::create_dir_all(root.join("apps/api")).expect("test invariant");
    std::fs::write(
        root.join("packages/utils/package.json"),
        r#"{"name": "@scope/utils", "main": "src/index.ts"}"#,
    )
    .expect("test invariant");
    std::fs::write(
        root.join("packages/utils/src/index.ts"),
        "export const helper = 1;\n",
    )
    .expect("test invariant");
    let consumer = root.join("apps/api/main.ts");
    std::fs::write(&consumer, "import { helper } from '@scope/utils';\n").expect("write fixture");
    let result = extract_js(&consumer);
    let imports_from: Vec<&graphify_extract::types::Edge> = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports_from")
        .collect();
    assert!(
        !imports_from.is_empty(),
        "expected at least one imports_from edge: {:?}",
        result.edges
    );
    let any_resolved = imports_from
        .iter()
        .any(|e| e.target.contains("utils") || e.target.contains("index"));
    assert!(
        any_resolved,
        "at least one imports_from target should reference a resolved \
         workspace path; got: {:?}",
        imports_from.iter().map(|e| &e.target).collect::<Vec<_>>()
    );
}

#[test]
fn extract_ts_tsconfig_array_extends_alias_resolves_existing_ts_file() {
    // graphify-py #1017: TypeScript 5.0 allows `extends` as an array; later
    // entries override earlier ones. Before the fix, an array `extends`
    // raised an error inside the alias loader, which silently dropped every
    // file that depended on those aliases. The fix is in
    // `crates/graphify-extract/src/tsconfig.rs::read_tsconfig_aliases`.
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path();
    write_file(
        &root.join("tsconfig.base.json"),
        "{\"compilerOptions\": {\"strict\": true}}",
    );
    write_file(
        &root.join("tsconfig.paths.json"),
        "{\"compilerOptions\": {\"baseUrl\": \".\", \"paths\": {\"$lib/*\": [\"src/lib/*\"]}}}",
    );
    write_file(
        &root.join("tsconfig.json"),
        "{\"extends\": [\"./tsconfig.base.json\", \"./tsconfig.paths.json\"]}",
    );
    let target = root.join("src/lib/types/type-helpers.ts");
    write_file(&target, "export type Helper = string\n");
    let importer = root.join("src/routes/page.ts");
    write_file(
        &importer,
        "import type { Helper } from '$lib/types/type-helpers'\nconst value: Helper = 'x'\n",
    );

    let result = extract_js(&importer);
    let targets = import_targets(&result, Some("imports_from"));
    let target_canon = target.canonicalize().unwrap_or(target);
    assert!(
        targets.contains(&make_id1(&target_canon.to_string_lossy())),
        "type-helpers.ts not in alias-resolved targets: {targets:?}"
    );
}

#[test]
fn extract_js_pure_export_no_from_not_treated_as_reexport() {
    // `export { x }` with no `from` clause is a local re-bind — must NOT
    // emit a re_exports edge.
    let tmp = tempfile::tempdir().expect("tempdir");
    let file = tmp.path().join("local.ts");
    std::fs::write(&file, "const x = 1;\nexport { x };\n").expect("write fixture");
    let result = extract_js(&file);
    let re_exports: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "re_exports")
        .collect();
    assert!(re_exports.is_empty(), "no re_exports for pure local export");
}

#[test]
fn extract_swift_merges_extension_across_files() {
    // Two Swift files: `Foo.swift` declares `class Foo`, `Foo+Ext.swift`
    // declares `extension Foo`. The Swift merge pass collapses the
    // extension node onto the canonical class so downstream consumers see
    // a single `Foo` node.
    use graphify_extract::extract;
    let tmp = tempfile::tempdir().expect("tempdir");
    let canonical = tmp.path().join("Foo.swift");
    let extension = tmp.path().join("Foo+Ext.swift");
    std::fs::write(&canonical, "class Foo {\n    func bar() {}\n}\n").expect("test invariant");
    std::fs::write(&extension, "extension Foo {\n    func baz() {}\n}\n").expect("test invariant");
    let result = extract(&[canonical.clone(), extension.clone()], None);
    let foo_nodes: Vec<_> = result
        .nodes
        .iter()
        .filter(|n| n.get("label").and_then(serde_json::Value::as_str) == Some("Foo"))
        .collect();
    assert_eq!(
        foo_nodes.len(),
        1,
        "expected a single canonical Foo node, got {}: {foo_nodes:?}",
        foo_nodes.len()
    );
}

#[test]
fn extract_bash_source_user_defined_emits_calls_not_imports_from() {
    // When `source` is user-defined as a function (shadowing the builtin),
    // `source ./helpers.sh` must emit a `calls` edge to the function, not
    // an `imports_from` edge. The pre-scan ensures the shadow is detected
    // even when the function definition appears AFTER the source call.
    let tmp = tempfile::tempdir().expect("tempdir");
    let script = tmp.path().join("shadow.sh");
    std::fs::write(
        &script,
        "#!/bin/bash\nsource ./helpers.sh\nsource() { echo custom; }\n",
    )
    .expect("test invariant");
    let result = extract_bash(&script);
    let imports_from: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports_from")
        .collect();
    let calls: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls" && e.target.ends_with("_source"))
        .collect();
    assert!(
        imports_from.is_empty(),
        "user-defined source must not emit imports_from"
    );
    assert!(
        !calls.is_empty(),
        "user-defined source must emit a calls edge: {:?}",
        result.edges
    );
}

#[test]
fn extract_bash_no_self_loops() {
    let result = extract_bash(&fixtures().join("sample.sh"));
    for edge in &result.edges {
        assert_ne!(edge.source, edge.target, "Self-loop: {edge:?}");
    }
}

#[test]
fn extract_bash_no_dangling_edges() {
    let result = extract_bash(&fixtures().join("sample.sh"));
    let node_ids: std::collections::HashSet<&str> =
        result.nodes.iter().map(|n| n.id.as_str()).collect();
    for edge in &result.edges {
        assert!(
            node_ids.contains(edge.source.as_str()),
            "Dangling source: {}",
            edge.source
        );
        if edge.relation != "imports_from" && edge.relation != "imports" {
            assert!(
                node_ids.contains(edge.target.as_str()),
                "Dangling target: {}",
                edge.target
            );
        }
    }
}

#[test]
fn extract_bash_skip_builtins_in_calls() {
    let result = extract_bash(&fixtures().join("sample.sh"));
    let builtins = [
        "echo", "cd", "set", "export", "local", "mkdir", "if", "then",
    ];
    let call_targets: std::collections::HashSet<&str> = result
        .edges
        .iter()
        .filter(|e| e.relation == "calls")
        .map(|e| e.target.as_str())
        .collect();
    for b in &builtins {
        assert!(
            !call_targets.iter().any(|t| t.contains(b)),
            "Builtin '{b}' appeared as calls target"
        );
    }
}

// ── JSON extractor ────────────────────────────────────────────────────────────

#[test]
fn extract_json_top_level_keys() {
    let result = extract_json(&fixtures().join("sample.json"));
    assert!(result.error.is_none());
    let labels: std::collections::HashSet<&str> =
        result.nodes.iter().map(|n| n.label.as_str()).collect();
    assert!(labels.contains("name"), "labels: {labels:?}");
    assert!(labels.contains("version"), "labels: {labels:?}");
    assert!(labels.contains("scripts"), "labels: {labels:?}");
    assert!(labels.contains("dependencies"), "labels: {labels:?}");
}

#[test]
fn extract_json_nested_contains() {
    let result = extract_json(&fixtures().join("sample.json"));
    let contains: Vec<(&str, &str)> = result
        .edges
        .iter()
        .filter(|e| e.relation == "contains")
        .map(|e| (e.source.as_str(), e.target.as_str()))
        .collect();
    assert!(
        contains
            .iter()
            .any(|(s, t)| s.contains("scripts") && t.contains("build"))
    );
    assert!(
        contains
            .iter()
            .any(|(s, t)| s.contains("scripts") && t.contains("test"))
    );
    assert!(
        contains
            .iter()
            .any(|(s, t)| s.contains("dependencies") && t.contains("react"))
    );
}

#[test]
fn extract_json_dependencies_become_imports() {
    let result = extract_json(&fixtures().join("sample.json"));
    let targets: std::collections::HashSet<&str> = result
        .edges
        .iter()
        .filter(|e| e.relation == "imports")
        .map(|e| e.target.as_str())
        .collect();
    assert!(
        targets.iter().any(|t| t.contains("react")),
        "targets: {targets:?}"
    );
    assert!(
        targets.iter().any(|t| t.contains("axios")),
        "targets: {targets:?}"
    );
    assert!(
        targets.iter().any(|t| t.contains("typescript")),
        "targets: {targets:?}"
    );
}

#[test]
fn extract_json_extends_resolved() {
    let result = extract_json(&fixtures().join("sample_tsconfig.json"));
    let extends_edges: Vec<_> = result
        .edges
        .iter()
        .filter(|e| e.relation == "extends")
        .collect();
    assert!(
        !extends_edges.is_empty(),
        "Expected at least one extends edge"
    );
    assert_eq!(extends_edges[0].context.as_deref(), Some("import"));
}

#[test]
fn extract_json_large_file_skipped() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let big = tmp.path().join("big.json");
    let mut content = b"{\"x\": \"".to_vec();
    content.extend(vec![b'a'; 1_048_576]);
    content.extend(b"\"}");
    std::fs::write(&big, &content).expect("write fixture");
    let result = extract_json(&big);
    assert!(result.error.is_some(), "Expected error for large file");
    assert!(result.nodes.is_empty());
}

#[test]
fn extract_json_handles_invalid_json() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let bad = tmp.path().join("broken.json");
    std::fs::write(&bad, "{this is not: valid json!!!").expect("write fixture");
    let result = extract_json(&bad);
    // Must not crash — error or empty result is acceptable
    let _ = result; // just verify no panic
}

#[test]
fn extract_json_no_self_loops() {
    let result = extract_json(&fixtures().join("sample.json"));
    for edge in &result.edges {
        assert_ne!(edge.source, edge.target, "Self-loop: {edge:?}");
    }
}

// ── Astro extractor ───────────────────────────────────────────────────────────

fn write_file(path: &Path, body: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(path, body).expect("write file");
}

fn import_targets(
    result: &graphify_extract::FileResult,
    relation: Option<&str>,
) -> std::collections::HashSet<String> {
    result
        .edges
        .iter()
        .filter(|e| relation.is_none_or(|r| e.relation == r))
        .map(|e| e.target.clone())
        .collect()
}

#[test]
fn extract_astro_picks_up_frontmatter_static_imports() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let page = tmp.path().join("src/pages/index.astro");
    write_file(
        &page,
        "---\nimport Layout from '../layouts/Layout.astro';\nimport Hero from '../components/Hero.astro';\nconst { title } = Astro.props;\n---\n\n<Layout title={title}>\n  <Hero />\n</Layout>\n",
    );
    let layout = tmp.path().join("src/layouts/Layout.astro");
    write_file(&layout, "---\n---\n<slot />\n");
    let hero = tmp.path().join("src/components/Hero.astro");
    write_file(&hero, "---\n---\n<h1>hi</h1>\n");

    let result = extract_astro(&page);
    let targets = import_targets(&result, Some("imports_from"));
    assert!(
        targets.contains(&make_id1(&layout.to_string_lossy())),
        "layout not in targets: {targets:?}"
    );
    assert!(
        targets.contains(&make_id1(&hero.to_string_lossy())),
        "hero not in targets: {targets:?}"
    );
}

#[test]
fn extract_astro_handles_dynamic_import_in_frontmatter() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let page = tmp.path().join("src/pages/lazy.astro");
    write_file(
        &page,
        "---\nconst Mod = await import('./Other.astro');\n---\n\n<div>{Mod.default}</div>\n",
    );
    let other = tmp.path().join("src/pages/Other.astro");
    write_file(&other, "---\n---\n<p>o</p>\n");

    let result = extract_astro(&page);
    let targets = import_targets(&result, Some("dynamic_import"));
    assert!(
        targets.contains(&make_id1(&other.to_string_lossy())),
        "other not in targets: {targets:?}"
    );
}

#[test]
fn extract_astro_picks_up_client_side_script_imports() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let page = tmp.path().join("src/pages/with-script.astro");
    write_file(
        &page,
        "---\nimport Layout from '../layouts/Layout.astro';\n---\n\n<Layout>\n  <button id=\"b\">click</button>\n</Layout>\n\n<script>\n  import { hydrate } from '../client/hydrate.ts';\n  hydrate(document.getElementById('b'));\n</script>\n",
    );
    let layout = tmp.path().join("src/layouts/Layout.astro");
    write_file(&layout, "---\n---\n<slot />\n");
    let hydrate = tmp.path().join("src/client/hydrate.ts");
    write_file(&hydrate, "export function hydrate(){}\n");

    let result = extract_astro(&page);
    let targets = import_targets(&result, Some("imports_from"));
    assert!(
        targets.contains(&make_id1(&layout.to_string_lossy())),
        "layout not in targets: {targets:?}"
    );
    assert!(
        targets.contains(&make_id1(&hydrate.to_string_lossy())),
        "hydrate not in targets: {targets:?}"
    );
}

#[test]
fn extract_astro_no_frontmatter_does_not_crash() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let page = tmp.path().join("src/pages/plain.astro");
    write_file(&page, "<h1>no frontmatter here</h1>\n");
    let result = extract_astro(&page);
    // Must not panic; empty result acceptable
    assert_eq!(
        import_targets(&result, Some("imports_from")),
        std::collections::HashSet::new()
    );
}

#[test]
fn extract_astro_handles_tsconfig_path_alias() {
    let tmp = tempfile::tempdir().expect("tempdir");
    write_file(
        &tmp.path().join("tsconfig.json"),
        "{\n  \"compilerOptions\": {\n    \"baseUrl\": \".\",\n    \"paths\": { \"@components/*\": [\"src/components/*\"] }\n  }\n}\n",
    );
    let page = tmp.path().join("src/pages/alias.astro");
    write_file(
        &page,
        "---\nimport Hero from '@components/Hero.astro';\n---\n\n<Hero />\n",
    );
    let hero = tmp.path().join("src/components/Hero.astro");
    write_file(&hero, "---\n---\n<h1>h</h1>\n");

    let result = extract_astro(&page);
    let targets = import_targets(&result, Some("imports_from"));
    // Canonicalize to match how the tsconfig resolver resolves paths (follows macOS /private symlink)
    let hero_canon = hero.canonicalize().unwrap_or(hero);
    assert!(
        targets.contains(&make_id1(&hero_canon.to_string_lossy())),
        "hero not in targets: {targets:?}"
    );
}
