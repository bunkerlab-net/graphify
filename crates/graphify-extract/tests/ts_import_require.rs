//! The TypeScript import-equals form `import x = require("./m")` (9811def).
//!
//! Ports `graphify-py/tests/test_ts_import_require.py` (v0.9.12 state, including
//! the e2ef4ef ref-namespaced external target). The module string sits inside an
//! `import_require_clause`, not as a direct child of the statement, so the naive
//! string scan missed it and emitted no import edge.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::Path;

use graphify_extract::{ExtractOutput, extract, file_node_id, make_id1};
use serde_json::Value;
use tempfile::tempdir;

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(path, text).expect("write");
}

/// Set of `(source_id, target_id, relation)` edge triples.
fn edge_triples(out: &ExtractOutput) -> Vec<(String, String, String)> {
    out.edges
        .iter()
        .filter_map(|e| {
            Some((
                e.get("source").and_then(Value::as_str)?.to_string(),
                e.get("target").and_then(Value::as_str)?.to_string(),
                e.get("relation").and_then(Value::as_str)?.to_string(),
            ))
        })
        .collect()
}

fn has_file_edge(out: &ExtractOutput, source: &str, target: &str) -> bool {
    let s = file_node_id(Path::new(source));
    let t = file_node_id(Path::new(target));
    edge_triples(out)
        .iter()
        .any(|(es, et, er)| *es == s && *et == t && er == "imports_from")
}

#[test]
fn import_require_relative_emits_file_edge() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    write(
        &root.join("src/lib/legacy.ts"),
        "export function foo(): number { return 1 }\n",
    );
    write(
        &root.join("src/lib/consumer.ts"),
        "import legacy = require(\"./legacy\");\nconst n = legacy.foo();\n",
    );
    let out = extract(
        &[
            root.join("src/lib/legacy.ts"),
            root.join("src/lib/consumer.ts"),
        ],
        Some(root),
    );
    assert!(has_file_edge(
        &out,
        "src/lib/consumer.ts",
        "src/lib/legacy.ts"
    ));
}

#[test]
fn import_require_single_quotes() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    write(&root.join("src/util.ts"), "export const V = 1\n");
    write(
        &root.join("src/main.ts"),
        "import util = require('./util');\nexport const x = util.V;\n",
    );
    let out = extract(
        &[root.join("src/util.ts"), root.join("src/main.ts")],
        Some(root),
    );
    assert!(has_file_edge(&out, "src/main.ts", "src/util.ts"));
}

#[test]
fn import_require_bare_module_targets_ref_stub() {
    // A bare module (`require("fs")`) is external → a ref-namespaced stub edge,
    // NOT the bare `make_id("fs")` id that would collide with a local `fs.*` via
    // build's alias index (#1638). Parity with the ESM external path.
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    write(
        &root.join("src/io.ts"),
        "import fs = require(\"fs\");\nexport const data = fs.readFileSync(\"x\");\n",
    );
    let out = extract(&[root.join("src/io.ts")], Some(root));
    let src = file_node_id(Path::new("src/io.ts"));
    let import_targets: Vec<String> = edge_triples(&out)
        .into_iter()
        .filter(|(s, _, r)| *s == src && r == "imports_from")
        .map(|(_, t, _)| t)
        .collect();
    assert!(
        !import_targets.is_empty(),
        "bare-module import-equals should still emit an external stub edge"
    );
    assert!(
        !import_targets.contains(&make_id1("fs")),
        "must not use the bare collision-prone id: {import_targets:?}"
    );
    assert!(
        import_targets.iter().any(|t| t.starts_with("ref")),
        "external target must be ref-namespaced: {import_targets:?}"
    );
}

#[test]
fn import_require_parity_with_namespace_import() {
    // `import x = require("./m")` must produce the same non-`contains` edges as
    // `import * as x from "./m"`.
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    write(&root.join("a/dep.ts"), "export function f() {}\n");
    write(
        &root.join("a/via_require.ts"),
        "import dep = require(\"./dep\");\ndep.f();\n",
    );
    write(
        &root.join("a/via_esm.ts"),
        "import * as dep from \"./dep\";\ndep.f();\n",
    );
    let out = extract(
        &[
            root.join("a/dep.ts"),
            root.join("a/via_require.ts"),
            root.join("a/via_esm.ts"),
        ],
        Some(root),
    );
    let edges_from = |source: &str| -> Vec<(String, String)> {
        let src = file_node_id(Path::new(source));
        let mut v: Vec<(String, String)> = edge_triples(&out)
            .into_iter()
            .filter(|(s, _, r)| *s == src && r != "contains")
            .map(|(_, t, r)| (t, r))
            .collect();
        v.sort();
        v
    };
    assert!(has_file_edge(&out, "a/via_require.ts", "a/dep.ts"));
    assert_eq!(edges_from("a/via_require.ts"), edges_from("a/via_esm.ts"));
}

#[test]
fn esm_imports_unaffected() {
    // The restructured string scan must not change ESM handling: file-level edge
    // + named-import symbol edge both still emitted.
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path();
    write(&root.join("src/bar.ts"), "export class Bar {}\n");
    write(
        &root.join("src/app.ts"),
        "import { Bar } from \"./bar\";\nexport const b = new Bar();\n",
    );
    let out = extract(
        &[root.join("src/bar.ts"), root.join("src/app.ts")],
        Some(root),
    );
    assert!(has_file_edge(&out, "src/app.ts", "src/bar.ts"));
    let src = file_node_id(Path::new("src/app.ts"));
    let has_symbol_edge = edge_triples(&out)
        .iter()
        .any(|(s, _, r)| *s == src && r == "imports");
    assert!(
        has_symbol_edge,
        "named-import symbol edge should still be emitted"
    );
}
