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
        // A nested method has its own scope — don't descend into it.
        if matches!(child.kind(), "method" | "singleton_method") {
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
