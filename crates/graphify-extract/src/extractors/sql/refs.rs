//! SQL FROM/JOIN data-flow reference edges.

use super::read_text;
use crate::ids::make_id;
use crate::types::Edge;

/// Recursively walk a SQL AST finding `FROM` and `JOIN` clauses and emitting `references` edges.
///
/// Used to add query-time data-flow edges from functions/views to the tables they read.
/// Mirrors Python `_walk_from_refs`.
pub(super) fn walk_from_refs(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    caller_nid: &str,
    edges: &mut Vec<Edge>,
) {
    if matches!(node.kind(), "from" | "join") {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().kind() == "relation" {
                    let mut rc = cur.node().walk();
                    if rc.goto_first_child() {
                        loop {
                            if rc.node().kind() == "object_reference" {
                                let tbl = read_text(rc.node(), source);
                                let tbl_nid = make_id(&[stem, tbl]);
                                let line = rc.node().start_position().row + 1;
                                edges.push(Edge {
                                    external: false,
                                    source: caller_nid.to_string(),
                                    target: tbl_nid,
                                    relation: "reads_from".to_string(),
                                    confidence: "EXTRACTED".to_string(),
                                    source_file: str_path.to_string(),
                                    source_location: Some(format!("L{line}")),
                                    weight: 1.0,
                                    context: None,
                                    confidence_score: None,
                                    deferred: false,
                                    metadata: None,
                                });
                            }
                            if !rc.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_from_refs(cur.node(), source, str_path, stem, caller_nid, edges);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
