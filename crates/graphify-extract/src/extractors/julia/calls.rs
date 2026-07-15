//! Julia call-graph pass.

use super::read_text;
use crate::ids::make_id;
use crate::types::Edge;
use std::collections::HashSet;

/// Collect `calls` edges within a Julia function body's byte range.
///
/// Skips nested `function_definition` nodes. Emits `calls` edges for `call_expression` nodes
/// whose callee matches a known NID. Mirrors Python `_walk_calls_julia`.
/// Shared state threaded through every [`walk_calls_julia`] recursion.
pub(super) struct JuliaCallCtx<'a> {
    pub(super) str_path: &'a str,
    pub(super) stem: &'a str,
    pub(super) edges: &'a mut Vec<Edge>,
    pub(super) seen_ids: &'a HashSet<String>,
}

pub(super) fn walk_calls_julia(
    ctx: &mut JuliaCallCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    func_nid: &str,
    body_start: usize,
    body_end: usize,
) {
    if node.start_byte() >= body_end || node.end_byte() <= body_start {
        return;
    }
    if matches!(
        node.kind(),
        "function_definition" | "short_function_definition"
    ) {
        return;
    }
    if node.kind() == "call_expression" && node.child_count() > 0 {
        let callee = {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                Some(cur.node())
            } else {
                None
            }
        };
        if let Some(callee_node) = callee {
            if callee_node.kind() == "identifier" {
                let callee_name = read_text(callee_node, source);
                let target_nid = make_id(&[ctx.stem, callee_name]);
                if ctx.seen_ids.contains(&target_nid) && target_nid != func_nid {
                    ctx.edges.push(Edge {
                        external: false,
                        source: func_nid.to_string(),
                        target: target_nid,
                        relation: "calls".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{}", node.start_position().row + 1)),
                        weight: 1.0,
                        context: Some("call".to_string()),
                        confidence_score: None,
                        deferred: false,
                        metadata: None,
                    });
                }
            } else if callee_node.kind() == "field_expression" && callee_node.child_count() >= 3 {
                let count = u32::try_from(callee_node.child_count()).unwrap_or(0);
                let method_node = callee_node.child(count - 1);
                if let Some(mn) = method_node {
                    let method_name = read_text(mn, source);
                    let target_nid = make_id(&[ctx.stem, method_name]);
                    if ctx.seen_ids.contains(&target_nid) && target_nid != func_nid {
                        ctx.edges.push(Edge {
                            external: false,
                            source: func_nid.to_string(),
                            target: target_nid,
                            relation: "calls".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{}", node.start_position().row + 1)),
                            weight: 1.0,
                            context: Some("call".to_string()),
                            confidence_score: None,
                            deferred: false,
                            metadata: None,
                        });
                    }
                }
            }
        }
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_calls_julia(ctx, cur.node(), source, func_nid, body_start, body_end);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Walk the body children of a `function_definition` node, calling `walk_calls_julia` on each.
///
/// Finds the `function_definition` node by byte range, then iterates its children starting
/// after the signature, so nested function bodies are attributed to the right caller.
// Walk children of a function_definition node (skipping signature)
pub(super) fn walk_calls_julia_children(
    ctx: &mut JuliaCallCtx<'_>,
    tree_root: tree_sitter::Node<'_>,
    source: &[u8],
    func_nid: &str,
    node_start: usize,
    node_end: usize,
) {
    // Find the function_definition node by byte range
    /// Search the subtree rooted at `n` for a `function_definition` node matching `start`/`end` byte offsets.
    fn find_node(
        n: tree_sitter::Node<'_>,
        start: usize,
        end: usize,
    ) -> Option<tree_sitter::Node<'_>> {
        if n.start_byte() == start && n.end_byte() == end && n.kind() == "function_definition" {
            return Some(n);
        }
        let mut cur = n.walk();
        if cur.goto_first_child() {
            loop {
                if let Some(found) = find_node(cur.node(), start, end) {
                    return Some(found);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        None
    }

    let Some(func_node) = find_node(tree_root, node_start, node_end) else {
        return;
    };
    let mut cur = func_node.walk();
    if cur.goto_first_child() {
        loop {
            let child = cur.node();
            if child.kind() != "signature" {
                walk_calls_julia(ctx, child, source, func_nid, node_start, node_end);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
