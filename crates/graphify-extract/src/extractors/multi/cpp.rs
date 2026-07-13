//! Cross-file C++ member-call resolution (#1547).
//!
//! The shared cross-file pass drops every `is_member_call` because a bare method
//! name (`bar`) collides across the corpus and inflates god-nodes. The C++
//! extractor records each member call's receiver and a per-file `var -> ClassName`
//! table (`cpp_var_types` -> `RawCall::receiver_type`). This pass types the
//! receiver, then emits an edge ONLY when that type resolves to exactly ONE
//! definition (the god-node guard).
//!
//! Receiver typing, by precision tier: `Foo::bar()` — the scope `Foo` names the
//! type explicitly (EXTRACTED); `this->bar()` — the receiver is the caller's own
//! enclosing class (EXTRACTED); `f.bar()` / `f->bar()` — `f` typed via the file's
//! local table (INFERRED). A receiver whose type can't be inferred is SKIPPED (no
//! guess). The `merge_decl_def_classes` pass has already folded each header/impl
//! class pair into one node, so a paired class clears the single-definition guard.
//!
//! Mirrors graphify-py `_resolve_cpp_member_calls`.

use std::collections::{HashMap, HashSet};

use super::java::is_type_like_definition;
use crate::types::{Edge, Node, RawCall, RawCallLang};

/// Normalise a C++ type/method label to a comparison key (drop punctuation,
/// fold). Mirrors the inner `_key` of graphify-py `_resolve_cpp_member_calls`.
fn cpp_key(label: &str) -> String {
    label
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase()
}

/// Resolve deferred C++ member calls (`f.bar()`, `f->bar()`, `Foo::bar()`,
/// `this->bar()`) to the real definition of the receiver's type. Purely additive:
/// only handles member calls the shared pass deferred. Mirrors graphify-py
/// `_resolve_cpp_member_calls`.
pub(super) fn resolve_cpp_member_calls(
    all_nodes: &[Node],
    all_edges: &mut Vec<Edge>,
    all_raw_calls: &[RawCall],
) {
    let node_by_id: HashMap<&str, &Node> = all_nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let contained: HashSet<&str> = all_edges
        .iter()
        .filter(|e| e.relation == "contains")
        .map(|e| e.target.as_str())
        .collect();

    // key(label) -> type-definition node ids (source-backed, contained, type-like).
    let mut type_def_nids: HashMap<String, Vec<String>> = HashMap::new();
    for n in all_nodes {
        if !n.source_file.is_empty()
            && contained.contains(n.id.as_str())
            && is_type_like_definition(n)
        {
            type_def_nids
                .entry(cpp_key(&n.label))
                .or_default()
                .push(n.id.clone());
        }
    }

    // (type_nid, method_key) -> method_nid, and method -> owning type. Index both
    // `defines` (in-class declarations the extractor models as fields) and
    // `method` (out-of-line definitions); `method` wins when a key has both, so a
    // header-declared `void bar();` still resolves.
    let mut method_index: HashMap<(String, String), String> = HashMap::new();
    let mut enclosing_type: HashMap<String, String> = HashMap::new();
    for rel in ["defines", "method"] {
        for e in all_edges.iter() {
            if e.relation != rel {
                continue;
            }
            let Some(tnode) = node_by_id.get(e.target.as_str()) else {
                continue;
            };
            enclosing_type
                .entry(e.target.clone())
                .or_insert_with(|| e.source.clone());
            method_index.insert((e.source.clone(), cpp_key(&tnode.label)), e.target.clone());
        }
    }

    let mut existing_pairs: HashSet<(String, String)> = all_edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let mut new_edges: Vec<Edge> = Vec::new();
    for rc in all_raw_calls {
        if rc.lang != Some(RawCallLang::Cpp) || !rc.is_member_call {
            continue;
        }
        let receiver = match rc.receiver.as_deref() {
            Some(r) if !r.is_empty() => r,
            _ => continue,
        };
        if rc.callee.is_empty() || rc.caller_nid.is_empty() {
            continue;
        }
        let caller = rc.caller_nid.as_str();

        let (type_nid, exact): (String, bool) = if receiver == "this" {
            match enclosing_type.get(caller) {
                Some(t) => (t.clone(), true),
                None => continue,
            }
        } else if receiver.chars().next().is_some_and(char::is_uppercase) {
            // Foo::bar(): the type is named explicitly in source.
            match type_def_nids.get(&cpp_key(receiver)) {
                Some(defs) if defs.len() == 1 => (defs[0].clone(), true),
                _ => continue, // absent or ambiguous -> god-node guard
            }
        } else {
            // f.bar() / f->bar(): type the receiver via the extractor's local table.
            let Some(type_name) = rc.receiver_type.as_deref() else {
                continue;
            };
            match type_def_nids.get(&cpp_key(type_name)) {
                Some(defs) if defs.len() == 1 => (defs[0].clone(), false),
                _ => continue,
            }
        };

        let (target, relation) = match method_index.get(&(type_nid.clone(), cpp_key(&rc.callee))) {
            Some(m) => (m.clone(), "calls"),
            None => (type_nid, "references"),
        };
        if target == caller || !existing_pairs.insert((caller.to_string(), target.clone())) {
            continue;
        }
        new_edges.push(Edge {
            external: false,
            source: caller.to_string(),
            target,
            relation: relation.to_string(),
            confidence: if exact { "EXTRACTED" } else { "INFERRED" }.to_string(),
            source_file: rc.source_file.clone(),
            source_location: Some(rc.source_location.clone()),
            weight: 1.0,
            context: Some("call".to_string()),
            confidence_score: Some(if exact { 1.0 } else { 0.8 }),
            deferred: false,
            metadata: None,
        });
    }
    all_edges.extend(new_edges);
}
