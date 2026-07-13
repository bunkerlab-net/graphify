//! Rust call-graph pass.

use super::{RUST_TRAIT_METHOD_BLOCKLIST, read_text};
use crate::types::{Edge, RawCall};
use std::collections::{HashMap, HashSet};

/// Collect `calls` ctx.edges within a Rust function body's byte range.
///
/// Recurses through the body AST, emitting `calls` ctx.edges for `call_expression` and
/// `macro_invocation` ctx.nodes whose callee matches a known NID. Mirrors Python `_walk_calls_rust`.
/// Shared state threaded through every [`walk_calls_rust`] recursion.
pub(super) struct RustCallCtx<'a> {
    pub(super) str_path: &'a str,
    pub(super) label_to_nid: &'a HashMap<String, String>,
    pub(super) edges: &'a mut Vec<Edge>,
    pub(super) seen_call_pairs: &'a mut HashSet<(String, String)>,
    pub(super) raw_calls: &'a mut Vec<RawCall>,
}

pub(super) fn walk_calls_rust(
    ctx: &mut RustCallCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    caller_nid: &str,
    body_start: usize,
    body_end: usize,
) {
    if node.start_byte() >= body_end || node.end_byte() <= body_start {
        return;
    }
    if node.kind() == "function_item" {
        return;
    }

    if node.kind() == "call_expression"
        && let Some(func_node) = node.child_by_field_name("function")
    {
        let mut callee_name: Option<String> = None;
        let mut is_member_call = false;
        let mut is_scoped_call = false;
        match func_node.kind() {
            "identifier" => {
                callee_name = Some(read_text(func_node, source).to_string());
            }
            "field_expression" => {
                is_member_call = true;
                if let Some(field) = func_node.child_by_field_name("field") {
                    callee_name = Some(read_text(field, source).to_string());
                }
            }
            "scoped_identifier" => {
                is_scoped_call = true;
                if let Some(name) = func_node.child_by_field_name("name") {
                    callee_name = Some(read_text(name, source).to_string());
                }
            }
            _ => {}
        }
        if let Some(cn) = callee_name {
            // Resolve first so a built-in name backing a real local symbol is
            // kept; only drop unresolved built-ins (god-node guard, #726).
            let tgt_nid = ctx.label_to_nid.get(&cn.to_lowercase()).cloned();
            if let Some(tgt) = tgt_nid {
                if tgt != caller_nid {
                    let pair = (caller_nid.to_string(), tgt.clone());
                    if ctx.seen_call_pairs.insert(pair) {
                        let line = node.start_position().row + 1;
                        ctx.edges.push(Edge {
                            external: false,
                            source: caller_nid.to_string(),
                            target: tgt,
                            relation: "calls".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: Some("call".to_string()),
                            confidence_score: None,
                            deferred: false,
                            metadata: None,
                        });
                    }
                }
            } else if !is_scoped_call
                && !RUST_TRAIT_METHOD_BLOCKLIST.contains(cn.to_lowercase().as_str())
                && !crate::builtins::is_language_builtin_global(&cn)
            {
                ctx.raw_calls.push(RawCall {
                    caller_nid: caller_nid.to_string(),
                    callee: cn,
                    is_member_call,
                    source_file: ctx.str_path.to_string(),
                    source_location: format!("L{}", node.start_position().row + 1),
                    receiver: None,
                    receiver_type: None,
                    lang: None,
                    ..Default::default()
                });
            }
        }
    }

    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_calls_rust(ctx, cur.node(), source, caller_nid, body_start, body_end);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
