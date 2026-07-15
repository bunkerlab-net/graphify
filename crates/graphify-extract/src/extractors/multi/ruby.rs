//! Type-aware cross-file resolution for Ruby member calls (#1499).
//!
//! Ruby has no type annotations and reuses method names heavily, so resolving
//! `obj.method()` by globally-unique name is both lossy (drops on collision) and
//! unsafe (can attach to the wrong same-named method). This pass instead uses the
//! receiver's *type*, inferred at extraction time from local `var = ClassName.new`
//! bindings and carried on each member-call `RawCall` as `receiver_type`.
//!
//! It resolves three shapes, all at EXTRACTED (1.0) confidence and only when the
//! target is certain (single owning class, single owned method) — bail otherwise:
//!
//!   * `Processor.new`  -> a `calls` edge to the `Processor` class
//!   * `p.run` where `p` is a `Processor` -> a `calls` edge to `Processor#run`
//!   * `Service.call` / `Model.where` (constant receiver) -> the class's owned
//!     singleton/instance method when present, else the class node itself so
//!     inherited/dynamic class methods still give blast-radius (#1634)
//!
//! Runs after id-disambiguation, so node ids and raw-call caller ids are final.

use std::collections::{HashMap, HashSet};

use super::java::is_type_like_definition;
use crate::types::{Edge, Node, RawCall};

/// Normalise a class/method label to a comparison key (drop punctuation, fold).
/// Mirrors Python `_key`.
fn key(label: &str) -> String {
    label
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase()
}

/// The single class node id owning `name`, or `None` when absent or ambiguous
/// (god-node guard). Mirrors Python `_unique_class`.
fn unique_class<'a>(
    class_def_nids: &'a HashMap<String, Vec<String>>,
    name: &str,
) -> Option<&'a str> {
    match class_def_nids.get(&key(name)) {
        Some(nids) if nids.len() == 1 => nids.first().map(String::as_str),
        _ => None,
    }
}

/// Append a `calls` edge unless it is a self-loop or already present. Mirrors
/// Python `_emit`.
fn push_edge(
    caller: &str,
    target: &str,
    rc: &RawCall,
    existing: &mut HashSet<(String, String)>,
    out: &mut Vec<Edge>,
) {
    if caller.is_empty() || target.is_empty() || caller == target {
        return;
    }
    if !existing.insert((caller.to_string(), target.to_string())) {
        return;
    }
    out.push(Edge {
        external: false,
        source: caller.to_string(),
        target: target.to_string(),
        relation: "calls".to_string(),
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

/// Resolve Ruby `Class.new` and typed `var.method` calls by receiver type.
///
/// Purely additive: only emits edges the shared (name-based) call pass skips
/// because they are member calls. Each emission requires a single owning class
/// (god-node guard) so an ambiguous class name resolves to nothing rather than a
/// wrong edge. Mirrors Python `resolve_ruby_member_calls`.
pub(super) fn resolve_ruby_member_calls(
    all_nodes: &[Node],
    all_edges: &mut Vec<Edge>,
    all_raw_calls: &[RawCall],
) {
    let node_by_id: HashMap<&str, &Node> = all_nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    // Index class-definition nodes independently of method ownership: a Ruby
    // class/module is a `contains` target whose label is a Constant (upper-cased)
    // type-like definition. This lets a method-less class (`class Config; end`) or
    // an empty `module`/`Class.new(...)` still resolve `Config.new`/`Service.call`
    // (#1640/#1634). A Rust-specific equivalent of graphify-py, which builds the
    // class index from `method` edges (extract.py:9621) plus a `.rb` bare-constant
    // pass; the `contains` + type-like filter here is narrower than that broad
    // bare-constant scan. Scoped to `.rb` nodes so a Ruby `Model.where` never binds
    // to a same-named Java/C# `Model` (nor gets falsely poisoned by one).
    let mut class_def_nids: HashMap<String, Vec<String>> = HashMap::new();
    let mut method_index: HashMap<(String, String), String> = HashMap::new();
    let contained: HashSet<&str> = all_edges
        .iter()
        .filter(|e| e.relation == "contains")
        .map(|e| e.target.as_str())
        .collect();
    for n in all_nodes {
        if crate::lang_configs::ends_with_suffix_ci(&n.source_file, &[".rb"])
            && contained.contains(n.id.as_str())
            && is_type_like_definition(n)
            && n.label.chars().next().is_some_and(char::is_uppercase)
        {
            class_def_nids
                .entry(key(&n.label))
                .or_default()
                .push(n.id.clone());
        }
    }
    // (class_node_id, method_key) -> method id, from `method` edges. The edge
    // source also confirms a class node (belt-and-braces with the index above),
    // again `.rb`-scoped.
    for e in all_edges.iter() {
        if e.relation != "method" {
            continue;
        }
        if let Some(cnode) = node_by_id.get(e.source.as_str())
            && crate::lang_configs::ends_with_suffix_ci(&cnode.source_file, &[".rb"])
        {
            class_def_nids
                .entry(key(&cnode.label))
                .or_default()
                .push(e.source.clone());
        }
        if let Some(tnode) = node_by_id.get(e.target.as_str()) {
            method_index.insert((e.source.clone(), key(&tnode.label)), e.target.clone());
        }
    }
    for nids in class_def_nids.values_mut() {
        nids.sort();
        nids.dedup();
    }

    let mut existing_pairs: HashSet<(String, String)> = all_edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let mut new_edges: Vec<Edge> = Vec::new();
    for rc in all_raw_calls {
        // Scope to Ruby raw_calls (mirrors Python `_ruby_raw_calls`); only member
        // calls are resolved here (the shared pass already handled the rest).
        if !crate::lang_configs::ends_with_suffix_ci(&rc.source_file, &[".rb"])
            || !rc.is_member_call
        {
            continue;
        }
        if rc.caller_nid.is_empty() || rc.callee.is_empty() {
            continue;
        }

        // Constant receiver: `Processor.new` (instantiation) or `Service.call` /
        // `Model.where` (singleton / class method, #1634). The bare method name
        // would collide with unrelated same-named methods, so resolve by the
        // receiver's class under the single-owning-class god-node guard.
        if let Some(receiver) = rc.receiver.as_deref()
            && receiver.chars().next().is_some_and(char::is_uppercase)
        {
            if let Some(class_nid) = unique_class(&class_def_nids, receiver) {
                let target = if rc.callee == "new" {
                    class_nid
                } else {
                    // Emit to the singleton/instance method the class owns
                    // (`def self.call`) if present; otherwise to the class node
                    // itself so inherited/dynamic class methods (ActiveRecord
                    // `where`/`find_by`) still give correct blast-radius.
                    method_index
                        .get(&(class_nid.to_string(), key(&rc.callee)))
                        .map_or(class_nid, String::as_str)
                };
                push_edge(
                    &rc.caller_nid,
                    target,
                    rc,
                    &mut existing_pairs,
                    &mut new_edges,
                );
            }
            continue;
        }

        // `p.run` where p's type is known -> edge to that class's method.
        let Some(receiver_type) = rc.receiver_type.as_deref() else {
            continue;
        };
        let Some(class_nid) = unique_class(&class_def_nids, receiver_type) else {
            continue;
        };
        if let Some(method_nid) = method_index.get(&(class_nid.to_string(), key(&rc.callee))) {
            push_edge(
                &rc.caller_nid,
                method_nid,
                rc,
                &mut existing_pairs,
                &mut new_edges,
            );
        }
    }
    all_edges.extend(new_edges);
}
