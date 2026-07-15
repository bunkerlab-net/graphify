//! #1659 — a JS/TS call with no local definition and no import must not bind to
//! a same-named export in an unrelated package that was never imported.
//!
//! Ports `graphify-py/tests/test_phantom_cross_package_call.py`. JS/TS modules
//! have no implicit cross-module scope: a call into another file is real only if
//! the caller imported it. The resolver used to fall back to any lone same-named
//! export repo-wide (INFERRED/0.8), fabricating cross-package dependencies. The
//! fix gates JS/TS cross-file call attribution on import evidence; other
//! languages keep the single-candidate resolution (headers, Ruby autoload,
//! same-package implicit scope).
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use graphify_extract::extract;
use serde_json::Value;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn write(path: &Path, text: &str) -> TestResult {
    std::fs::create_dir_all(path.parent().ok_or("file has no parent")?)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Set of `(source_label, target_label, confidence)` for `calls` edges.
fn call_triples(root: &Path, paths: &[PathBuf]) -> HashSet<(String, String, String)> {
    let out = extract(paths, Some(root));
    let label_of = |id: &str| -> String {
        out.nodes
            .iter()
            .find(|n| n.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|n| n.get("label").and_then(Value::as_str))
            .unwrap_or("")
            .to_string()
    };
    out.edges
        .iter()
        .filter(|e| e.get("relation").and_then(Value::as_str) == Some("calls"))
        .filter_map(|e| {
            let s = e.get("source").and_then(Value::as_str)?;
            let t = e.get("target").and_then(Value::as_str)?;
            let c = e
                .get("confidence")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some((label_of(s), label_of(t), c))
        })
        .collect()
}

#[test]
fn unimported_cross_package_call_emits_no_edge() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("pkg-a/src/index.ts"),
        "declare function validate(x: number): boolean;\nexport function run(x: number): boolean { return validate(x); }\n",
    )?;
    write(
        &root.join("pkg-b/src/index.ts"),
        "export function validate(name: string): boolean { return name.length > 0; }\n",
    )?;
    let calls = call_triples(
        root,
        &[
            root.join("pkg-a/src/index.ts"),
            root.join("pkg-b/src/index.ts"),
        ],
    );
    assert!(
        !calls
            .iter()
            .any(|(s, t, _)| s.contains("run") && t.contains("validate")),
        "unimported cross-package call resolved: {calls:?}"
    );
    Ok(())
}

/// #1659 + #1671: the phantom-edge guard is case-insensitive, matching
/// `get_extractor`'s dispatch. Uppercase `.TS` files are still JS/TS, so an
/// unimported cross-package call must stay unresolved (a lowercase-only suffix
/// check would leak the phantom edge here).
#[test]
fn unimported_cross_package_call_uppercase_ext_emits_no_edge() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("pkg-a/src/index.TS"),
        "declare function validate(x: number): boolean;\nexport function run(x: number): boolean { return validate(x); }\n",
    )?;
    write(
        &root.join("pkg-b/src/index.TS"),
        "export function validate(name: string): boolean { return name.length > 0; }\n",
    )?;
    let calls = call_triples(
        root,
        &[
            root.join("pkg-a/src/index.TS"),
            root.join("pkg-b/src/index.TS"),
        ],
    );
    assert!(
        !calls
            .iter()
            .any(|(s, t, _)| s.contains("run") && t.contains("validate")),
        "uppercase-ext unimported cross-package call resolved: {calls:?}"
    );
    Ok(())
}

#[test]
fn many_files_do_not_collapse_onto_one_export() -> TestResult {
    // The real-world symptom: N packages importing nothing all showed edges to a
    // single package that exported a generically-named symbol.
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("proto/index.ts"),
        "export function encode(x: string): string { return x; }\n",
    )?;
    let mut paths = vec![root.join("proto/index.ts")];
    for i in 0..4 {
        let p = root.join(format!("svc{i}/index.ts"));
        write(
            &p,
            &format!(
                "declare function encode(x: string): string;\nexport function use{i}(x: string) {{ return encode(x); }}\n"
            ),
        )?;
        paths.push(p);
    }
    let calls = call_triples(root, &paths);
    assert!(
        !calls.iter().any(|(_, t, _)| t.contains("encode")),
        "unimported calls collapsed onto the shared export: {calls:?}"
    );
    Ok(())
}

#[test]
fn imported_cross_file_call_still_resolves() -> TestResult {
    // A real import must still resolve (and be promoted to EXTRACTED).
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("a.ts"),
        "import { validate } from \"./b\";\nexport function run(x: number) { return validate(x); }\n",
    )?;
    write(
        &root.join("b.ts"),
        "export function validate(name: string): boolean { return name.length > 0; }\n",
    )?;
    let calls = call_triples(root, &[root.join("a.ts"), root.join("b.ts")]);
    let resolved: Vec<_> = calls
        .iter()
        .filter(|(s, t, _)| s.contains("run") && t.contains("validate"))
        .collect();
    assert!(
        !resolved.is_empty(),
        "imported call did not resolve: {calls:?}"
    );
    assert!(
        resolved.iter().all(|(_, _, c)| c == "EXTRACTED"),
        "imported call must be EXTRACTED: {resolved:?}"
    );
    Ok(())
}

#[test]
fn same_file_call_unaffected() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("s.ts"),
        "function helper() { return 1; }\nexport function main() { return helper(); }\n",
    )?;
    let calls = call_triples(root, &[root.join("s.ts")]);
    assert!(
        calls
            .iter()
            .any(|(s, t, _)| s.contains("main") && t.contains("helper")),
        "same-file call must still resolve: {calls:?}"
    );
    Ok(())
}

#[test]
fn non_js_single_candidate_cross_file_still_resolves() -> TestResult {
    // The gate is JS/TS-only. Ruby (autoload, no require) legitimately calls a
    // lone same-named function across files without an import — keep the
    // single-candidate resolution for it.
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("helper.rb"),
        "def transform(data)\n  data.upcase\nend\n",
    )?;
    write(
        &root.join("main.rb"),
        "def handle(v)\n  transform(v)\nend\n",
    )?;
    let calls = call_triples(root, &[root.join("main.rb"), root.join("helper.rb")]);
    assert!(
        calls
            .iter()
            .any(|(s, t, _)| s.contains("handle") && t.contains("transform")),
        "Ruby cross-file single-candidate call must still resolve: {calls:?}"
    );
    Ok(())
}
