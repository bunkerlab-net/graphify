//! Parity tests for indirect-dispatch (`indirect_call`) edges.
//!
//! 1:1 ports of `graphify-py/tests/test_indirect_dispatch.py`,
//! `test_indirect_dispatch_assign_return.py`, and `test_indirect_dispatch_getattr.py`.
//!
//! A function referenced BY NAME — passed as a call argument, listed in a dispatch
//! table, bound / returned, or named by a `getattr` string — emits a distinct
//! INFERRED `indirect_call` edge (leaving the precise `calls` relation untouched)
//! that `affected` picks up, guarded against param/local shadows and non-callable
//! targets (#1565/#1566).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use graphify_affected::{DEFAULT_AFFECTED_RELATIONS, affected_nodes};
use graphify_build::build_from_json;
use graphify_extract::{Edge, ExtractOutput, FileResult, extract, extract_python};
use serde_json::{Value, json};
use tempfile::tempdir;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// `{ label-without-parens : id }` for a single-file [`FileResult`].
fn nid_map(result: &FileResult) -> HashMap<String, String> {
    result
        .nodes
        .iter()
        .map(|n| {
            (
                n.label.trim_end_matches(['(', ')']).to_string(),
                n.id.clone(),
            )
        })
        .collect()
}

/// Set of `(source, target)` pairs for edges with the given relation.
fn rels(edges: &[Edge], relation: &str) -> HashSet<(String, String)> {
    edges
        .iter()
        .filter(|e| e.relation == relation)
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect()
}

/// Write `m.py` with `src` and extract it (single-file Python).
fn extract_py(dir: &Path, src: &str) -> (FileResult, HashMap<String, String>) {
    let path = dir.join("m.py");
    std::fs::write(&path, src).expect("write m.py");
    let r = extract_python(&path);
    let nid = nid_map(&r);
    (r, nid)
}

// ── ExtractOutput (multi-file) helpers ────────────────────────────────────────

/// `{ label-without-parens : id }` for a multi-file [`ExtractOutput`].
fn out_nid(out: &ExtractOutput) -> HashMap<String, String> {
    out.nodes
        .iter()
        .map(|n| {
            let label = n.get("label").and_then(Value::as_str).unwrap_or_default();
            let id = n.get("id").and_then(Value::as_str).unwrap_or_default();
            (
                label.trim_end_matches(['(', ')']).to_string(),
                id.to_string(),
            )
        })
        .collect()
}

/// Node id whose label is exactly `label` (e.g. a file node `"m.py"`).
fn out_id_by_label(out: &ExtractOutput, label: &str) -> String {
    out.nodes
        .iter()
        .find(|n| n.get("label").and_then(Value::as_str) == Some(label))
        .and_then(|n| n.get("id").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

fn out_rels(out: &ExtractOutput, relation: &str) -> HashSet<(String, String)> {
    out.edges
        .iter()
        .filter(|e| e.get("relation").and_then(Value::as_str) == Some(relation))
        .map(|e| {
            (
                e.get("source")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
                e.get("target")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            )
        })
        .collect()
}

/// Write `files` under `base/<subdir>` and run the multi-file `extract` with the
/// project root at `base` (`cache_root` == root triggers id relativization, the
/// `graphify extract` CLI shape).
fn extract_dir(base: &Path, subdir: &str, files: &[(&str, &str)]) -> ExtractOutput {
    let dir = base.join(subdir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    let mut paths: Vec<PathBuf> = Vec::new();
    for (name, body) in files {
        let p = dir.join(name);
        std::fs::write(&p, body).expect("write");
        paths.push(p);
    }
    extract(&paths, Some(base))
}

/// Build a graph from an [`ExtractOutput`] and return the ids reachable in a
/// reverse `affected` walk from `seed`.
fn affected_ids(out: &ExtractOutput, root: &Path, seed: &str) -> HashSet<String> {
    let value = serde_json::to_value(out).expect("serialise");
    let graph = build_from_json(value, false, Some(root)).expect("build graph");
    affected_nodes(&graph, seed, DEFAULT_AFFECTED_RELATIONS, 2)
        .into_iter()
        .map(|h| h.node_id)
        .collect()
}

/// Same, from a single-file [`FileResult`].
fn affected_ids_file(result: &FileResult, seed: &str) -> HashSet<String> {
    let value = json!({ "nodes": &result.nodes, "edges": &result.edges });
    let graph = build_from_json(value, false, None).expect("build graph");
    affected_nodes(&graph, seed, DEFAULT_AFFECTED_RELATIONS, 2)
        .into_iter()
        .map(|h| h.node_id)
        .collect()
}

const SRC: &str = "import threading\n\n\ndef handler(x):\n    return x * 2\n\n\ndef direct():\n    return handler(1)\n\n\ndef via_submit(pool):\n    pool.submit(handler, 1)\n\n\ndef via_thread():\n    threading.Thread(target=handler).start()\n\n\ndef via_map(xs):\n    return map(handler, xs)\n";

// ── Same-file argument callbacks ──────────────────────────────────────────────

#[test]
fn emits_indirect_call_edges_and_keeps_calls_precise() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(tmp.path(), SRC);
    let calls = rels(&r.edges, "calls");
    let indirect = rels(&r.edges, "indirect_call");
    let handler = &nid["handler"];

    assert!(calls.contains(&(nid["direct"].clone(), handler.clone())));
    assert!(indirect.contains(&(nid["via_submit"].clone(), handler.clone())));
    assert!(indirect.contains(&(nid["via_thread"].clone(), handler.clone())));
    assert!(indirect.contains(&(nid["via_map"].clone(), handler.clone())));
    assert!(!calls.contains(&(nid["via_submit"].clone(), handler.clone())));
    assert!(!calls.contains(&(nid["via_thread"].clone(), handler.clone())));
    assert!(!calls.contains(&(nid["via_map"].clone(), handler.clone())));

    for e in r.edges.iter().filter(|e| e.relation == "indirect_call") {
        assert_eq!(e.context.as_deref(), Some("argument"));
        assert_eq!(e.confidence, "INFERRED");
    }
}

#[test]
fn affected_includes_indirect_callers() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(tmp.path(), SRC);
    let affected = affected_ids_file(&r, &nid["handler"]);
    assert!(affected.contains(&nid["via_submit"]));
    assert!(affected.contains(&nid["via_thread"]));
    assert!(affected.contains(&nid["via_map"]));
}

// ── Soundness guards ──────────────────────────────────────────────────────────

#[test]
fn param_shadow_emits_no_indirect_call() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler():\n    return 1\n\n\ndef via(pool, handler):\n    pool.submit(handler)\n",
    );
    let indirect = rels(&r.edges, "indirect_call");
    assert!(!indirect.contains(&(nid["via"].clone(), nid["handler"].clone())));
    assert!(indirect.iter().all(|(_s, t)| *t != nid["handler"]));
}

#[test]
fn local_assignment_shadow_emits_no_indirect_call() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler():\n    return 1\n\n\ndef make():\n    return lambda: None\n\n\ndef via(pool):\n    handler = make()\n    pool.submit(handler)\n",
    );
    let indirect = rels(&r.edges, "indirect_call");
    assert!(!indirect.contains(&(nid["via"].clone(), nid["handler"].clone())));
    assert!(indirect.iter().all(|(_s, t)| *t != nid["handler"]));
}

#[test]
fn data_var_matching_function_name_emits_no_indirect_call() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def config():\n    return {\"k\": \"v\"}\n\n\ndef process(x):\n    return x\n\n\ndef use():\n    config = {\"k\": \"v\"}\n    process(config)\n",
    );
    let indirect = rels(&r.edges, "indirect_call");
    assert!(!indirect.contains(&(nid["use"].clone(), nid["config"].clone())));
}

#[test]
fn genuine_module_function_still_emits_indirect_call() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler():\n    return 1\n\n\ndef via(pool):\n    pool.submit(handler)\n",
    );
    let indirect = rels(&r.edges, "indirect_call");
    assert!(indirect.contains(&(nid["via"].clone(), nid["handler"].clone())));
}

// ── Cross-file indirect dispatch ──────────────────────────────────────────────

#[test]
fn cross_file_indirect_survives_id_relativization() {
    let tmp = tempdir().unwrap();
    let base = tmp.path();
    std::fs::create_dir_all(base.join("handlers")).unwrap();
    std::fs::write(
        base.join("handlers/__init__.py"),
        "def on_event(x):\n    return x\n",
    )
    .unwrap();
    std::fs::write(
        base.join("scheduler.py"),
        "from handlers import on_event\n\n\ndef schedule(pool):\n    pool.submit(on_event)\n",
    )
    .unwrap();
    let out = extract(
        &[base.join("handlers/__init__.py"), base.join("scheduler.py")],
        Some(base),
    );
    let nid = out_nid(&out);
    assert!(
        out_rels(&out, "indirect_call")
            .contains(&(nid["schedule"].clone(), nid["on_event"].clone()))
    );
    // the internal callable marker must never ship to graph.json — it is stamped
    // inside a node's nested `metadata` map, so check there (and the top level).
    assert!(out.nodes.iter().all(|n| {
        !n.contains_key("_callable")
            && n.get("metadata")
                .and_then(serde_json::Value::as_object)
                .is_none_or(|m| !m.contains_key("_callable"))
    }));
}

#[test]
fn cross_file_indirect_survives_ast_cache_roundtrip() {
    // The `_callable` marker + `indirect` raw_calls must round-trip the per-file
    // AST cache: a cold run writes them, a warm run (same cache_root) reads them
    // back. Python files are cached (not in the JS bypass list), so a second
    // extract must still emit the cross-file indirect_call from cached data.
    let tmp = tempdir().unwrap();
    let files = [
        ("handlers.py", "def on_event(x):\n    return x\n"),
        (
            "scheduler.py",
            "from handlers import on_event\n\n\ndef schedule(pool):\n    pool.submit(on_event)\n",
        ),
    ];
    let cold = extract_dir(tmp.path(), "pkg", &files);
    let warm = extract_dir(tmp.path(), "pkg", &files);
    for out in [&cold, &warm] {
        let nid = out_nid(out);
        let pair = (nid["schedule"].clone(), nid["on_event"].clone());
        assert!(
            out_rels(out, "indirect_call").contains(&pair),
            "cross-file indirect_call must survive the AST cache round-trip"
        );
        assert!(out.nodes.iter().all(|n| {
            !n.contains_key("_callable")
                && n.get("metadata")
                    .and_then(serde_json::Value::as_object)
                    .is_none_or(|m| !m.contains_key("_callable"))
        }));
    }
}

#[test]
fn cross_file_imported_callback_emits_indirect_call() {
    let tmp = tempdir().unwrap();
    let out = extract_dir(
        tmp.path(),
        "pkg",
        &[
            ("handlers.py", "def on_event(x):\n    return x\n"),
            (
                "scheduler.py",
                "from handlers import on_event\n\n\ndef schedule(pool):\n    pool.submit(on_event)\n",
            ),
        ],
    );
    let nid = out_nid(&out);
    let pair = (nid["schedule"].clone(), nid["on_event"].clone());
    assert!(out_rels(&out, "indirect_call").contains(&pair));
    assert!(!out_rels(&out, "calls").contains(&pair));
    for e in &out.edges {
        if e.get("relation").and_then(Value::as_str) == Some("indirect_call")
            && e.get("target").and_then(Value::as_str) == Some(nid["on_event"].as_str())
        {
            assert_eq!(
                e.get("confidence").and_then(Value::as_str),
                Some("INFERRED")
            );
        }
    }
}

#[test]
fn cross_file_affected_includes_importing_dispatcher() {
    let tmp = tempdir().unwrap();
    let out = extract_dir(
        tmp.path(),
        "pkg",
        &[
            ("handlers.py", "def on_event(x):\n    return x\n"),
            (
                "scheduler.py",
                "from handlers import on_event\n\n\ndef schedule(pool):\n    pool.submit(on_event)\n",
            ),
        ],
    );
    let nid = out_nid(&out);
    let affected = affected_ids(&out, tmp.path(), &nid["on_event"]);
    assert!(affected.contains(&nid["schedule"]));
}

#[test]
fn cross_file_param_shadow_emits_no_indirect_call() {
    let tmp = tempdir().unwrap();
    let out = extract_dir(
        tmp.path(),
        "pkg",
        &[
            ("handlers.py", "def on_event(x):\n    return x\n"),
            (
                "scheduler.py",
                "from handlers import on_event\n\n\ndef schedule(pool, on_event):\n    pool.submit(on_event)\n",
            ),
        ],
    );
    let nid = out_nid(&out);
    assert!(
        !out_rels(&out, "indirect_call")
            .contains(&(nid["schedule"].clone(), nid["on_event"].clone()))
    );
}

// ── Dispatch tables ───────────────────────────────────────────────────────────

#[test]
fn module_level_dict_registry_emits_indirect_call() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def create(x):\n    return x\n\n\ndef delete(x):\n    return x\n\n\nROUTES = {\"create\": create, \"delete\": delete}\n",
    );
    let indirect = rels(&r.edges, "indirect_call");
    let file_nid = r
        .nodes
        .iter()
        .find(|n| n.label == "m.py")
        .unwrap()
        .id
        .clone();
    assert!(indirect.contains(&(file_nid.clone(), nid["create"].clone())));
    assert!(indirect.contains(&(file_nid.clone(), nid["delete"].clone())));
    assert!(!rels(&r.edges, "calls").contains(&(file_nid, nid["create"].clone())));
}

#[test]
fn module_level_list_registry_emits_indirect_call() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def on_start():\n    pass\n\n\ndef on_stop():\n    pass\n\n\nHOOKS = [on_start, on_stop]\n",
    );
    let indirect = rels(&r.edges, "indirect_call");
    let file_nid = r
        .nodes
        .iter()
        .find(|n| n.label == "m.py")
        .unwrap()
        .id
        .clone();
    assert!(indirect.contains(&(file_nid.clone(), nid["on_start"].clone())));
    assert!(indirect.contains(&(file_nid, nid["on_stop"].clone())));
}

#[test]
fn function_scoped_dispatch_table_attributes_to_function() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def cb(x):\n    return x\n\n\ndef build():\n    return {\"k\": cb}\n",
    );
    assert!(rels(&r.edges, "indirect_call").contains(&(nid["build"].clone(), nid["cb"].clone())));
}

#[test]
fn dict_keys_are_not_dispatch_targets() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def keyfn():\n    pass\n\n\ndef valfn():\n    pass\n\n\nT = {keyfn: valfn}\n",
    );
    let indirect = rels(&r.edges, "indirect_call");
    assert!(indirect.iter().all(|(_s, t)| *t != nid["keyfn"]));
    let file_nid = r
        .nodes
        .iter()
        .find(|n| n.label == "m.py")
        .unwrap()
        .id
        .clone();
    assert!(indirect.contains(&(file_nid, nid["valfn"].clone())));
}

#[test]
fn non_callable_collection_value_emits_no_indirect_call() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def use():\n    pass\n\n\nCONF = {\"timeout\": 30, \"name\": use}\n",
    );
    let indirect = rels(&r.edges, "indirect_call");
    let file_nid = r
        .nodes
        .iter()
        .find(|n| n.label == "m.py")
        .unwrap()
        .id
        .clone();
    assert!(indirect.contains(&(file_nid, nid["use"].clone())));
    assert_eq!(indirect.len(), 1);
}

#[test]
fn module_level_reassigned_name_shadows_dispatch_value() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler():\n    pass\n\n\nhandler = object()\nT = {\"h\": handler}\n",
    );
    let indirect = rels(&r.edges, "indirect_call");
    assert!(indirect.iter().all(|(_s, t)| *t != nid["handler"]));
}

#[test]
fn cross_file_dict_registry_emits_indirect_call() {
    let tmp = tempdir().unwrap();
    let out = extract_dir(
        tmp.path(),
        "pkg",
        &[
            ("handlers.py", "def on_event(x):\n    return x\n"),
            (
                "registry.py",
                "from handlers import on_event\n\n\nROUTES = {\"event\": on_event}\n",
            ),
        ],
    );
    let nid = out_nid(&out);
    let reg_file = out_id_by_label(&out, "registry.py");
    assert!(out_rels(&out, "indirect_call").contains(&(reg_file, nid["on_event"].clone())));
}

// ── Assignment / return references ────────────────────────────────────────────

const ASSIGN_RETURN: &str = "def handler(): ...\ndef other(): ...\n\ndef bind():\n    cb = handler\n    return cb\n\ndef make():\n    return other\n";

#[test]
fn assignment_and_return_emit_indirect_call() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(tmp.path(), ASSIGN_RETURN);
    let ind = rels(&r.edges, "indirect_call");
    assert!(ind.contains(&(nid["bind"].clone(), nid["handler"].clone())));
    assert!(ind.contains(&(nid["make"].clone(), nid["other"].clone())));
    assert!(!rels(&r.edges, "calls").contains(&(nid["bind"].clone(), nid["handler"].clone())));
    for e in r.edges.iter().filter(|e| e.relation == "indirect_call") {
        assert!(matches!(
            e.context.as_deref(),
            Some("assignment" | "return")
        ));
        assert_eq!(e.confidence, "INFERRED");
    }
}

#[test]
fn multiple_assignment_emits_for_each() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def f(): ...\ndef g(): ...\n\ndef via():\n    a, b = f, g\n    return a\n",
    );
    let ind = rels(&r.edges, "indirect_call");
    assert!(ind.contains(&(nid["via"].clone(), nid["f"].clone())));
    assert!(ind.contains(&(nid["via"].clone(), nid["g"].clone())));
}

#[test]
fn module_level_assignment_emits_indirect_call() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(tmp.path(), "def handler(): ...\n\nCALLBACK = handler\n");
    assert!(
        rels(&r.edges, "indirect_call").contains(&(nid["m.py"].clone(), nid["handler"].clone()))
    );
}

#[test]
fn assignment_feeds_affected() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(tmp.path(), ASSIGN_RETURN);
    let affected = affected_ids_file(&r, &nid["handler"]);
    assert!(affected.contains(&nid["bind"]));
}

#[test]
fn assign_return_param_shadow_emits_nothing() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler(): ...\n\ndef via(handler):\n    cb = handler\n    return handler\n",
    );
    assert!(
        rels(&r.edges, "indirect_call")
            .iter()
            .all(|(_s, t)| *t != nid["handler"])
    );
}

#[test]
fn assign_return_local_shadow_emits_nothing() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler(): ...\n\ndef via():\n    handler = object()\n    cb = handler\n    return handler\n",
    );
    assert!(
        rels(&r.edges, "indirect_call")
            .iter()
            .all(|(_s, t)| *t != nid["handler"])
    );
}

#[test]
fn assign_return_non_callable_value_emits_nothing() {
    let tmp = tempdir().unwrap();
    let (r, _nid) = extract_py(
        tmp.path(),
        "def handler(): ...\n\ndef via():\n    cb = TIMEOUT\n    return cb\n",
    );
    assert!(rels(&r.edges, "indirect_call").is_empty());
}

// ── getattr reflective dispatch ───────────────────────────────────────────────

#[test]
fn getattr_string_literal_emits_indirect_call() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler(): ...\ndef other(): ...\n\ndef dispatch(obj):\n    fn = getattr(obj, \"handler\")\n    return fn()\n",
    );
    assert!(
        rels(&r.edges, "indirect_call")
            .contains(&(nid["dispatch"].clone(), nid["handler"].clone()))
    );
    assert!(!rels(&r.edges, "calls").contains(&(nid["dispatch"].clone(), nid["handler"].clone())));
    for e in r
        .edges
        .iter()
        .filter(|e| e.relation == "indirect_call" && e.target == nid["handler"])
    {
        assert_eq!(e.context.as_deref(), Some("getattr"));
        assert_eq!(e.confidence, "INFERRED");
    }
}

#[test]
fn getattr_with_default_emits() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler(): ...\n\ndef dispatch(obj):\n    return getattr(obj, \"handler\", None)()\n",
    );
    assert!(
        rels(&r.edges, "indirect_call")
            .contains(&(nid["dispatch"].clone(), nid["handler"].clone()))
    );
}

#[test]
fn module_level_getattr_emits() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "import sys\ndef handler(): ...\n\nHANDLER = getattr(sys.modules[__name__], \"handler\")\n",
    );
    assert!(
        rels(&r.edges, "indirect_call").contains(&(nid["m.py"].clone(), nid["handler"].clone()))
    );
}

#[test]
fn getattr_feeds_affected() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler(): ...\ndef other(): ...\n\ndef dispatch(obj):\n    fn = getattr(obj, \"handler\")\n    return fn()\n",
    );
    let affected = affected_ids_file(&r, &nid["handler"]);
    assert!(affected.contains(&nid["dispatch"]));
}

#[test]
fn getattr_string_not_shadowed_by_param() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler(): ...\n\ndef via(handler):\n    return getattr(handler, \"handler\")\n",
    );
    let got: Vec<&Edge> = r
        .edges
        .iter()
        .filter(|e| {
            e.relation == "indirect_call"
                && (e.source.clone(), e.target.clone())
                    == (nid["via"].clone(), nid["handler"].clone())
        })
        .collect();
    assert!(!got.is_empty());
    assert!(got.iter().all(|e| e.context.as_deref() == Some("getattr")));
}

#[test]
fn dynamic_getattr_names_emit_nothing() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler(): ...\n\ndef via(obj, name, evt):\n    a = getattr(obj, name)\n    b = getattr(obj, f\"on_{evt}\")\n    c = getattr(obj, \"on_\" + evt)\n    return a, b, c\n",
    );
    assert!(
        rels(&r.edges, "indirect_call")
            .iter()
            .all(|(s, _t)| *s != nid["via"])
    );
}

#[test]
fn getattr_non_callable_name_emits_nothing() {
    let tmp = tempdir().unwrap();
    let (r, _nid) = extract_py(
        tmp.path(),
        "TIMEOUT = 30\n\ndef via(obj):\n    return getattr(obj, \"TIMEOUT\")\n",
    );
    assert!(rels(&r.edges, "indirect_call").is_empty());
}

#[test]
fn method_named_getattr_is_not_the_builtin() {
    let tmp = tempdir().unwrap();
    let (r, nid) = extract_py(
        tmp.path(),
        "def handler(): ...\n\nclass Registry:\n    def getattr(self, name): ...\n\ndef via(reg):\n    return reg.getattr(\"handler\")\n",
    );
    assert!(
        rels(&r.edges, "indirect_call")
            .iter()
            .all(|(_s, t)| *t != nid["handler"])
    );
}

// ── JS / TS ───────────────────────────────────────────────────────────────────

fn extract_js_dir(base: &Path, files: &[(&str, &str)]) -> ExtractOutput {
    extract_dir(base, "src", files)
}

#[test]
fn js_function_scoped_call_argument() {
    let tmp = tempdir().unwrap();
    let out = extract_js_dir(
        tmp.path(),
        &[(
            "a.js",
            "function handler(x){ return x; }\nfunction via(pool){ pool.submit(handler); }\n",
        )],
    );
    let nid = out_nid(&out);
    let pair = (nid["via"].clone(), nid["handler"].clone());
    assert!(out_rels(&out, "indirect_call").contains(&pair));
    assert!(!out_rels(&out, "calls").contains(&pair));
}

#[test]
fn js_module_object_and_array_registry() {
    let tmp = tempdir().unwrap();
    let out = extract_js_dir(
        tmp.path(),
        &[(
            "a.js",
            "function handler(x){ return x; }\nconst cb = () => {};\nconst ROUTES = { create: handler, run: cb };\nconst HOOKS = [handler, cb];\n",
        )],
    );
    let nid = out_nid(&out);
    let file_nid = out_id_by_label(&out, "a.js");
    let indirect = out_rels(&out, "indirect_call");
    assert!(indirect.contains(&(file_nid.clone(), nid["handler"].clone())));
    assert!(indirect.contains(&(file_nid, nid["cb"].clone())));
}

#[test]
fn js_module_level_callback_registration() {
    let tmp = tempdir().unwrap();
    let out = extract_js_dir(
        tmp.path(),
        &[(
            "r.js",
            "function home(req, res){}\nconst list = () => {};\napp.get(\"/\", home);\nemitter.on(\"evt\", list);\nsetTimeout(home, 100);\n",
        )],
    );
    let nid = out_nid(&out);
    let file_nid = out_id_by_label(&out, "r.js");
    let indirect = out_rels(&out, "indirect_call");
    assert!(indirect.contains(&(file_nid.clone(), nid["home"].clone())));
    assert!(indirect.contains(&(file_nid, nid["list"].clone())));
}

#[test]
fn js_inline_arrow_argument_is_not_a_reference() {
    let tmp = tempdir().unwrap();
    let out = extract_js_dir(
        tmp.path(),
        &[(
            "i.js",
            "function via(arr){ arr.map(x => x * 2); arr.forEach(function(y){}); }\n",
        )],
    );
    assert!(out_rels(&out, "indirect_call").is_empty());
}

#[test]
fn js_parameter_shadow_emits_no_indirect_call() {
    let tmp = tempdir().unwrap();
    let out = extract_js_dir(
        tmp.path(),
        &[(
            "s.js",
            "function handler(){}\nfunction via(pool, handler){ pool.submit(handler); }\n",
        )],
    );
    let nid = out_nid(&out);
    assert!(
        out_rels(&out, "indirect_call")
            .iter()
            .all(|(_s, t)| *t != nid["handler"])
    );
}

#[test]
fn js_object_keys_and_data_values_excluded() {
    let tmp = tempdir().unwrap();
    let out = extract_js_dir(
        tmp.path(),
        &[(
            "k.js",
            "function keyfn(){}\nfunction valfn(){}\nconst T = { [keyfn]: valfn, timeout: 30 };\n",
        )],
    );
    let nid = out_nid(&out);
    let indirect = out_rels(&out, "indirect_call");
    assert!(indirect.iter().all(|(_s, t)| *t != nid["keyfn"]));
    let file_nid = out_id_by_label(&out, "k.js");
    assert!(indirect.contains(&(file_nid, nid["valfn"].clone())));
}

#[test]
fn js_shorthand_property_reference() {
    let tmp = tempdir().unwrap();
    let out = extract_js_dir(
        tmp.path(),
        &[("sh.js", "function handler(){}\nconst obj = { handler };\n")],
    );
    let nid = out_nid(&out);
    let file_nid = out_id_by_label(&out, "sh.js");
    assert!(out_rels(&out, "indirect_call").contains(&(file_nid, nid["handler"].clone())));
}

#[test]
fn js_cross_file_imported_callback_in_object() {
    let tmp = tempdir().unwrap();
    let out = extract_js_dir(
        tmp.path(),
        &[
            ("h.js", "export function onEvent(x){ return x; }\n"),
            (
                "reg.js",
                "import { onEvent } from \"./h.js\";\nconst ROUTES = { e: onEvent };\n",
            ),
        ],
    );
    let nid = out_nid(&out);
    let reg_file = out_id_by_label(&out, "reg.js");
    assert!(out_rels(&out, "indirect_call").contains(&(reg_file, nid["onEvent"].clone())));
}

#[test]
fn typescript_typed_params_and_arrow_consts() {
    let tmp = tempdir().unwrap();
    let out = extract_js_dir(
        tmp.path(),
        &[(
            "t.ts",
            "function handler(x: number): number { return x; }\nconst cb = (): void => {};\nfunction via(pool: Pool): void { pool.submit(handler); }\nconst ROUTES: Record<string, unknown> = { create: handler, run: cb };\n",
        )],
    );
    let nid = out_nid(&out);
    let file_nid = out_id_by_label(&out, "t.ts");
    let indirect = out_rels(&out, "indirect_call");
    assert!(indirect.contains(&(nid["via"].clone(), nid["handler"].clone())));
    assert!(indirect.contains(&(file_nid.clone(), nid["handler"].clone())));
    assert!(indirect.contains(&(file_nid, nid["cb"].clone())));
}
