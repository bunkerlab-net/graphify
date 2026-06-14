//! 1:1 port of `graphify-py/tests/test_cargo_introspect.py`.
//!
//! Exercises Cargo workspace discovery: internal path/workspace dependencies
//! become `crate_depends_on` edges while registry-only packages stay out of the
//! graph, across virtual workspaces, root packages, globbed members, degenerate
//! manifests, and malformed TOML.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;

use graphify_extract::introspect_cargo;
use serde_json::{Value, json};

/// Write `content` (leading whitespace stripped, matching the Python
/// `content.lstrip()`) to `path`, creating parent dirs as needed.
fn write_manifest(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create dirs");
    }
    std::fs::write(path, content.trim_start()).expect("write manifest");
}

fn node_ids(result: &graphify_extract::CargoIntrospection) -> HashSet<String> {
    result
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// `test_cargo_introspect_workspace_internal_dependency_only`
#[test]
fn workspace_internal_dependency_only() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_manifest(
        &root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"app\", \"core\"]\n",
    );
    write_manifest(
        &root.join("app/Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\ncore = { path = \"../core\" }\nserde = \"1\"\n",
    );
    write_manifest(
        &root.join("core/Cargo.toml"),
        "[package]\nname = \"core\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );

    let result = introspect_cargo(root).expect("introspect");

    assert_eq!(
        node_ids(&result),
        HashSet::from(["crate:app".to_string(), "crate:core".to_string()])
    );
    assert!(!node_ids(&result).contains("crate:serde"));
    assert!(result.nodes.contains(&json!({
        "id": "crate:app",
        "label": "app",
        "source_file": "app/Cargo.toml",
        "source_location": "L1",
    })));
    assert!(result.edges.contains(&json!({
        "source": "crate:app",
        "target": "crate:core",
        "relation": "crate_depends_on",
        "context": "cargo_dependency",
        "weight": 1.0,
        "confidence": "EXTRACTED",
        "source_file": "app/Cargo.toml",
        "source_location": "L1",
    })));
    assert!(!result.edges.iter().any(|e| {
        e.get("source").and_then(Value::as_str) == Some("crate:app")
            && e.get("target").and_then(Value::as_str) == Some("crate:serde")
    }));
}

/// `test_cargo_introspect_malformed_toml_reports_parser_error`
#[test]
fn malformed_toml_reports_parser_error() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_manifest(&root.join("Cargo.toml"), "[package\nname = \"broken\"\n");

    let err = introspect_cargo(root).expect_err("malformed TOML must error");
    assert!(
        matches!(err, graphify_extract::CargoIntrospectError::Toml { .. }),
        "expected a TOML parse error, got {err:?}"
    );
}

/// `test_cargo_introspect_degenerate_manifests_return_empty_or_skip_bad_deps`
#[test]
fn degenerate_manifests_return_empty_or_skip_bad_deps() {
    let tmp = tempfile::tempdir().unwrap();

    // Empty manifest → no nodes, no edges.
    let empty = tmp.path().join("empty");
    write_manifest(&empty.join("Cargo.toml"), "");
    let empty_result = introspect_cargo(&empty).expect("introspect empty");
    assert!(empty_result.nodes.is_empty());
    assert!(empty_result.edges.is_empty());

    // Package without a name → no crate node.
    let nameless = tmp.path().join("nameless");
    write_manifest(
        &nameless.join("Cargo.toml"),
        "[package]\nversion = \"0.1.0\"\n",
    );
    let nameless_result = introspect_cargo(&nameless).expect("introspect nameless");
    assert!(nameless_result.nodes.is_empty());
    assert!(nameless_result.edges.is_empty());

    // Scalar (non-table) dependencies are ignored, the crate node still appears.
    let scalar = tmp.path().join("scalar-dependencies");
    write_manifest(
        &scalar.join("Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\ndependencies = \"not-a-table\"\n",
    );
    let scalar_result = introspect_cargo(&scalar).expect("introspect scalar");
    assert_eq!(
        scalar_result.nodes,
        vec![json!({
            "id": "crate:app",
            "label": "app",
            "source_file": "Cargo.toml",
            "source_location": "L1",
        })]
    );
    assert!(scalar_result.edges.is_empty());
}

/// `test_cargo_introspect_old_manifest_keeps_internal_path_dep_and_skips_external`
#[test]
fn old_manifest_keeps_internal_path_dep_and_skips_external() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_manifest(
        &root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"legacy\", \"internal\"]\n",
    );
    write_manifest(
        &root.join("legacy/Cargo.toml"),
        "[package]\nname = \"legacy\"\nversion = \"0.1.0\"\n\n[dependencies]\nrand = \"0.8\"\ninternal = { path = \"../internal\" }\n",
    );
    write_manifest(
        &root.join("internal/Cargo.toml"),
        "[package]\nname = \"internal\"\nversion = \"0.1.0\"\n",
    );

    let result = introspect_cargo(root).expect("introspect");
    assert_eq!(
        node_ids(&result),
        HashSet::from(["crate:legacy".to_string(), "crate:internal".to_string()])
    );
    assert!(!node_ids(&result).contains("crate:rand"));
    assert_eq!(result.edges.len(), 1);
    let pairs: HashSet<(String, String)> = result
        .edges
        .iter()
        .map(|e| {
            (
                e["source"].as_str().unwrap().to_string(),
                e["target"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(pairs.contains(&("crate:legacy".to_string(), "crate:internal".to_string())));
    assert!(!pairs.contains(&("crate:legacy".to_string(), "crate:rand".to_string())));
}

/// `test_cargo_introspect_modern_virtual_and_root_package_workspaces`
#[test]
fn modern_virtual_and_root_package_workspaces() {
    let tmp = tempfile::tempdir().unwrap();

    // Virtual workspace with globbed members and `{ workspace = true }` deps.
    let virtual_root = tmp.path().join("virtual");
    write_manifest(
        &virtual_root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.dependencies]\nbeta = { path = \"crates/beta\" }\nserde = \"1\"\n",
    );
    write_manifest(
        &virtual_root.join("crates/alpha/Cargo.toml"),
        "[package]\nname = \"alpha\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nbeta = { workspace = true }\nserde = { workspace = true }\n",
    );
    write_manifest(
        &virtual_root.join("crates/beta/Cargo.toml"),
        "[package]\nname = \"beta\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );

    let virtual_result = introspect_cargo(&virtual_root).expect("introspect virtual");
    assert_eq!(
        node_ids(&virtual_result),
        HashSet::from(["crate:alpha".to_string(), "crate:beta".to_string()])
    );
    assert_eq!(virtual_result.nodes.len(), 2);
    assert_eq!(virtual_result.edges.len(), 1);
    assert!(virtual_result.edges.contains(&json!({
        "source": "crate:alpha",
        "target": "crate:beta",
        "relation": "crate_depends_on",
        "context": "cargo_dependency",
        "weight": 1.0,
        "confidence": "EXTRACTED",
        "source_file": "crates/alpha/Cargo.toml",
        "source_location": "L1",
    })));

    // Root-package workspace: the root manifest is itself a crate.
    let package_root = tmp.path().join("package-root");
    write_manifest(
        &package_root.join("Cargo.toml"),
        "[package]\nname = \"root_pkg\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"crates/*\"]\n",
    );
    write_manifest(
        &package_root.join("crates/member/Cargo.toml"),
        "[package]\nname = \"member\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nroot_pkg = { path = \"../..\" }\n",
    );

    let package_result = introspect_cargo(&package_root).expect("introspect package root");
    assert_eq!(
        node_ids(&package_result),
        HashSet::from(["crate:root_pkg".to_string(), "crate:member".to_string()])
    );
    assert_eq!(package_result.nodes.len(), 2);
    assert_eq!(package_result.edges.len(), 1);
    assert!(package_result.edges.contains(&json!({
        "source": "crate:member",
        "target": "crate:root_pkg",
        "relation": "crate_depends_on",
        "context": "cargo_dependency",
        "weight": 1.0,
        "confidence": "EXTRACTED",
        "source_file": "crates/member/Cargo.toml",
        "source_location": "L1",
    })));
}

/// `test_cargo_introspect_large_workspace_dependency_chain`
#[test]
fn large_workspace_dependency_chain() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let crate_count = 200_usize;
    write_manifest(
        &root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n",
    );

    for index in 0..crate_count {
        let name = format!("crate_{index:03}");
        let mut manifest = format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n");
        if index > 0 {
            let prev = format!("crate_{:03}", index - 1);
            let _ = write!(
                manifest,
                "\n[dependencies]\n{prev} = {{ path = \"../{prev}\" }}\n"
            );
        }
        write_manifest(
            &root.join("crates").join(&name).join("Cargo.toml"),
            &manifest,
        );
    }

    let result = introspect_cargo(root).expect("introspect large");
    assert_eq!(result.nodes.len(), crate_count);
    assert_eq!(result.edges.len(), crate_count - 1);
}
