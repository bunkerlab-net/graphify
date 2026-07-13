//! C# cross-file type-reference and import resolution.
//!
//! Mirrors Python `graphify/extractors/csharp.py`'s `_resolve_csharp_type_references`
//! (#1562) and `_resolve_cross_file_csharp_imports` (#1552) — the C# counterparts
//! to the Java resolvers. Both are **metadata-driven**: they read the namespace,
//! `using`, alias, and ref-token information the walk already stamped onto nodes
//! and edges, rather than re-parsing the `.cs` files.
//!
//! [`resolve_csharp_type_references`] re-points dangling
//! `inherits`/`implements`/`references` edges that bare-name resolution left on
//! sourceless shadow stubs to the real definition, disambiguating same-named types
//! in different namespaces via each referencing node's enclosing namespace, its
//! in-scope `using N;` imports, and any `using X = N.T;` aliases. Lexical scope is
//! honoured through each edge's `scope_kind`/`scope_id` metadata: a `using` nested
//! in a namespace block binds only within that block. Qualified references
//! (`Q.T`) resolve `Q` as an alias first, then as an exact namespace. Ambiguous
//! matches are refused rather than guessed (the god-node guardrail); unresolved
//! references keep or gain a deterministic dangling stub.
//!
//! [`resolve_cross_file_csharp_imports`] re-points resolvable `using` import edges
//! to their canonical namespace node (`using N;`) or type node
//! (`using X = N.T;` where `N` is a known namespace).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ids::make_id;
use crate::types::{Edge, Node, RawCall};
use serde_json::Value;

/// C# edge relations re-pointed from sourceless shadow stubs to real defs.
const CSHARP_REPOINT_RELATIONS: &[&str] = &["implements", "inherits", "references"];

/// Read a string field from a node's metadata.
fn node_meta_str<'a>(node: &'a Node, key: &str) -> Option<&'a str> {
    node.metadata.as_ref()?.get(key)?.as_str()
}

/// `true` when a node's metadata flags `key` truthy (e.g. `is_nested_type`).
fn node_meta_bool(node: &Node, key: &str) -> bool {
    node.metadata
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Read a string field from an edge's metadata.
fn edge_meta_str<'a>(edge: &'a Edge, key: &str) -> Option<&'a str> {
    edge.metadata.as_ref()?.get(key)?.as_str()
}

/// `true` when an edge's metadata flags `key` truthy (e.g. `qualified`).
fn edge_meta_bool(edge: &Edge, key: &str) -> bool {
    edge.metadata
        .as_ref()
        .and_then(|m| m.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Deterministic node-choice key `(source_file, source_location, id)`.
fn node_sort_key(node: &Node) -> (&str, &str, &str) {
    (
        node.source_file.as_str(),
        node.source_location.as_deref().unwrap_or(""),
        node.id.as_str(),
    )
}

/// Namespace-keyed C# type-definition index `(namespace, name) -> node id`,
/// choosing the deterministically-first node per key. Skips namespace nodes,
/// nested types, non-`.cs`/non-code nodes, and method/dotted labels. Mirrors
/// graphify-py `_build_csharp_type_def_index`.
fn build_csharp_type_def_index(all_nodes: &[Node]) -> HashMap<(String, String), String> {
    let mut candidates: HashMap<(String, String), Vec<&Node>> = HashMap::new();
    for n in all_nodes {
        if n.node_type.as_deref() == Some("namespace") || node_meta_bool(n, "is_nested_type") {
            continue;
        }
        if n.id.is_empty()
            || n.label.is_empty()
            || !n.source_file.ends_with(".cs")
            || n.file_type != "code"
            || n.label.ends_with(')')
            || n.label.starts_with('.')
            || n.label.contains('.')
        {
            continue;
        }
        let ns = node_meta_str(n, "namespace").unwrap_or("").to_string();
        candidates.entry((ns, n.label.clone())).or_default().push(n);
    }
    candidates
        .into_iter()
        .map(|(key, mut nodes)| {
            nodes.sort_by(|a, b| node_sort_key(a).cmp(&node_sort_key(b)));
            (key, nodes[0].id.clone())
        })
        .collect()
}

/// Strip a balanced trailing `<...>` from a C# FQN. Mirrors
/// `_strip_trailing_csharp_generic_args`.
fn strip_trailing_csharp_generic_args(fqn: &str) -> String {
    let fqn = fqn.trim();
    if !fqn.ends_with('>') {
        return fqn.to_string();
    }
    let chars: Vec<char> = fqn.chars().collect();
    let mut depth = 0i32;
    for i in (0..chars.len()).rev() {
        match chars[i] {
            '>' => depth += 1,
            '<' => {
                depth -= 1;
                if depth == 0 {
                    return chars[..i].iter().collect::<String>().trim().to_string();
                }
            }
            _ => {}
        }
    }
    fqn.to_string()
}

/// Reverse the metadata HTML-escaping applied on store (`&`, `<`, `>`, `"`, `'`).
/// Mirrors Python `html.unescape` for that subset.
fn html_unescape_min(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

/// Namespace label -> canonical node id (deterministically-first per label).
fn csharp_namespace_id_by_label(all_nodes: &[Node]) -> HashMap<String, String> {
    let mut ns_nodes: Vec<&Node> = all_nodes
        .iter()
        .filter(|n| n.node_type.as_deref() == Some("namespace"))
        .collect();
    ns_nodes.sort_by(|a, b| node_sort_key(a).cmp(&node_sort_key(b)));
    let mut out: HashMap<String, String> = HashMap::new();
    for n in ns_nodes {
        if !n.label.is_empty() && !n.id.is_empty() {
            out.entry(n.label.clone()).or_insert_with(|| n.id.clone());
        }
    }
    out
}

/// Re-point resolvable C# `using` import edges to canonical namespace / type
/// nodes: a namespace import to the namespace node; an alias import when its
/// target's prefix is a known namespace and the simple type name exists. Mirrors
/// graphify-py `_resolve_cross_file_csharp_imports`.
pub(super) fn resolve_cross_file_csharp_imports(all_nodes: &mut Vec<Node>, all_edges: &mut [Edge]) {
    let ns_by_label = csharp_namespace_id_by_label(all_nodes);
    let type_def_index = build_csharp_type_def_index(all_nodes);
    if ns_by_label.is_empty() && type_def_index.is_empty() {
        return;
    }
    let mut repointed_from: HashSet<String> = HashSet::new();
    for edge in all_edges.iter_mut() {
        if edge.relation != "imports" {
            continue;
        }
        let (Some(using_kind), Some(target_fqn)) = (
            edge_meta_str(edge, "using_kind").map(str::to_string),
            edge_meta_str(edge, "target_fqn").map(str::to_string),
        ) else {
            continue;
        };
        if target_fqn.is_empty() {
            continue;
        }
        let resolved = match using_kind.as_str() {
            "namespace" => ns_by_label.get(&target_fqn).cloned(),
            "alias" => {
                let base = strip_trailing_csharp_generic_args(&html_unescape_min(&target_fqn));
                match base.rsplit_once('.') {
                    Some((prefix, name)) if ns_by_label.contains_key(prefix) => type_def_index
                        .get(&(prefix.to_string(), name.to_string()))
                        .cloned(),
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(r) = resolved
            && r != edge.target
        {
            let old = std::mem::replace(&mut edge.target, r);
            if !old.is_empty() {
                repointed_from.insert(old);
            }
        }
    }
    if repointed_from.is_empty() {
        return;
    }
    let still: HashSet<&str> = all_edges
        .iter()
        .flat_map(|e| [e.source.as_str(), e.target.as_str()])
        .collect();
    all_nodes.retain(|n| !repointed_from.contains(&n.id) || still.contains(n.id.as_str()));
}

/// A `using` with its lexical scope: `("file", None)` applies file-wide;
/// `("namespace", scope_id)` applies only where `scope_id` is in the ref's chain.
type ScopedUsing = (String, String, Option<String>);
/// Per-file namespace `using`s / alias `using`s keyed by referencing `.cs` path.
type NsUsings = HashMap<String, Vec<ScopedUsing>>;
type Aliases = HashMap<String, HashMap<String, Vec<ScopedUsing>>>;

/// `true` if a scoped using is visible from a reference with `scope_chain`.
fn using_in_scope(scope_kind: &str, scope_id: Option<&String>, scope_chain: &[String]) -> bool {
    scope_kind == "file" || scope_id.is_some_and(|sid| scope_chain.contains(sid))
}

fn append_unique(items: &mut Vec<String>, value: String) {
    if !items.contains(&value) {
        items.push(value);
    }
}

/// Read-only per-graph resolution indexes shared by the C# type-reference passes.
struct CsharpResolveCtx {
    type_def_index: HashMap<(String, String), String>,
    known_namespaces: HashSet<String>,
    node_ns: HashMap<String, String>,
    node_scope_chain: HashMap<String, Vec<String>>,
    ns_usings: NsUsings,
    aliases: Aliases,
}

impl CsharpResolveCtx {
    /// In-scope namespaces for a reference: its own namespace, the global
    /// namespace, and each in-scope `using N;`. Mirrors `_scopes_for`.
    fn scopes_for(&self, source_id: &str, source_file: &str) -> Vec<String> {
        let mut scopes = Vec::new();
        append_unique(
            &mut scopes,
            self.node_ns.get(source_id).cloned().unwrap_or_default(),
        );
        append_unique(&mut scopes, String::new());
        let empty = Vec::new();
        let chain = self.node_scope_chain.get(source_id).unwrap_or(&empty);
        if let Some(usings) = self.ns_usings.get(source_file) {
            for (ns, sk, sid) in usings {
                if using_in_scope(sk, sid.as_ref(), chain) {
                    append_unique(&mut scopes, ns.clone());
                }
            }
        }
        scopes
    }

    /// Resolve an in-scope alias `label` to a unique type definition. Mirrors
    /// `_resolve_alias`.
    fn resolve_alias(&self, label: &str, source_id: &str, source_file: &str) -> Option<String> {
        let empty = Vec::new();
        let chain = self.node_scope_chain.get(source_id).unwrap_or(&empty);
        let mut hits: HashSet<String> = HashSet::new();
        if let Some(entries) = self.aliases.get(source_file).and_then(|m| m.get(label)) {
            for (target_fqn, sk, sid) in entries {
                if !using_in_scope(sk, sid.as_ref(), chain) {
                    continue;
                }
                let base = strip_trailing_csharp_generic_args(&html_unescape_min(target_fqn));
                let (namespace, simple) = match base.rsplit_once('.') {
                    Some((p, t)) => (p.to_string(), t.to_string()),
                    None => (String::new(), String::new()),
                };
                if simple.is_empty() {
                    continue;
                }
                if let Some(hit) = self.type_def_index.get(&(namespace, simple)) {
                    hits.insert(hit.clone());
                }
            }
        }
        if hits.len() == 1 {
            hits.into_iter().next()
        } else {
            None
        }
    }

    /// Resolve a bare `label`: an in-scope alias shadows, else a unique in-scope
    /// namespace provides it. Mirrors `_resolve_label`.
    fn resolve_label(&self, label: &str, source_id: &str, source_file: &str) -> Option<String> {
        if self
            .aliases
            .get(source_file)
            .is_some_and(|m| m.contains_key(label))
        {
            return self.resolve_alias(label, source_id, source_file);
        }
        let mut candidates: Vec<String> = Vec::new();
        for ns in self.scopes_for(source_id, source_file) {
            if let Some(hit) = self.type_def_index.get(&(ns, label.to_string()))
                && !candidates.contains(hit)
            {
                candidates.push(hit.clone());
            }
        }
        if candidates.len() == 1 {
            candidates.pop()
        } else {
            None
        }
    }

    /// Resolve a qualified `Q.label`: an in-scope alias for `Q` shadows the
    /// namespace `Q`, else an exact known namespace `Q`. Mirrors
    /// `_resolve_qualified`.
    fn resolve_qualified(
        &self,
        label: &str,
        qualifier: Option<&str>,
        source_id: &str,
        source_file: &str,
    ) -> Option<String> {
        let qualifier = qualifier.filter(|q| !q.is_empty())?;
        let empty = Vec::new();
        let chain = self.node_scope_chain.get(source_id).unwrap_or(&empty);
        let in_scope: Vec<&ScopedUsing> = self
            .aliases
            .get(source_file)
            .and_then(|m| m.get(qualifier))
            .map(|v| {
                v.iter()
                    .filter(|(_, sk, sid)| using_in_scope(sk, sid.as_ref(), chain))
                    .collect()
            })
            .unwrap_or_default();
        if !in_scope.is_empty() {
            let mut hits: HashSet<String> = HashSet::new();
            for (target_fqn, _, _) in in_scope {
                let alias_ns = strip_trailing_csharp_generic_args(&html_unescape_min(target_fqn));
                if let Some(hit) = self.type_def_index.get(&(alias_ns, label.to_string())) {
                    hits.insert(hit.clone());
                }
            }
            return if hits.len() == 1 {
                hits.into_iter().next()
            } else {
                None
            };
        }
        if self.known_namespaces.contains(qualifier) {
            return self
                .type_def_index
                .get(&(qualifier.to_string(), label.to_string()))
                .cloned();
        }
        None
    }
}

/// The reference token for a type-ref edge target when no `ref_token` metadata is
/// present: the target's label, resolving a `*.cs` file label back to an alias or
/// stem. Mirrors `_label_for_type_ref_target`.
fn cs_label_for_type_ref_target(
    target_id: &str,
    source_file: &str,
    node_label: &HashMap<String, String>,
    aliases: &Aliases,
) -> Option<String> {
    let label = node_label.get(target_id)?;
    if label.is_empty() {
        return None;
    }
    let Some(stem) = label.strip_suffix(".cs") else {
        return Some(label.clone());
    };
    if let Some(m) = aliases.get(source_file) {
        for alias in m.keys() {
            if alias.eq_ignore_ascii_case(stem) || make_id(&[alias]) == make_id(&[stem]) {
                return Some(alias.clone());
            }
        }
    }
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_string())
    }
}

/// Arbitrate every C# `inherits`/`implements`/`references` target using only the
/// graph-stamped namespace/import/ref metadata: keep a binding only when the
/// referenced name resolves to one in-scope real type definition, else leave it
/// on a dangling stub. Mirrors graphify-py `_resolve_csharp_type_references`.
#[allow(clippy::too_many_lines)] // one soundness gate: index build + per-edge resolution + stub creation
pub(super) fn resolve_csharp_type_references(
    _cs_paths: &[PathBuf],
    all_nodes: &mut Vec<Node>,
    all_edges: &mut [Edge],
) {
    let type_def_index = build_csharp_type_def_index(all_nodes);
    let known_namespaces: HashSet<String> = all_nodes
        .iter()
        .filter(|n| n.node_type.as_deref() == Some("namespace"))
        .map(|n| n.label.clone())
        .collect();

    let mut node_ns: HashMap<String, String> = HashMap::new();
    let mut node_scope_chain: HashMap<String, Vec<String>> = HashMap::new();
    let mut node_label: HashMap<String, String> = HashMap::new();
    let mut placeholder: HashSet<String> = HashSet::new();
    let mut placeholder_ids_by_label: HashMap<String, Vec<String>> = HashMap::new();
    let mut existing_ids: HashSet<String> = HashSet::new();
    let mut cs_relevant: HashSet<String> = HashSet::new();
    for n in all_nodes.iter() {
        existing_ids.insert(n.id.clone());
        node_label.insert(n.id.clone(), n.label.clone());
        if let Some(ns) = node_meta_str(n, "namespace") {
            node_ns.insert(n.id.clone(), ns.to_string());
        }
        if let Some(arr) = n
            .metadata
            .as_ref()
            .and_then(|m| m.get("scope_chain"))
            .and_then(Value::as_array)
        {
            node_scope_chain.insert(
                n.id.clone(),
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
            );
        }
        if n.source_file.is_empty() {
            placeholder.insert(n.id.clone());
            placeholder_ids_by_label
                .entry(n.label.clone())
                .or_default()
                .push(n.id.clone());
        }
        if n.node_type.as_deref() == Some("namespace")
            || n.source_file.is_empty()
            || n.source_file.ends_with(".cs")
        {
            cs_relevant.insert(n.id.clone());
        }
    }

    let mut ns_usings: NsUsings = HashMap::new();
    let mut aliases: Aliases = HashMap::new();
    for e in all_edges.iter() {
        if e.relation != "imports" || !e.source_file.ends_with(".cs") {
            continue;
        }
        let Some(target_fqn) = edge_meta_str(e, "target_fqn") else {
            continue;
        };
        let scope_kind = edge_meta_str(e, "scope_kind").unwrap_or("file").to_string();
        let scope_id = edge_meta_str(e, "scope_id").map(str::to_string);
        match edge_meta_str(e, "using_kind") {
            Some("namespace") => {
                let entry = (target_fqn.to_string(), scope_kind, scope_id);
                let bucket = ns_usings.entry(e.source_file.clone()).or_default();
                if !bucket.contains(&entry) {
                    bucket.push(entry);
                }
            }
            Some("alias") => {
                if let Some(alias) = edge_meta_str(e, "alias") {
                    let entry = (target_fqn.to_string(), scope_kind, scope_id);
                    let bucket = aliases
                        .entry(e.source_file.clone())
                        .or_default()
                        .entry(alias.to_string())
                        .or_default();
                    if !bucket.contains(&entry) {
                        bucket.push(entry);
                    }
                }
            }
            _ => {}
        }
    }

    let ctx = CsharpResolveCtx {
        type_def_index,
        known_namespaces,
        node_ns,
        node_scope_chain,
        ns_usings,
        aliases,
    };

    let mut repointed_from: HashSet<String> = HashSet::new();
    let mut new_stubs: Vec<Node> = Vec::new();
    for e in all_edges.iter_mut() {
        if !CSHARP_REPOINT_RELATIONS.contains(&e.relation.as_str())
            || !e.source_file.ends_with(".cs")
            || !cs_relevant.contains(&e.target)
        {
            continue;
        }
        let source_id = e.source.clone();
        let source_file = e.source_file.clone();
        let current_target = e.target.clone();
        let qualified = edge_meta_bool(e, "qualified");
        let ref_qualifier = edge_meta_str(e, "ref_qualifier").map(str::to_string);
        let label = edge_meta_str(e, "ref_token")
            .map(str::to_string)
            .or_else(|| {
                cs_label_for_type_ref_target(
                    &current_target,
                    &source_file,
                    &node_label,
                    &ctx.aliases,
                )
            });
        let Some(label) = label else {
            continue;
        };
        let resolved = if qualified {
            ctx.resolve_qualified(&label, ref_qualifier.as_deref(), &source_id, &source_file)
        } else {
            ctx.resolve_label(&label, &source_id, &source_file)
        };
        let desired = match resolved {
            Some(r) => r,
            None => {
                if placeholder.contains(&current_target)
                    && node_label.get(&current_target).map(String::as_str) == Some(label.as_str())
                {
                    current_target.clone()
                } else if let Some(first) = placeholder_ids_by_label
                    .get(&label)
                    .and_then(|ids| ids.first())
                {
                    first.clone()
                } else {
                    let mut stub_id = make_id(&[&label]);
                    if existing_ids.contains(&stub_id) {
                        stub_id = make_id(&["csharp_type_ref", &label]);
                        let mut suffix = 2;
                        while existing_ids.contains(&stub_id) {
                            let s = suffix.to_string();
                            stub_id = make_id(&["csharp_type_ref", &label, &s]);
                            suffix += 1;
                        }
                    }
                    new_stubs.push(Node {
                        id: stub_id.clone(),
                        label: label.clone(),
                        file_type: "code".to_string(),
                        source_file: String::new(),
                        source_location: Some(String::new()),
                        node_type: None,
                        metadata: None,
                        origin_file: None,
                    });
                    existing_ids.insert(stub_id.clone());
                    placeholder.insert(stub_id.clone());
                    placeholder_ids_by_label
                        .entry(label.clone())
                        .or_default()
                        .push(stub_id.clone());
                    stub_id
                }
            }
        };
        if desired != current_target {
            let old_was_placeholder = placeholder.contains(&current_target);
            e.target = desired;
            if old_was_placeholder {
                repointed_from.insert(current_target);
            }
        }
    }
    all_nodes.append(&mut new_stubs);
    if repointed_from.is_empty() {
        return;
    }
    let still: HashSet<&str> = all_edges
        .iter()
        .flat_map(|e| [e.source.as_str(), e.target.as_str()])
        .collect();
    all_nodes.retain(|n| !repointed_from.contains(&n.id) || still.contains(n.id.as_str()));
}

// ── Cross-file C# member-call resolution (#1609) ──────────────────────────────

/// Normalise a C# type/method label to a comparison key (drop punctuation, fold).
/// Mirrors the inner `_key` of graphify-py `_resolve_csharp_member_calls`.
fn cs_key(label: &str) -> String {
    label
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_lowercase()
}

/// Resolve C# member calls (`recv.Method()`) to the receiver's declared type, so
/// a call binds to the ONE owning type's method instead of any same-named method
/// in the corpus (which silently mis-bound `_server.Save()` to `Cache.Save()`).
///
/// `this` binds to the caller's enclosing type (exact); a capitalised receiver is
/// itself the type (exact), falling back to the file table when its name is not a
/// unique type; a lower-cased field/property/param/local resolves via the
/// extractor's `receiver_type` (inferred). A missing, ambiguous, or method-less
/// target is skipped rather than guessed (single-definition god-node guard).
/// Purely additive: only handles member calls the shared pass deferred. Mirrors
/// graphify-py `_resolve_csharp_member_calls`.
pub(super) fn resolve_csharp_member_calls(
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
            && super::java::is_type_like_definition(n)
        {
            type_def_nids
                .entry(cs_key(&n.label))
                .or_default()
                .push(n.id.clone());
        }
    }

    // (owner_type_nid, method_key) -> method id (last wins), and method -> owner.
    let mut method_index: HashMap<(String, String), String> = HashMap::new();
    let mut enclosing_type: HashMap<String, String> = HashMap::new();
    for e in all_edges.iter() {
        if e.relation != "method" {
            continue;
        }
        let Some(mnode) = node_by_id.get(e.target.as_str()) else {
            continue;
        };
        enclosing_type
            .entry(e.target.clone())
            .or_insert_with(|| e.source.clone());
        method_index.insert((e.source.clone(), cs_key(&mnode.label)), e.target.clone());
    }

    let mut existing_pairs: HashSet<(String, String)> = all_edges
        .iter()
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();

    let unique_type = |name: &str| -> Option<&str> {
        match type_def_nids.get(&cs_key(name)) {
            Some(defs) if defs.len() == 1 => defs.first().map(String::as_str),
            _ => None,
        }
    };

    let mut new_edges: Vec<Edge> = Vec::new();
    for rc in all_raw_calls {
        if !rc.source_file.ends_with(".cs") || !rc.is_member_call {
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
            // `Type.M()` — the type is named explicitly; fall back to the file
            // table when the receiver name is not itself a unique type.
            let resolved = unique_type(receiver)
                .or_else(|| rc.receiver_type.as_deref().and_then(&unique_type));
            match resolved {
                Some(t) => (t.to_string(), true),
                None => continue,
            }
        } else {
            // `recv.M()` — typed via the extractor's file table (INFERRED).
            let Some(type_name) = rc.receiver_type.as_deref() else {
                continue;
            };
            match unique_type(type_name) {
                Some(t) => (t.to_string(), false),
                None => continue, // ambiguous or absent -> god-node guard
            }
        };

        let Some(method_nid) = method_index.get(&(type_nid, cs_key(&rc.callee))) else {
            continue; // receiver typed, but the type has no such method
        };
        let method_nid = method_nid.clone();
        if method_nid == caller || !existing_pairs.insert((caller.to_string(), method_nid.clone()))
        {
            continue;
        }
        new_edges.push(Edge {
            external: false,
            source: caller.to_string(),
            target: method_nid,
            relation: "calls".to_string(),
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
