//! Cross-file Objective-C message-send resolution (#1556).
//!
//! The Objective-C extractor keeps its same-file selector matching and additionally
//! emits a `RawCall` for every message send, with the receiver and reconstructed
//! selector as the callee (`RawCallLang::ObjC`). This pass types the receiver and
//! emits a cross-file `calls` edge ONLY when the type resolves to exactly ONE
//! definition (the god-node guard).
//!
//! Receiver typing: `self` / `super` — the caller's own enclosing class
//! (EXTRACTED); a capitalized receiver (`[Foo new]`) — the type named explicitly
//! (EXTRACTED); `[f doThing]` — `f` typed via the file's `Foo *f` local table
//! (INFERRED). An uninferable receiver is SKIPPED (no guess). `merge_decl_def_classes`
//! folds each @interface/@implementation pair into one node, so a paired class
//! clears the single-definition guard.
//!
//! Mirrors graphify-py `_resolve_objc_member_calls`.

use std::collections::{HashMap, HashSet};

use super::java::is_type_like_definition;
use crate::types::{Edge, Node, RawCall, RawCallLang};

/// Normalise an Objective-C label to a comparison key (drop punctuation incl. the
/// `+`/`-` method sigil, fold). Mirrors the inner `_key`.
fn objc_key(label: &str) -> String {
    label
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase()
}

/// Resolve deferred Objective-C message sends (`[recv sel]`) to the real definition of
/// the receiver's type. Purely additive. Mirrors graphify-py
/// `_resolve_objc_member_calls`.
pub(super) fn resolve_objc_member_calls(
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

    let mut type_def_nids: HashMap<String, Vec<String>> = HashMap::new();
    for n in all_nodes {
        if !n.source_file.is_empty()
            && contained.contains(n.id.as_str())
            && is_type_like_definition(n)
        {
            type_def_nids
                .entry(objc_key(&n.label))
                .or_default()
                .push(n.id.clone());
        }
    }

    // (type_nid, selector_key) -> method_nid, from `method` ownership edges. ObjC
    // method labels carry a `+`/`-` sigil (`-doThing`); `objc_key` strips it so
    // the selector `doThing` keys to the method.
    let mut method_index: HashMap<(String, String), String> = HashMap::new();
    let mut enclosing_type: HashMap<String, String> = HashMap::new();
    for e in all_edges.iter() {
        if e.relation != "method" {
            continue;
        }
        let Some(tnode) = node_by_id.get(e.target.as_str()) else {
            continue;
        };
        enclosing_type
            .entry(e.target.clone())
            .or_insert_with(|| e.source.clone());
        method_index.insert((e.source.clone(), objc_key(&tnode.label)), e.target.clone());
    }

    let mut existing_pairs: HashSet<(String, String)> = all_edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let mut new_edges: Vec<Edge> = Vec::new();
    for rc in all_raw_calls {
        if rc.lang != Some(RawCallLang::ObjC) || !rc.is_member_call {
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

        let (type_nid, exact): (String, bool) = if receiver == "self" || receiver == "super" {
            match enclosing_type.get(caller) {
                Some(t) => (t.clone(), true),
                None => continue,
            }
        } else if receiver.chars().next().is_some_and(char::is_uppercase) {
            match type_def_nids.get(&objc_key(receiver)) {
                Some(defs) if defs.len() == 1 => (defs[0].clone(), true),
                _ => continue,
            }
        } else {
            let Some(type_name) = rc.receiver_type.as_deref() else {
                continue;
            };
            match type_def_nids.get(&objc_key(type_name)) {
                Some(defs) if defs.len() == 1 => (defs[0].clone(), false),
                _ => continue,
            }
        };

        let (target, relation) = match method_index.get(&(type_nid.clone(), objc_key(&rc.callee))) {
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
