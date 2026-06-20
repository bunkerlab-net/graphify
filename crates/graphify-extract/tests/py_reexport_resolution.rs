//! Cross-file Python package re-export resolution.
//!
//! Ports `test_python_package_reexport_resolves_import_and_call_to_origin_symbol`
//! from `graphify-py/tests/test_python_import_resolution.py`: a `pkg/__init__.py`
//! doing `from .foo import Foo as PublicFoo` must let `from pkg import PublicFoo`
//! (and a call through it) resolve to the origin symbol in `pkg/foo.py`.
#![allow(clippy::expect_used)]

use std::error::Error;
use std::path::Path;

use graphify_extract::{ExtractOutput, extract, file_node_id, file_stem, make_id};
use serde_json::Value;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn Error>>;

fn write(path: &Path, text: &str) -> TestResult {
    std::fs::create_dir_all(path.parent().ok_or("path has no parent")?)?;
    std::fs::write(path, text)?;
    Ok(())
}

fn edge_exists(out: &ExtractOutput, source: &str, target: &str, relation: &str) -> bool {
    out.edges.iter().any(|e| {
        e.get("source").and_then(Value::as_str) == Some(source)
            && e.get("target").and_then(Value::as_str) == Some(target)
            && e.get("relation").and_then(Value::as_str) == Some(relation)
    })
}

fn has_file_edge(out: &ExtractOutput, source_rel: &str, target_rel: &str, relation: &str) -> bool {
    edge_exists(
        out,
        &file_node_id(Path::new(source_rel)),
        &file_node_id(Path::new(target_rel)),
        relation,
    )
}

fn has_symbol_edge(out: &ExtractOutput, source_rel: &str, target_file: &str, symbol: &str) -> bool {
    let target = make_id(&[&file_stem(Path::new(target_file)), symbol]);
    edge_exists(
        out,
        &file_node_id(Path::new(source_rel)),
        &target,
        "imports",
    )
}

fn has_symbol_to_symbol_edge(
    out: &ExtractOutput,
    source_file: &str,
    source_symbol: &str,
    target_file: &str,
    target_symbol: &str,
    relation: &str,
) -> bool {
    let source = make_id(&[&file_stem(Path::new(source_file)), source_symbol]);
    let target = make_id(&[&file_stem(Path::new(target_file)), target_symbol]);
    edge_exists(out, &source, &target, relation)
}

#[test]
fn python_package_reexport_resolves_import_and_call_to_origin_symbol() -> TestResult {
    let tmp = tempdir()?;
    let root = tmp.path();
    write(&root.join("pkg/foo.py"), "def Foo():\n    return 1\n")?;
    write(
        &root.join("pkg/__init__.py"),
        "from .foo import Foo as PublicFoo\n",
    )?;
    write(
        &root.join("app.py"),
        "from pkg import PublicFoo\n\ndef X():\n    return PublicFoo()\n",
    )?;
    let out = extract(
        &[
            root.join("pkg/foo.py"),
            root.join("pkg/__init__.py"),
            root.join("app.py"),
        ],
        Some(root),
    );
    assert!(
        has_file_edge(&out, "pkg/__init__.py", "pkg/foo.py", "re_exports"),
        "package __init__ should re_export the origin module"
    );
    assert!(
        has_symbol_edge(&out, "app.py", "pkg/foo.py", "Foo"),
        "consumer should import the origin symbol, not a barrel stub"
    );
    assert!(
        has_symbol_to_symbol_edge(&out, "app.py", "X", "pkg/foo.py", "Foo", "calls"),
        "call through the re-exported alias should target the origin symbol"
    );
    Ok(())
}

#[test]
fn python_subpackage_reexport_resolves_import_and_call_to_origin_symbol() -> TestResult {
    // Same chain as the module-origin case, but the re-exported symbol lives in
    // a *package* (`pkg/subpkg/__init__.py`), so `from .subpkg import Helper`
    // must resolve to the package's `__init__.py`, not a nonexistent `subpkg.py`.
    let tmp = tempdir()?;
    let root = tmp.path();
    write(
        &root.join("pkg/subpkg/__init__.py"),
        "def Helper():\n    return 1\n",
    )?;
    write(
        &root.join("pkg/__init__.py"),
        "from .subpkg import Helper as PublicHelper\n",
    )?;
    write(
        &root.join("app.py"),
        "from pkg import PublicHelper\n\ndef X():\n    return PublicHelper()\n",
    )?;
    let out = extract(
        &[
            root.join("pkg/subpkg/__init__.py"),
            root.join("pkg/__init__.py"),
            root.join("app.py"),
        ],
        Some(root),
    );
    assert!(
        has_file_edge(
            &out,
            "pkg/__init__.py",
            "pkg/subpkg/__init__.py",
            "re_exports"
        ),
        "package __init__ should re_export the origin subpackage"
    );
    assert!(
        has_symbol_edge(&out, "app.py", "pkg/subpkg/__init__.py", "Helper"),
        "consumer should import the origin symbol from the subpackage"
    );
    assert!(
        has_symbol_to_symbol_edge(
            &out,
            "app.py",
            "X",
            "pkg/subpkg/__init__.py",
            "Helper",
            "calls"
        ),
        "call through the re-exported alias should target the subpackage origin symbol"
    );
    Ok(())
}
