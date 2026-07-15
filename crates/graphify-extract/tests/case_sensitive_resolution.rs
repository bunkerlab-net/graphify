//! Cross-file name resolution respects case in case-sensitive languages (#1581).
//!
//! Ports `graphify-py/tests/test_case_sensitive_resolution.py`. Case is semantic
//! in most languages: `Path` (a class), `PATH` (an env var), and `path` (a
//! variable) are distinct. Cross-file resolution used to fold case for every
//! language, so `from pathlib import Path` resolved to a shell script's
//! `export PATH=...` node — turning one shell variable into the corpus's #1
//! god-node. These pin: case-sensitive languages match by exact case, while
//! genuinely case-insensitive languages (PHP) still fold.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::path::Path;

use graphify_extract::{Edge, ExtractOutput, Node, extract};
use serde_json::Value;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Write `files` under `root` and run the cross-file extractor over them.
fn extract_files(root: &Path, files: &[(&str, &str)]) -> ExtractResult {
    let mut paths = Vec::with_capacity(files.len());
    for (name, body) in files {
        let p = root.join(name);
        std::fs::create_dir_all(p.parent().ok_or("file has no parent")?)?;
        std::fs::write(&p, body)?;
        paths.push(p);
    }
    Ok(extract(&paths, Some(root)))
}

type ExtractResult = Result<ExtractOutput, Box<dyn std::error::Error>>;

/// A code node with the given `id` / `label` / `source_file` (empty = sourceless stub).
fn node(id: &str, label: &str, source_file: &str) -> Node {
    Node {
        id: id.to_string(),
        label: label.to_string(),
        file_type: "code".to_string(),
        source_file: source_file.to_string(),
        source_location: None,
        origin_file: None,
        node_type: None,
        metadata: None,
    }
}

/// A minimal edge with the given endpoints/relation.
fn edge(source: &str, target: &str, relation: &str) -> Edge {
    Edge {
        source: source.to_string(),
        target: target.to_string(),
        relation: relation.to_string(),
        confidence: String::new(),
        source_file: String::new(),
        source_location: None,
        weight: 0.0,
        context: None,
        confidence_score: None,
        external: false,
        deferred: false,
        metadata: None,
    }
}

/// Node id whose label is exactly `label`, if any.
fn nid_with_label(out: &ExtractOutput, label: &str) -> Option<String> {
    out.nodes
        .iter()
        .find(|n| n.get("label").and_then(Value::as_str) == Some(label))
        .and_then(|n| n.get("id").and_then(Value::as_str))
        .map(str::to_string)
}

/// `label` of the node with id `id`, or `""`.
fn label_of(out: &ExtractOutput, id: &str) -> String {
    out.nodes
        .iter()
        .find(|n| n.get("id").and_then(Value::as_str) == Some(id))
        .and_then(|n| n.get("label").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

/// Set of `(source_label, target_label)` for edges with relation `calls`.
fn call_pairs(out: &ExtractOutput) -> HashSet<(String, String)> {
    out.edges
        .iter()
        .filter(|e| e.get("relation").and_then(Value::as_str) == Some("calls"))
        .filter_map(|e| {
            let s = e.get("source").and_then(Value::as_str)?;
            let t = e.get("target").and_then(Value::as_str)?;
            Some((label_of(out, s), label_of(out, t)))
        })
        .collect()
}

#[test]
fn python_path_does_not_resolve_to_shell_path() -> TestResult {
    let tmp = tempdir()?;
    let out = extract_files(
        tmp.path(),
        &[
            ("run.sh", "export PATH=/usr/local/bin:$PATH\n"),
            (
                "mod.py",
                "from pathlib import Path\ndef load(p: Path) -> Path:\n    return Path(p)\ndef other():\n    return load(Path('x'))\n",
            ),
        ],
    )?;
    let path_nid = nid_with_label(&out, "PATH").expect("shell PATH node present");
    // No edge from the Python functions may land on the shell PATH node.
    let false_edges: Vec<_> = out
        .edges
        .iter()
        .filter(|e| {
            e.get("target").and_then(Value::as_str) == Some(path_nid.as_str())
                && e.get("source")
                    .and_then(Value::as_str)
                    .map(|s| label_of(&out, s))
                    .is_some_and(|l| l.starts_with("load") || l.starts_with("other"))
        })
        .collect();
    assert!(
        false_edges.is_empty(),
        "Python Path leaked onto shell PATH: {false_edges:?}"
    );
    // PATH keeps only its own `defines` edge (from run.sh), not a false super-hub.
    let incoming = out
        .edges
        .iter()
        .filter(|e| e.get("target").and_then(Value::as_str) == Some(path_nid.as_str()))
        .count();
    assert!(
        incoming <= 1,
        "PATH became a super-hub: {incoming} incoming"
    );
    Ok(())
}

#[test]
fn stub_rewire_respects_case_in_case_sensitive_language() {
    // A sourceless type-reference stub `Path` (from a cross-language inheritance
    // placeholder) must NOT rewire onto a case-differing `PATH` definition in a
    // case-sensitive language (#1581). The old case-folding index collapsed the
    // two, manufacturing a false edge / super-hub. `.rs` is case-sensitive, so no
    // fold fallback applies and the stub stays unresolved.
    let mut nodes = vec![
        node("stub", "Path", ""),          // unresolved reference (no source_file)
        node("real", "PATH", "consts.rs"), // case-differing real definition
    ];
    let mut edges = vec![edge("user", "stub", "inherits")];
    graphify_extract::postprocess::rewire_unique_stub_nodes(&mut nodes, &mut edges);
    assert!(
        nodes.iter().any(|n| n.id == "stub"),
        "a case-differing `Path` stub must not rewire onto `PATH`"
    );
    assert_eq!(
        edges[0].target, "stub",
        "the edge must stay on the unresolved stub"
    );
}

#[test]
fn stub_rewire_folds_case_in_case_insensitive_language() {
    // PHP resolves identifiers case-insensitively, so a `greet` stub legitimately
    // folds onto a `Greet` definition in a `.php` file — the case-insensitive
    // fallback the fix deliberately preserves (#1581).
    let mut nodes = vec![node("stub", "greet", ""), node("real", "Greet", "lib.php")];
    let mut edges = vec![edge("user", "stub", "inherits")];
    graphify_extract::postprocess::rewire_unique_stub_nodes(&mut nodes, &mut edges);
    assert!(
        nodes.iter().all(|n| n.id != "stub"),
        "a PHP `greet` stub should fold-rewire onto `Greet`"
    );
    assert_eq!(edges[0].target, "real");
}

#[test]
fn exact_case_cross_file_still_resolves() -> TestResult {
    let tmp = tempdir()?;
    let out = extract_files(
        tmp.path(),
        &[
            ("h.py", "def helper():\n    return 1\n"),
            (
                "m.py",
                "from h import helper\ndef go():\n    return helper()\n",
            ),
        ],
    )?;
    let pairs = call_pairs(&out);
    assert!(
        pairs.contains(&("go()".to_string(), "helper()".to_string())),
        "exact-case call must still resolve: {pairs:?}"
    );
    Ok(())
}

#[test]
fn php_case_insensitive_resolution_preserved() -> TestResult {
    let tmp = tempdir()?;
    let out = extract_files(
        tmp.path(),
        &[
            ("lib.php", "<?php\nfunction Greet() { return 1; }\n"),
            ("main.php", "<?php\nfunction run() { return greet(); }\n"),
        ],
    )?;
    let pairs = call_pairs(&out);
    assert!(
        pairs.contains(&("run()".to_string(), "Greet()".to_string())),
        "PHP identifiers are case-insensitive; fold must still apply: {pairs:?}"
    );
    Ok(())
}

#[test]
fn same_family_call_does_not_fold_case() -> TestResult {
    // Same-language (python↔python, so the cross-family guard never fires): only
    // exact-case matching at the CALL resolver prevents a `Path()` call binding to
    // a `PATH()` definition. Folding would collapse them and manufacture an edge
    // (#1581) — this isolates the call-site fix from the language-family guard.
    let tmp = tempdir()?;
    let out = extract_files(
        tmp.path(),
        &[
            ("caller.py", "def go():\n    return Path()\n"),
            ("defs.py", "def PATH():\n    return 1\n"),
        ],
    )?;
    let pairs = call_pairs(&out);
    assert!(
        !pairs.contains(&("go()".to_string(), "PATH()".to_string())),
        "a `Path()` call must not fold onto a same-family `PATH()` def: {pairs:?}"
    );
    Ok(())
}
