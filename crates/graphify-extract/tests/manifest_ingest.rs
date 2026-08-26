//! Parity tests for deterministic package-manifest ingestion (#1377), ported
//! from `graphify-py/tests/test_manifest_ingest.py`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use graphify_build::build_from_json;
use graphify_extract::{Node, extract, extract_package_manifest};

fn write_file(root: &Path, rel: &str, text: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("create_dir_all");
    std::fs::write(&p, text).expect("write");
    p
}

fn node_type(n: &Node) -> Option<&str> {
    n.metadata.as_ref()?.get("type")?.as_str()
}

fn node_meta_str<'a>(n: &'a Node, key: &str) -> Option<&'a str> {
    n.metadata.as_ref()?.get(key)?.as_str()
}

fn pkg_node(r: &graphify_extract::FileResult) -> &Node {
    r.nodes
        .iter()
        .find(|n| node_type(n) == Some("package"))
        .expect("package node")
}

fn dep_targets(r: &graphify_extract::FileResult) -> HashSet<&str> {
    r.edges
        .iter()
        .filter(|e| e.relation == "depends_on")
        .map(|e| e.target.as_str())
        .collect()
}

#[test]
fn apm_parses_name_and_deps() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write_file(
        tmp.path(),
        "apm.yml",
        "name: my-pkg\nversion: 1.2.3\ndependencies:\n  - dep-a\n  - dep-b\n",
    );
    let r = extract_package_manifest(&p);
    let pkg = pkg_node(&r);
    assert_eq!(pkg.label, "my-pkg");
    assert_eq!(node_meta_str(pkg, "version"), Some("1.2.3"));
    let deps = dep_targets(&r);
    assert!(
        deps.contains("pkg_dep_a") && deps.contains("pkg_dep_b"),
        "{deps:?}"
    );
}

#[test]
fn pyproject_parses_pep508_deps() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write_file(
        tmp.path(),
        "pyproject.toml",
        "[project]\nname = \"cool-lib\"\nversion = \"0.1\"\n\
         dependencies = [\"requests>=2.0\", \"rich[jupyter]==13.0\", \"tomli; python_version<'3.11'\"]\n",
    );
    let r = extract_package_manifest(&p);
    assert_eq!(pkg_node(&r).label, "cool-lib");
    let deps = dep_targets(&r);
    // versions / extras / markers stripped.
    assert!(
        deps.contains("pkg_requests") && deps.contains("pkg_rich") && deps.contains("pkg_tomli"),
        "{deps:?}"
    );
}

#[test]
fn gomod_parses_module_and_requires() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write_file(
        tmp.path(),
        "go.mod",
        "module example.com/me/app\n\ngo 1.22\n\nrequire (\n\
         \tgithub.com/x/y v1.2.3\n\tgithub.com/a/b v0.4.0\n)\n",
    );
    let r = extract_package_manifest(&p);
    assert_eq!(pkg_node(&r).label, "example.com/me/app");
    let deps = dep_targets(&r);
    assert!(
        deps.contains("pkg_github_com_x_y") && deps.contains("pkg_github_com_a_b"),
        "{deps:?}"
    );
}

#[test]
fn pom_parses_artifact_and_deps() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write_file(
        tmp.path(),
        "pom.xml",
        "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n\
         \x20 <groupId>com.acme</groupId>\n  <artifactId>widget</artifactId>\n  <version>2.0</version>\n\
         \x20 <dependencies>\n    <dependency><groupId>org.lib</groupId><artifactId>core</artifactId></dependency>\n\
         \x20 </dependencies>\n</project>\n",
    );
    let r = extract_package_manifest(&p);
    assert_eq!(pkg_node(&r).label, "com.acme:widget");
    assert!(
        dep_targets(&r).contains("pkg_org_lib_core"),
        "{:?}",
        dep_targets(&r)
    );
}

#[test]
fn apm_dependency_collapses_to_single_canonical_node() {
    // #1377: a package referenced by N manifests is ONE node.
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().join("packages");
    let mut files = vec![
        write_file(
            &base,
            "core/apm.yml",
            "name: coding-standards-core\nversion: 1.0.4\n",
        ),
        write_file(
            &base,
            "csharp/apm.yml",
            "name: coding-standards-csharp\ndependencies:\n  - coding-standards-core\n",
        ),
        write_file(
            &base,
            "python/apm.yml",
            "name: coding-standards-python\ndependencies:\n  coding-standards-core: \">=1.0\"\n",
        ),
    ];
    files.sort();
    let res = extract(&files, Some(tmp.path()));

    let core: Vec<&_> = res
        .nodes
        .iter()
        .filter(|n| {
            n.get("metadata")
                .and_then(|m| m.get("type"))
                .and_then(|v| v.as_str())
                == Some("package")
                && n.get("label").and_then(|v| v.as_str()) == Some("coding-standards-core")
        })
        .collect();
    assert_eq!(
        core.len(),
        1,
        "core package must be a single canonical node"
    );
    assert_eq!(
        core[0].get("id").and_then(|v| v.as_str()),
        Some("pkg_coding_standards_core")
    );
    assert_ne!(
        core[0]
            .get("source_file")
            .and_then(|v| v.as_str())
            .unwrap_or(""),
        ""
    );

    let dep_edges = res
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("depends_on")
                && e.get("target").and_then(|v| v.as_str()) == Some("pkg_coding_standards_core")
        })
        .count();
    assert_eq!(dep_edges, 2, "both dependents point at the one core node");

    let g = build_from_json(serde_json::to_value(&res).unwrap(), false, None).expect("build");
    assert!(g.contains_node("pkg_coding_standards_core"));
    let core_count = g
        .nodes()
        .filter(|(_, attrs)| {
            attrs.get("label").and_then(|v| v.as_str()) == Some("coding-standards-core")
        })
        .count();
    assert_eq!(core_count, 1);
}

#[test]
fn external_dependency_edge_pruned_not_orphaned() {
    // A dep whose manifest isn't in the corpus: the edge dangles and build prunes
    // it, with no fabricated external node.
    let tmp = tempfile::tempdir().unwrap();
    let p = write_file(
        tmp.path(),
        "apm.yml",
        "name: leaf\ndependencies:\n  - some-external-pkg\n",
    );
    let res = extract(&[p], Some(tmp.path()));
    let g = build_from_json(serde_json::to_value(&res).unwrap(), false, None).expect("build");
    assert!(!g.contains_node("pkg_some_external_pkg"));
    assert!(
        g.nodes()
            .any(|(_, attrs)| attrs.get("label").and_then(|v| v.as_str()) == Some("leaf")),
        "leaf package node missing"
    );
}

#[test]
fn malformed_manifest_does_not_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let p = write_file(tmp.path(), "pom.xml", "<project><not closed");
    let r = extract_package_manifest(&p); // parse error -> empty, no panic
    assert!(r.nodes.is_empty() && r.edges.is_empty());
}
