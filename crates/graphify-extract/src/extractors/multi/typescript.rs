//! Cross-file TS/JS member-call resolution (#1316/#1630/#1726).
//!
//! `this.repo.findById()` / `svc.doThing()` drop out of the shared cross-file
//! pass because a bare method name collides across the corpus (god-node guard).
//! The extractor records each member call's receiver and a per-file
//! `name -> TypeName` table (`ts_var_types` -> `RawCall::receiver_type`) from
//! constructor-injected `this.field` types, local `new` bindings, and typed
//! parameters. This pass types the receiver, then emits an edge ONLY when that
//! type resolves to exactly ONE definition (the god-node guard).
//!
//! An uppercase receiver (`ClassName.method()`) is itself the type name; a
//! builtin global type (`Date`, `Promise`, `Map`, …) is skipped so it never binds
//! to a same-named user class (#1726). Mirrors graphify-py
//! `_resolve_typescript_member_calls`.
//!
//! Divergence: gated on the JS/TS source-file suffix (Rust convention — every
//! other member-call resolver gates likewise) so it never claims another
//! language's `raw_call`; graphify-py leaves the pass unfiltered and relies on the
//! type-def index + dedup to avoid cross-language edges.

use std::collections::{HashMap, HashSet};

use super::java::is_type_like_definition;
use crate::types::{Edge, Node, RawCall};

/// JS/TS source-file extensions whose member-call `raw_calls` this pass claims —
/// exactly graphify-py's `typescript_member_calls` registration set.
const TS_JS_SUFFIXES: [&str; 4] = [".ts", ".tsx", ".js", ".jsx"];

/// Normalise a label to a comparison key (drop punctuation, fold). Mirrors the
/// inner `_key`.
fn ts_key(label: &str) -> String {
    label
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase()
}

/// Resolve deferred TS/JS member calls to the real definition of the receiver's
/// type. Purely additive. Mirrors graphify-py `_resolve_typescript_member_calls`.
pub(super) fn resolve_typescript_member_calls(
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
                .entry(ts_key(&n.label))
                .or_default()
                .push(n.id.clone());
        }
    }

    let mut method_index: HashMap<(String, String), String> = HashMap::new();
    for e in all_edges.iter() {
        if e.relation != "method" {
            continue;
        }
        if let Some(tnode) = node_by_id.get(e.target.as_str()) {
            method_index.insert((e.source.clone(), ts_key(&tnode.label)), e.target.clone());
        }
    }

    let mut existing_pairs: HashSet<(String, String)> = all_edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let mut new_edges: Vec<Edge> = Vec::new();
    for rc in all_raw_calls {
        if !rc.is_member_call
            || !crate::lang_configs::ends_with_suffix_ci(&rc.source_file, &TS_JS_SUFFIXES)
        {
            continue;
        }
        let receiver = match rc.receiver.as_deref() {
            Some(r) if !r.is_empty() => r,
            _ => continue,
        };
        if rc.callee.is_empty() || rc.caller_nid.is_empty() {
            continue;
        }
        // Uppercase receiver names the type explicitly; else use the extractor's
        // inferred type. No type -> skip (no guess).
        let type_name = if receiver.chars().next().is_some_and(char::is_uppercase) {
            receiver.to_string()
        } else {
            match rc.receiver_type.as_deref() {
                Some(t) => t.to_string(),
                None => continue,
            }
        };
        // A builtin global type (Date, Promise, Map, …) must not bind to a
        // same-named user class (#1726).
        if crate::builtins::is_language_builtin_global(&type_name) {
            continue;
        }
        let type_nid = match type_def_nids.get(&ts_key(&type_name)) {
            Some(defs) if defs.len() == 1 => defs[0].clone(),
            _ => continue, // absent or ambiguous -> god-node guard
        };
        let caller = rc.caller_nid.as_str();
        let (target, relation) = match method_index.get(&(type_nid.clone(), ts_key(&rc.callee))) {
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
            confidence: "EXTRACTED".to_string(),
            source_file: rc.source_file.clone(),
            source_location: Some(rc.source_location.clone()),
            weight: 1.0,
            context: Some("call".to_string()),
            confidence_score: Some(1.0),
            deferred: false,
            metadata: None,
        });
    }
    all_edges.extend(new_edges);
}
