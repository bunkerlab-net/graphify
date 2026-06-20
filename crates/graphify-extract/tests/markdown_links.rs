//! Parity tests for markdown link edges (#1376), ported from
//! `graphify-py/tests/test_languages.py` (`test_markdown_link_*`).

use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};

use graphify_extract::{extract, extract_markdown};

type TestResult = Result<(), Box<dyn Error>>;

/// A hub doc linking to sibling docs, plus those docs (#1376).
fn md_link_fixture(root: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let pkg = root.join("packages").join("coding-standards-csharp");
    std::fs::create_dir_all(&pkg)?;
    std::fs::write(
        pkg.join("index.md"),
        "# C# Coding Standards\n\n\
         | Topic | Doc |\n| --- | --- |\n\
         | Repository | [C# Repository Standards](./repository.md) |\n\
         | HTTP Client | [C# HTTP Client Standards](http-client.md) |\n\
         | Unit Tests | [C# Unit Test Standards](unit-tests.md) |\n\n\
         See also [external](https://example.com/x) and ![logo](./logo.png).\n\
         Anchor: [section](./repository.md#setup).\n\
         Wikilink: [[http-client]].\n",
    )?;
    std::fs::write(
        pkg.join("repository.md"),
        "# C# Repository Standards\nContent.\n",
    )?;
    std::fs::write(
        pkg.join("http-client.md"),
        "# C# HTTP Client Standards\nContent.\n",
    )?;
    std::fs::write(
        pkg.join("unit-tests.md"),
        "# C# Unit Test Standards\nContent.\n",
    )?;
    Ok(pkg)
}

fn md_paths(pkg: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(pkg)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    paths.sort();
    Ok(paths)
}

#[test]
fn markdown_link_edges_emitted() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let pkg = md_link_fixture(tmp.path())?;
    let r = extract_markdown(&pkg.join("index.md"));
    let refs: Vec<&str> = r
        .edges
        .iter()
        .filter(|e| e.relation == "references")
        .map(|e| e.target.as_str())
        .collect();
    // repository, http-client, unit-tests — each exactly once (deduped despite
    // the anchor link and wikilink pointing at repository/http-client again).
    assert_eq!(refs.len(), 3, "expected 3 reference edges, got {refs:?}");
    assert!(refs.iter().any(|t| t.contains("repository")), "{refs:?}");
    assert!(refs.iter().any(|t| t.contains("http_client")), "{refs:?}");
    assert!(refs.iter().any(|t| t.contains("unit_tests")), "{refs:?}");
    Ok(())
}

#[test]
fn markdown_link_skips_external_and_images() -> TestResult {
    let tmp = tempfile::tempdir()?;
    let pkg = md_link_fixture(tmp.path())?;
    let r = extract_markdown(&pkg.join("index.md"));
    for e in r.edges.iter().filter(|e| e.relation == "references") {
        assert!(
            !e.target.contains("example"),
            "external leaked: {}",
            e.target
        );
        assert!(!e.target.contains("logo"), "image leaked: {}", e.target);
    }
    Ok(())
}

#[test]
fn markdown_link_edges_resolve_to_real_nodes() -> TestResult {
    // End-to-end: after extract()'s ID remap, link targets are real doc nodes,
    // so the hub doc gains edges into existing nodes instead of ghost nodes.
    let tmp = tempfile::tempdir()?;
    let pkg = md_link_fixture(tmp.path())?;
    let paths = md_paths(&pkg)?;
    let res = extract(&paths, Some(tmp.path()));

    let node_ids: HashSet<&str> = res
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(|v| v.as_str()))
        .collect();
    let ref_targets: Vec<&str> = res
        .edges
        .iter()
        .filter(|e| e.get("relation").and_then(|v| v.as_str()) == Some("references"))
        .filter_map(|e| e.get("target").and_then(|v| v.as_str()))
        .collect();
    assert!(
        !ref_targets.is_empty(),
        "expected reference edges after full extract"
    );
    for target in &ref_targets {
        assert!(
            node_ids.contains(target),
            "link target is a ghost node: {target}"
        );
    }

    let index_id = res
        .nodes
        .iter()
        .find(|n| n.get("label").and_then(|v| v.as_str()) == Some("index.md"))
        .and_then(|n| n.get("id").and_then(|v| v.as_str()))
        .ok_or("index.md node present")?;
    let index_refs: HashSet<&str> = res
        .edges
        .iter()
        .filter(|e| {
            e.get("relation").and_then(|v| v.as_str()) == Some("references")
                && e.get("source").and_then(|v| v.as_str()) == Some(index_id)
        })
        .filter_map(|e| e.get("target").and_then(|v| v.as_str()))
        .collect();
    assert_eq!(
        index_refs.len(),
        3,
        "hub doc under-connected: {index_refs:?}"
    );
    Ok(())
}
