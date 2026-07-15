//! Ruby local type inference for member-call resolution (#1499).
//!
//! Ruby has no type annotations, so a member call `obj.method()` can only be
//! resolved by name unless we know `obj`'s type. We infer it from local
//! `var = ClassName.new` bindings within a single method body and carry it on
//! each member-call `RawCall` as `receiver_type`, letting the cross-file pass
//! resolve `var.method` by type rather than by globally-unique method name.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use tree_sitter::Node;

use super::names::read_text_owned;

/// Return `ClassName` if `node` is a `ClassName.new(...)` call, else `None`.
///
/// Only a bare capitalized constant receiver counts (`Processor.new`);
/// namespaced (`A::B.new`) and dynamic receivers are intentionally ignored so
/// the binding stays unambiguous. Mirrors Python `_ruby_new_class_name`.
fn ruby_new_class_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    if node.kind() != "call" {
        return None;
    }
    let recv = node.child_by_field_name("receiver")?;
    let meth = node.child_by_field_name("method")?;
    if recv.kind() != "constant" || read_text_owned(meth, source) != "new" {
        return None;
    }
    Some(read_text_owned(recv, source))
}

/// Map `local_var -> ClassName` for `var = ClassName.new` within one Ruby method
/// body, not descending into nested method definitions.
///
/// 100%-confidence contract: a variable assigned more than once, or to anything
/// other than a single `Constant.new`, maps to `None` (ambiguous) so callers
/// never resolve it. Only the certain single-binding case carries a type.
/// Mirrors Python `_ruby_local_class_bindings`.
#[must_use]
pub(super) fn ruby_local_class_bindings(
    body_node: Node<'_>,
    source: &[u8],
) -> HashMap<String, Option<String>> {
    let mut bindings: HashMap<String, Option<String>> = HashMap::new();
    visit(body_node, source, &mut bindings);
    bindings
}

fn visit(node: Node<'_>, source: &[u8], bindings: &mut HashMap<String, Option<String>>) {
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        // A nested scope has its own bindings — don't let its assignments leak
        // into this method. graphify-py only skips `method`/`singleton_method`
        // (extract.py:2513): a `class`/`module` body is a syntax error inside a
        // method, so MRI never produces one here, but tree-sitter's error
        // recovery can on partial/invalid source (e.g. mid-edit under `watch`),
        // so we guard them too.
        if matches!(
            child.kind(),
            "method" | "singleton_method" | "class" | "module"
        ) {
            if !cur.goto_next_sibling() {
                break;
            }
            continue;
        }
        if child.kind() == "assignment"
            && let Some(left) = child.child_by_field_name("left")
            && left.kind() == "identifier"
        {
            let var = read_text_owned(left, source);
            let cls = child
                .child_by_field_name("right")
                .and_then(|right| ruby_new_class_name(right, source));
            match cls {
                // Assigned to something we can't type: poison only if it was
                // already typed (matches Python — an untyped var stays absent).
                None => {
                    if let Entry::Occupied(mut e) = bindings.entry(var) {
                        e.insert(None);
                    }
                }
                Some(c) => match bindings.entry(var) {
                    Entry::Occupied(mut e) => {
                        // Reassigned to a different class → ambiguous (poison);
                        // an identical re-binding keeps the type.
                        if e.get().as_deref() != Some(c.as_str()) {
                            e.insert(None);
                        }
                    }
                    Entry::Vacant(e) => {
                        e.insert(Some(c));
                    }
                },
            }
        }
        visit(child, source, bindings);
        if !cur.goto_next_sibling() {
            break;
        }
    }
}
/// Last constant of a `constant` or `scope_resolution` (`A::B::C` -> `C`).
/// Mirrors Python `_ruby_const_last_name`.
pub(super) fn ruby_const_last_name(node: Node<'_>, source: &[u8]) -> String {
    match node.kind() {
        "constant" => read_text_owned(node, source),
        "scope_resolution" => {
            let mut cur = node.walk();
            let mut last = String::new();
            if cur.goto_first_child() {
                loop {
                    let c = cur.node();
                    if c.kind() == "constant" {
                        last = read_text_owned(c, source);
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            last
        }
        _ => String::new(),
    }
}

/// A constant assignment whose RHS is `Struct.new(...)`, `Class.new(Super)` or
/// `Data.define(...)` defines a class named after the constant (#1640).
/// tree-sitter parses each as an `assignment` (not a `class`), so the generic
/// class branch never sees them. Synthesise the class node, attach block-defined
/// methods via `method` (recursing the block with the new node as parent), and
/// emit an `inherits` edge for `Class.new(Super)`. Returns `true` when handled.
/// Mirrors Python `_ruby_extra_walk`.
pub(super) fn ruby_extra_walk<'tree>(
    ctx: &mut super::walk::WalkCtx<'_, 'tree>,
    node: Node<'tree>,
    source: &[u8],
) -> bool {
    use super::graph::{add_edge, add_node};
    use crate::ids::make_id;

    if node.kind() != "assignment" {
        return false;
    }
    let (Some(left), Some(right)) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
    ) else {
        return false;
    };
    if left.kind() != "constant" || right.kind() != "call" {
        return false;
    }
    let (Some(recv), Some(meth)) = (
        right.child_by_field_name("receiver"),
        right.child_by_field_name("method"),
    ) else {
        return false;
    };
    if recv.kind() != "constant" {
        return false;
    }
    let recv_name = read_text_owned(recv, source);
    let meth_name = read_text_owned(meth, source);
    if !matches!(
        (recv_name.as_str(), meth_name.as_str()),
        ("Struct" | "Class", "new") | ("Data", "define")
    ) {
        return false;
    }
    let const_name = read_text_owned(left, source);
    if const_name.is_empty() {
        return false;
    }
    let line = u32::try_from(node.start_position().row)
        .unwrap_or(u32::MAX)
        .saturating_add(1);
    let class_nid = make_id(&[ctx.stem, &const_name]);
    add_node(
        &class_nid,
        &const_name,
        line,
        ctx.str_path,
        ctx.nodes,
        ctx.seen_ids,
    );
    // A class is callable via its constructor — register it like every other
    // class/function def site so indirect dispatch can resolve it (parity with
    // graphify-py's `_ruby_extra_walk`, which adds the synthesized class here).
    ctx.callable_def_nids.insert(class_nid.clone());
    // Mirror the generic class branch: containment always hangs off the file node.
    add_edge(
        ctx.file_nid,
        &class_nid,
        "contains",
        line,
        ctx.str_path,
        None,
        ctx.edges,
    );

    // `Class.new(Super)` — the first positional constant argument is the superclass.
    if recv_name == "Class" {
        ruby_emit_class_new_super(ctx, right, &class_nid, line, source);
    }

    // Recurse the do/brace block so block-defined methods attach to the class.
    ruby_recurse_block_methods(ctx, right, &class_nid, source);
    true
}

/// Emit an `inherits` edge from `class_nid` to the first positional constant
/// argument of a `Class.new(Super)` call. Mirrors the `Class.new` arm of
/// Python `_ruby_extra_walk`.
fn ruby_emit_class_new_super<'tree>(
    ctx: &mut super::walk::WalkCtx<'_, 'tree>,
    call: Node<'tree>,
    class_nid: &str,
    line: u32,
    source: &[u8],
) {
    use super::graph::add_edge;
    use super::inherit::emit_base_node;

    let mut rc = call.walk();
    let Some(args) = call.children(&mut rc).find(|c| c.kind() == "argument_list") else {
        return;
    };
    let mut acur = args.walk();
    if !acur.goto_first_child() {
        return;
    }
    loop {
        let arg = acur.node();
        if matches!(arg.kind(), "constant" | "scope_resolution") {
            let base = ruby_const_last_name(arg, source);
            if !base.is_empty() {
                let base_nid =
                    emit_base_node(&base, line, ctx.stem, ctx.str_path, ctx.nodes, ctx.seen_ids);
                add_edge(
                    class_nid,
                    &base_nid,
                    "inherits",
                    line,
                    ctx.str_path,
                    None,
                    ctx.edges,
                );
            }
            break;
        }
        if !acur.goto_next_sibling() {
            break;
        }
    }
}

/// Recurse a `Struct.new`/`Class.new`/`Data.define` do/brace block so its
/// block-defined methods attach to `class_nid` (the method handler sees it as
/// parent). The block wraps its statements in a `body_statement` like a class body.
fn ruby_recurse_block_methods<'tree>(
    ctx: &mut super::walk::WalkCtx<'_, 'tree>,
    call: Node<'tree>,
    class_nid: &str,
    source: &[u8],
) {
    let mut bc = call.walk();
    let Some(block) = call
        .children(&mut bc)
        .find(|c| matches!(c.kind(), "do_block" | "block"))
    else {
        return;
    };
    let mut inner = block.walk();
    let body = block
        .children(&mut inner)
        .find(|c| c.kind() == "body_statement")
        .unwrap_or(block);
    let mut bcur = body.walk();
    if bcur.goto_first_child() {
        loop {
            super::walk::walk(ctx, bcur.node(), Some(class_nid), source);
            if !bcur.goto_next_sibling() {
                break;
            }
        }
    }
}
