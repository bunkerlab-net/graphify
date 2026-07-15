//! Kotlin inheritance-edge emitter.

use super::emit_base_node;
use crate::generic::walk::{add_edge, first_child_kind, named_children};
use tree_sitter::Node;

/// Emit `inherits` (`: Base()`) / `implements` (`: Interface`) edges for a
/// Kotlin class's `delegation_specifiers`, plus `references[generic_arg]` for
/// type arguments on the base. Mirrors Python `_extract_kotlin`.
pub(crate) fn emit_kotlin_inheritance(
    ctx: &mut crate::generic::walk::WalkCtx<'_, '_>,
    node: Node<'_>,
    source: &[u8],
    class_nid: &str,
    line: u32,
) {
    use crate::generic::references::{RefRole, kotlin_collect_type_refs, kotlin_user_type_name};
    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;

    for child in named_children(node) {
        if child.kind() != "delegation_specifiers" {
            continue;
        }
        for spec in named_children(child) {
            if spec.kind() != "delegation_specifier" {
                continue;
            }
            let mut relation = "implements";
            let mut user_type_node: Option<Node<'_>> = None;
            for sub in named_children(spec) {
                if sub.kind() == "constructor_invocation" {
                    relation = "inherits";
                    user_type_node = first_child_kind(sub, "user_type");
                    break;
                }
                if sub.kind() == "user_type" {
                    user_type_node = Some(sub);
                    break;
                }
                if sub.kind() == "explicit_delegation" {
                    // `class Foo : Bar by baz` wraps the delegated interface `Bar`
                    // in an `explicit_delegation` node; grab its first `user_type`
                    // so the implements edge (+ generic-arg recovery) still fire.
                    user_type_node = first_child_kind(sub, "user_type");
                    break;
                }
            }
            let Some(ut) = user_type_node else { continue };
            // Skip empty base names (consistent with the PHP emitter) so a
            // malformed `user_type` never spawns an empty-label node.
            let Some(base) = kotlin_user_type_name(ut, source).filter(|b| !b.is_empty()) else {
                continue;
            };
            let base_nid = emit_base_node(&base, line, stem, str_path, nodes, seen_ids);
            add_edge(class_nid, &base_nid, relation, line, str_path, None, edges);
            for arg_child in named_children(ut) {
                if arg_child.kind() != "type_arguments" {
                    continue;
                }
                for arg in named_children(arg_child) {
                    let mut refs: Vec<(String, RefRole)> = Vec::new();
                    if arg.kind() == "type_projection" {
                        for inner in named_children(arg) {
                            kotlin_collect_type_refs(inner, source, true, &mut refs);
                        }
                    } else {
                        kotlin_collect_type_refs(arg, source, true, &mut refs);
                    }
                    for (ref_name, _role) in refs {
                        let target = crate::generic::walk::ensure_named_node(
                            &ref_name, stem, str_path, nodes, seen_ids,
                        );
                        add_edge(
                            class_nid,
                            &target,
                            "references",
                            line,
                            str_path,
                            Some("generic_arg"),
                            edges,
                        );
                    }
                }
            }
        }
    }
}
