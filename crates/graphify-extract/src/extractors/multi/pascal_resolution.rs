//! Cross-file resolution for Pascal/Delphi calls to inherited methods (#1739).
//!
//! The per-file Pascal extractor resolves a call to a method on the caller's own
//! class, its ancestor chain, or a file-level free function — but only within the
//! single file being extracted. Real Delphi codebases commonly split a class
//! across two files (a code-generator base class and a manual descendant in a
//! separate unit), so a call from the descendant to a method it inherits from the
//! generated base falls outside any one file's own scope; the per-file pass emits
//! those as `raw_calls`.
//!
//! This resolver runs after all files are extracted (registered in the resolver
//! registry) with the full merged node/edge corpus, so it walks an `inherits`
//! chain across file boundaries. It intentionally does NOT fall back to a global
//! by-name match — an unqualified call resolving to a specific ancestor mirrors
//! Delphi's nearest-ancestor method lookup, a structural resolution rather than a
//! corpus-wide by-name guess.

use std::collections::{HashMap, HashSet};

use crate::types::{Edge, Node, RawCall};

/// Pascal/Delphi source suffixes whose `raw_calls` this resolver handles.
const PASCAL_SUFFIXES: [&str; 5] = [".pas", ".pp", ".dpr", ".dpk", ".inc"];

/// Resolve Pascal/Delphi calls to a method inherited across file boundaries.
///
/// Purely additive: only emits edges for raw calls the per-file pass could not
/// resolve locally. Each emission requires a single owning class at the nearest
/// matching level of the caller's `inherits` chain (god-node guard, same as
/// `resolve_ruby_member_calls`) — an ambiguous or unresolved name produces no
/// edge rather than a guess. Mirrors Python `resolve_pascal_inherited_calls`.
pub(super) fn resolve_pascal_inherited_calls(
    all_nodes: &[Node],
    all_edges: &mut Vec<Edge>,
    all_raw_calls: &[RawCall],
) {
    let node_by_id: HashMap<&str, &Node> = all_nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    // class -> its base classes (from `inherits` edges).
    let mut class_bases: HashMap<&str, Vec<&str>> = HashMap::new();
    // method_nid -> owning class nid, so a raw call's caller_nid (a method or
    // free-function nid) maps to the CLASS whose inherits chain should be walked.
    let mut owner_of: HashMap<&str, &str> = HashMap::new();
    // class nid -> (method_name_lower -> [distinct method_nid]).
    let mut class_procs: HashMap<&str, HashMap<String, Vec<&str>>> = HashMap::new();
    for e in all_edges.iter() {
        match e.relation.as_str() {
            "inherits" => {
                class_bases
                    .entry(e.source.as_str())
                    .or_default()
                    .push(e.target.as_str());
            }
            "method" => {
                let (owner, method_nid) = (e.source.as_str(), e.target.as_str());
                owner_of.insert(method_nid, owner);
                let Some(mnode) = node_by_id.get(method_nid) else {
                    continue;
                };
                let name_lower = mnode
                    .label
                    .strip_suffix("()")
                    .unwrap_or(&mnode.label)
                    .to_lowercase();
                // Count DISTINCT methods, not edge multiplicity: the interface
                // declaration and the implementation both emit a `method` edge to
                // the same node id, so the same method_nid arrives twice. Deduping
                // keeps the single-owner god-node guard measuring real same-name
                // collisions across classes, not one method double-counted —
                // otherwise every inherited call looks ambiguous.
                let bucket = class_procs
                    .entry(owner)
                    .or_default()
                    .entry(name_lower)
                    .or_default();
                if !bucket.contains(&method_nid) {
                    bucket.push(method_nid);
                }
            }
            _ => {}
        }
    }

    let mut existing_pairs: HashSet<(String, String)> = all_edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let mut new_edges: Vec<Edge> = Vec::new();
    for rc in all_raw_calls {
        // Case-insensitive suffix match to mirror get_extractor's dispatch (#1671):
        // an uppercase `Foo.PAS` is still Pascal, so its inherited calls resolve.
        // Divergence from graphify-py's case-sensitive `.endswith(_PASCAL_SUFFIXES)`.
        if !crate::lang_configs::ends_with_suffix_ci(&rc.source_file, &PASCAL_SUFFIXES) {
            continue;
        }
        if rc.caller_nid.is_empty() || rc.callee.is_empty() {
            continue;
        }
        let Some(&owner) = owner_of.get(rc.caller_nid.as_str()) else {
            continue;
        };
        // `rc.callee` is already lowercased at emission (`extractors/pascal` lowercases
        // the call name), so it satisfies `resolve_up_chain`'s `name_lower` contract
        // and matches the lowercased `class_procs` keys — no re-fold needed here.
        let Some(target) = resolve_up_chain(owner, &rc.callee, &class_bases, &class_procs) else {
            continue;
        };
        if target == rc.caller_nid {
            continue;
        }
        let pair = (rc.caller_nid.clone(), target.to_string());
        if !existing_pairs.insert(pair) {
            continue;
        }
        new_edges.push(Edge {
            external: false,
            source: rc.caller_nid.clone(),
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
    all_edges.extend(new_edges);
}

/// Walk `owner`'s `inherits` chain (BFS) for the nearest base declaring
/// `name_lower`. Returns that base's method nid when it declares the name exactly
/// once, `None` when it declares the name more than once (a genuine same-class
/// ambiguity) or the name is nowhere in the chain. Ports graphify-py `_resolve`:
/// the FIRST base reached in BFS (declaration) order wins. Delphi lists the
/// parent class before any implemented interfaces (`class(TParent, IFace, ...)`),
/// so the first base is the parent whose implementation should own the call.
/// Aggregating same-depth bases into a "multiple owners -> bail" check would
/// regress that: an interface re-declaring an inherited name would suppress the
/// real parent-method call. Kept nearest-base-first to match the reference.
fn resolve_up_chain<'a>(
    owner: &'a str,
    name_lower: &str,
    class_bases: &HashMap<&'a str, Vec<&'a str>>,
    class_procs: &HashMap<&'a str, HashMap<String, Vec<&'a str>>>,
) -> Option<&'a str> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut queue: Vec<&str> = class_bases.get(owner).cloned().unwrap_or_default();
    let mut head = 0;
    while head < queue.len() {
        let base = queue[head];
        head += 1;
        if !seen.insert(base) {
            continue;
        }
        if let Some(candidates) = class_procs.get(base).and_then(|m| m.get(name_lower)) {
            return if candidates.len() == 1 {
                Some(candidates[0])
            } else {
                None
            };
        }
        if let Some(bases) = class_bases.get(base) {
            queue.extend(bases.iter().copied());
        }
    }
    None
}
