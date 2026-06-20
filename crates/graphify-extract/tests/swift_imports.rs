//! Parity tests for Swift module-anchor collapse (#1327), ported from
//! `graphify-py/tests/test_swift_import_resolution.py`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use graphify_build::build_from_json;
use graphify_extract::{ExtractOutput, extract};

fn write_file(root: &Path, rel: &str, text: &str) -> PathBuf {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().expect("parent")).expect("create_dir_all");
    std::fs::write(&p, text).expect("write");
    p
}

fn module_node_ids<'a>(res: &'a ExtractOutput, label: &str) -> HashSet<&'a str> {
    res.nodes
        .iter()
        .filter(|n| {
            n.get("metadata")
                .and_then(|m| m.get("type"))
                .and_then(|v| v.as_str())
                == Some("module")
                && n.get("label").and_then(|v| v.as_str()) == Some(label)
        })
        .filter_map(|n| n.get("id").and_then(|v| v.as_str()))
        .collect()
}

fn import_edges(res: &ExtractOutput) -> Vec<(&str, &str)> {
    res.edges
        .iter()
        .filter(|e| e.get("relation").and_then(|v| v.as_str()) == Some("imports"))
        .filter_map(|e| Some((e.get("source")?.as_str()?, e.get("target")?.as_str()?)))
        .collect()
}

#[test]
fn swift_import_resolves_to_module_node() {
    let tmp = tempfile::tempdir().unwrap();
    let core = write_file(
        tmp.path(),
        "Sources/CoreKit/CoreKit.swift",
        "public struct CoreKit {}\n",
    );
    let feature = write_file(
        tmp.path(),
        "Sources/FeatureKit/FeatureKit.swift",
        "import CoreKit\n\npublic struct FeatureKit {}\n",
    );
    let res = extract(&[core, feature], Some(tmp.path()));

    let node_ids: HashSet<&str> = res
        .nodes
        .iter()
        .filter_map(|n| n.get("id").and_then(|v| v.as_str()))
        .collect();
    let imports = import_edges(&res);
    assert!(!imports.is_empty(), "expected an imports edge");
    for (_, target) in &imports {
        assert!(
            node_ids.contains(target),
            "import target {target} is not a node"
        );
    }
    assert!(
        !module_node_ids(&res, "CoreKit").is_empty(),
        "no CoreKit module node"
    );
}

#[test]
fn swift_same_module_imported_twice_collapses_to_one_node() {
    let tmp = tempfile::tempdir().unwrap();
    let core = write_file(
        tmp.path(),
        "Sources/CoreKit/CoreKit.swift",
        "public struct CoreKit {}\n",
    );
    let a = write_file(
        tmp.path(),
        "Sources/AKit/AKit.swift",
        "import CoreKit\n\npublic struct AKit {}\n",
    );
    let b = write_file(
        tmp.path(),
        "Sources/BKit/BKit.swift",
        "import CoreKit\n\npublic struct BKit {}\n",
    );
    let res = extract(&[core, a, b], Some(tmp.path()));

    let module_ids = module_node_ids(&res, "CoreKit");
    assert_eq!(module_ids.len(), 1, "CoreKit module split into duplicates");
    let targets: HashSet<&str> = import_edges(&res).into_iter().map(|(_, t)| t).collect();
    assert_eq!(
        targets, module_ids,
        "imports must point at the one module id"
    );
}

#[test]
fn swift_import_edges_survive_build() {
    let tmp = tempfile::tempdir().unwrap();
    let core = write_file(
        tmp.path(),
        "Sources/CoreKit/CoreKit.swift",
        "public struct CoreKit {}\n",
    );
    let a = write_file(tmp.path(), "Sources/AKit/AKit.swift", "import CoreKit\n");
    let b = write_file(tmp.path(), "Sources/BKit/BKit.swift", "import CoreKit\n");
    let res = extract(&[core, a, b], Some(tmp.path()));

    let imports = import_edges(&res);
    assert_eq!(imports.len(), 2, "{imports:?}");
    let targets: HashSet<&str> = imports.iter().map(|(_, t)| *t).collect();
    assert_eq!(
        targets.len(),
        1,
        "both imports must land on one module node"
    );

    let g = build_from_json(serde_json::to_value(&res).unwrap(), true, None).expect("build");
    for (s, t) in &imports {
        assert!(g.edge_data(s, t).is_some(), "import edge {s}->{t} pruned");
    }
}
