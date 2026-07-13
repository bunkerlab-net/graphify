//! Go call-graph pass.

use super::read_text;
use crate::types::{Edge, RawCall};
use std::collections::{HashMap, HashSet};

/// Collect `calls` edges within a Go function or method body.
///
/// Recurses through the body AST, emitting `calls` edges for `call_expression` nodes whose
/// callee matches a known function NID in this file. Selector expressions (package.Func) are
/// resolved against `go_imported_pkgs`. Mirrors Python `_walk_calls_go`.
/// Shared state threaded through every [`walk_calls_go`] recursion.
pub(super) struct GoCallCtx<'a> {
    pub(super) str_path: &'a str,
    pub(super) label_to_nid: &'a HashMap<String, String>,
    pub(super) go_imported_pkgs: &'a HashSet<String>,
    pub(super) edges: &'a mut Vec<Edge>,
    pub(super) seen_call_pairs: &'a mut HashSet<(String, String)>,
    pub(super) raw_calls: &'a mut Vec<RawCall>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over Go's call-site AST shapes
pub(super) fn walk_calls_go(
    ctx: &mut GoCallCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    caller_nid: &str,
    body_start: usize,
    body_end: usize,
) {
    let str_path = ctx.str_path;
    let label_to_nid = ctx.label_to_nid;
    let go_imported_pkgs = ctx.go_imported_pkgs;
    let edges = &mut *ctx.edges;
    let seen_call_pairs = &mut *ctx.seen_call_pairs;
    let raw_calls = &mut *ctx.raw_calls;
    // Only visit nodes within the body range
    if node.start_byte() >= body_end || node.end_byte() <= body_start {
        return;
    }

    match node.kind() {
        "function_declaration" | "method_declaration" => {
            // Don't recurse into nested functions
        }
        "call_expression" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let mut callee_name: Option<String> = None;
                let mut is_member_call = false;
                match func_node.kind() {
                    "identifier" => {
                        callee_name = Some(read_text(func_node, source).to_string());
                    }
                    "selector_expression" => {
                        let field = func_node.child_by_field_name("field");
                        let operand = func_node.child_by_field_name("operand");
                        let receiver_name = operand
                            .map(|n| read_text(n, source).to_string())
                            .unwrap_or_default();
                        // Package-qualified call: fmt.Println → not a member call
                        is_member_call = !go_imported_pkgs.contains(&receiver_name);
                        if let Some(f) = field {
                            callee_name = Some(read_text(f, source).to_string());
                        }
                    }
                    _ => {}
                }
                // Built-in suppression applies only to unqualified identifier
                // calls; a selector call (`obj.len()`, `pkg.len()`) names a method
                // or package function, not the language built-in, so it must not
                // be filtered.
                let is_unqualified = func_node.kind() == "identifier";
                if let Some(cn) = callee_name {
                    let tgt_nid = label_to_nid.get(&cn.to_lowercase()).cloned();
                    if let Some(tgt) = tgt_nid {
                        if tgt != caller_nid {
                            let pair = (caller_nid.to_string(), tgt.clone());
                            if seen_call_pairs.insert(pair) {
                                let line = node.start_position().row + 1;
                                edges.push(Edge {
                                    external: false,
                                    source: caller_nid.to_string(),
                                    target: tgt,
                                    relation: "calls".to_string(),
                                    confidence: "EXTRACTED".to_string(),
                                    source_file: str_path.to_string(),
                                    source_location: Some(format!("L{line}")),
                                    weight: 1.0,
                                    context: Some("call".to_string()),
                                    confidence_score: None,
                                    deferred: false,
                                    metadata: None,
                                });
                            }
                        }
                    } else if !(is_unqualified && crate::builtins::is_language_builtin_global(&cn))
                    {
                        raw_calls.push(RawCall {
                            caller_nid: caller_nid.to_string(),
                            callee: cn,
                            is_member_call,
                            source_file: str_path.to_string(),
                            source_location: format!("L{}", node.start_position().row + 1),
                            receiver: None,
                            receiver_type: None,
                            lang: None,
                            ..Default::default()
                        });
                    }
                }
            }
            // Recurse into arguments
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_calls_go(ctx, cur.node(), source, caller_nid, body_start, body_end);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_calls_go(ctx, cur.node(), source, caller_nid, body_start, body_end);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}
