//! A node whose `source_file` equals the scan root must not crash build (#1618).
//!
//! Ports the public-API cases of
//! `graphify-py/tests/test_semantic_id_remap_root.py`. `norm_source_file`
//! relativizes an absolute `source_file` equal to the root to `.`, which then fed
//! `file_stem`'s `with_extension("")` — a project-level node (`source_file` == root)
//! has no per-file identity to remap, so its id is left untouched. The private
//! `_semantic_id_remap`/`_file_stem` cases are exercised through `build_from_json`.
#![allow(clippy::expect_used)]

use std::path::Path;

use graphify_build::build_from_json;
use serde_json::json;

#[test]
fn build_from_json_with_root_level_concept_node() {
    // Previously crashed in the id-remap when a node's source_file equalled the
    // scan root (relativized to `.`). Must assemble both nodes without panicking.
    let combined = json!({
        "nodes": [
            {"id": "proj_concept", "label": "Project", "file_type": "concept",
             "source_file": "/proj", "_origin": "semantic"},
            {"id": "src_foo", "label": "foo", "file_type": "code",
             "source_file": "src/foo.py", "_origin": "ast"},
        ],
        "edges": [],
    });
    let g = build_from_json(combined, false, Some(Path::new("/proj"))).expect("build_from_json");
    assert_eq!(g.node_count(), 2);
}
