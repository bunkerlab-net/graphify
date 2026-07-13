//! #1638 — an unresolved bare npm import must not collapse onto an unrelated
//! local file of the same stem.
//!
//! Ports `graphify-py/tests/test_phantom_external_import.py`. `import colors from
//! "tailwindcss/colors"` used to emit an `imports_from` edge to the bare id
//! `colors`, which the graph builder's pre-migration alias index then remapped
//! onto a local `backend/utils/colors.py` — a confident EXTRACTED cross-language
//! phantom. The external-import fallback now namespaces its target with the `ref`
//! prefix, so it can never collide with a local node id; build drops the
//! ref-target as an external reference (it has no node).
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::case_sensitive_file_extension_comparisons
)]

use std::collections::HashSet;
use std::path::Path;

use graphify_build::build_from_json;
use graphify_extract::generic::resolve_js_import_target;
use graphify_extract::{extract, make_id1};
use serde_json::Value;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn write(path: &Path, text: &str) -> TestResult {
    std::fs::create_dir_all(path.parent().ok_or("file has no parent")?)?;
    std::fs::write(path, text)?;
    Ok(())
}

// ── unit: the resolver never returns a bare local-shaped id for an external ──

#[test]
fn unresolved_bare_import_is_ref_namespaced() {
    let (tgt, resolved) =
        resolve_js_import_target("tailwindcss/colors", "frontend/src/SomeChart.tsx");
    assert!(resolved.is_none());
    // Must not be the bare last-segment id that collides with a local `colors` file.
    assert_ne!(tgt, make_id1("colors"));
    assert_ne!(tgt, make_id1("colors.py"));
    assert!(tgt.starts_with("ref"), "target not ref-namespaced: {tgt}");
}

#[test]
fn scoped_package_import_is_ref_namespaced() {
    let (tgt, resolved) = resolve_js_import_target("@scope/utils", "src/thing.ts");
    assert!(resolved.is_none());
    assert_ne!(tgt, make_id1("utils"));
    assert!(tgt.starts_with("ref"), "target not ref-namespaced: {tgt}");
}

// ── end-to-end: the reporter's synthetic monorepo ───────────────────────────

/// Node ids whose `source_file` ends with `suffix`.
fn ids_with_source_suffix(g: &graphify_build::Graph, suffix: &str) -> HashSet<String> {
    g.nodes()
        .filter(|(_, a)| {
            a.get("source_file")
                .and_then(Value::as_str)
                .is_some_and(|s| s.ends_with(suffix))
        })
        .map(|(id, _)| id.clone())
        .collect()
}

fn source_file_of(g: &graphify_build::Graph, id: &str) -> String {
    g.nodes()
        .find(|(nid, _)| nid.as_str() == id)
        .and_then(|(_, a)| a.get("source_file").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

#[test]
fn no_phantom_edge_from_tsx_to_unrelated_python_file() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    let py = root.join("backend/utils/colors.py");
    let tsx = root.join("frontend/src/SomeChart.tsx");
    write(&py, "def hex_to_rgb(value):\n    return (0, 0, 0)\n")?;
    write(
        &tsx,
        "import colors from \"tailwindcss/colors\";\n\nexport const CHART_COLOR = colors.blue[500];\n",
    )?;

    let out = extract(&[py, tsx], Some(root));
    let g = build_from_json(serde_json::to_value(&out)?, false, Some(root))?;

    let py_ids = ids_with_source_suffix(&g, "colors.py");
    assert!(!py_ids.is_empty(), "colors.py should have produced a node");

    // No `imports_from` edge from a TS/TSX source may land on the python file.
    for e in g.edges() {
        if e.attrs.get("relation").and_then(Value::as_str) != Some("imports_from") {
            continue;
        }
        for (endpoint, other) in [(&e.source, &e.target), (&e.target, &e.source)] {
            if py_ids.contains(endpoint) {
                let other_sf = source_file_of(&g, other);
                assert!(
                    !(other_sf.ends_with(".tsx") || other_sf.ends_with(".ts")),
                    "phantom cross-language imports_from edge onto colors.py: {} -> {}",
                    e.source,
                    e.target
                );
            }
        }
    }
    Ok(())
}

#[test]
fn multiple_tsx_files_do_not_all_alias_onto_one_python_file() -> TestResult {
    // The real-world symptom: N unrelated .tsx files doing the same bare import
    // showed up as N imports_from sources on one python module.
    let tmp = tempdir()?;
    let root = tmp.path();
    let py = root.join("backend/utils/colors.py");
    write(&py, "def hex_to_rgb(value):\n    return (0, 0, 0)\n")?;
    let mut paths = vec![py];
    for i in 0..3 {
        let p = root.join(format!("frontend/src/Chart{i}.tsx"));
        write(
            &p,
            &format!(
                "import colors from \"tailwindcss/colors\";\nexport const C{i} = colors.blue;\n"
            ),
        )?;
        paths.push(p);
    }

    let out = extract(&paths, Some(root));
    let g = build_from_json(serde_json::to_value(&out)?, false, Some(root))?;

    let py_ids = ids_with_source_suffix(&g, "colors.py");
    let phantom: Vec<(String, String)> = g
        .edges()
        .filter(|e| e.attrs.get("relation").and_then(Value::as_str) == Some("imports_from"))
        .filter(|e| py_ids.contains(&e.source) || py_ids.contains(&e.target))
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();
    assert!(
        phantom.is_empty(),
        "phantom edges onto colors.py: {phantom:?}"
    );
    Ok(())
}
