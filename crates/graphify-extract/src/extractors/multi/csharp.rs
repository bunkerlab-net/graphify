//! C# cross-file type-reference resolution.
//!
//! Mirrors Python `graphify/extractors/csharp.py::_resolve_csharp_type_references`
//! — the C# counterpart to the Java resolver. It re-points dangling
//! `inherits`/`implements`/`references` edges that bare-name resolution left on
//! sourceless shadow stubs to the real definition, disambiguating same-named
//! types in different namespaces via each referencing file's `using` directives
//! and enclosing namespace. Ambiguous matches are refused rather than guessed
//! (the god-node guardrail).
//!
//! C# deltas from Java: a plain `using N;` is NAMESPACE-WIDE (resolve a bare
//! `T` by trying `(N, T)` for each open namespace and accepting only a UNIQUE
//! hit), while `using X = N.T;` is a single-type alias. `global using` is
//! normalised (the `global` prefix stripped); `using static N.T;` is ignored
//! (it imports members, not a namespace/type). The global namespace is keyed as
//! the bare label. A file with MULTIPLE namespace blocks does not register its
//! defs (which namespace each def belongs to needs source-range tracking) —
//! deferred.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::types::{Edge, Node};

/// C# edge relations re-pointed from sourceless shadow stubs to real defs.
const CSHARP_REPOINT_RELATIONS: &[&str] = &["implements", "inherits", "references"];

/// FQN key: the bare label in the global namespace, else `Namespace.Label`.
fn csharp_key(ns: &str, label: &str) -> String {
    if ns.is_empty() {
        label.to_string()
    } else {
        format!("{ns}.{label}")
    }
}

/// Recursively collect a file's namespace declarations, plain `using N;`
/// imports, and `using X = N.T;` aliases from a parsed C# tree.
fn collect_csharp_scope(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    own_ns: &mut Vec<String>,
    usings: &mut Vec<String>,
    aliases: &mut HashMap<String, String>,
) {
    match node.kind() {
        "namespace_declaration" | "file_scoped_namespace_declaration" => {
            if let Some(nm) = node.child_by_field_name("name") {
                own_ns.push(nm.utf8_text(source).unwrap_or("").trim().to_string());
            }
        }
        "using_directive" => {
            let raw = node
                .utf8_text(source)
                .unwrap_or("")
                .trim()
                .trim_end_matches(';');
            // `global using N;` is normalised to `using N;`.
            let text = raw.strip_prefix("global ").map_or(raw, str::trim);
            if let Some(body) = text.strip_prefix("using") {
                let body = body.trim();
                if body.starts_with("static ") {
                    // `using static N.T;` imports members, not a type/namespace — skip.
                } else if let Some((lhs, rhs)) = body.split_once('=') {
                    let (lhs, rhs) = (lhs.trim(), rhs.trim());
                    if !lhs.is_empty() && !rhs.is_empty() {
                        aliases.insert(lhs.to_string(), rhs.to_string());
                    }
                } else if !body.is_empty() {
                    usings.push(body.to_string());
                }
            }
        }
        _ => {}
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            collect_csharp_scope(cur.node(), source, own_ns, usings, aliases);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Re-point dangling C# `inherits`/`implements`/`references` edges left on
/// sourceless shadow stubs to the real definition, then drop the orphaned stubs.
///
/// Mirrors Python `_resolve_csharp_type_references`. Runs after id-disambiguation
/// and `rewire_unique_stub_nodes` (so it only handles the ambiguous remainder),
/// keyed by the absolute `source_file` strings the nodes/edges still carry
/// before the closing relativisation pass.
pub(super) fn resolve_csharp_type_references(
    cs_paths: &[PathBuf],
    all_nodes: &mut Vec<Node>,
    all_edges: &mut [Edge],
) {
    let Some((own_ns_by_file, scope_by_file, aliases_by_file)) = build_csharp_scopes(cs_paths)
    else {
        return;
    };

    // FQN -> definition node id, for source-backed type-like defs. A file with
    // multiple namespaces is skipped (def→namespace needs source-range tracking).
    let mut fqn_to_id: HashMap<String, String> = HashMap::new();
    for n in all_nodes.iter() {
        if n.label.is_empty() || n.source_file.is_empty() || n.id.is_empty() {
            continue;
        }
        let Some(ns_list) = own_ns_by_file.get(&n.source_file) else {
            continue;
        };
        let first_upper = n.label.chars().next().is_some_and(char::is_uppercase);
        if !first_upper || n.label.ends_with(')') || n.label.ends_with(".cs") {
            continue;
        }
        let key = match ns_list.as_slice() {
            [] => Some(csharp_key("", &n.label)),
            [ns] => Some(csharp_key(ns, &n.label)),
            // Multiple namespace blocks in one file (sibling OR nested, e.g.
            // `namespace A { namespace B { class T } }`) are flattened to a name
            // list that can't say which namespace a def belongs to, so registration
            // is skipped — byte-identical to graphify-py `csharp.py` (`# len > 1:
            // skip (deferred)`). Composing `A.B.T` here would resolve types
            // graphify-py leaves dangling and break byte-identical output; deferred
            // upstream pending source-range namespace tracking.
            _ => None,
        };
        if let Some(key) = key {
            fqn_to_id.entry(key).or_insert_with(|| n.id.clone());
        }
    }

    // Sourceless shadow stubs with a capitalised (type-like) label.
    let stub_label: HashMap<String, String> = all_nodes
        .iter()
        .filter(|n| {
            !n.id.is_empty()
                && n.source_file.is_empty()
                && n.label.chars().next().is_some_and(char::is_uppercase)
        })
        .map(|n| (n.id.clone(), n.label.clone()))
        .collect();
    if stub_label.is_empty() {
        return;
    }

    let mut repointed_from: HashSet<String> = HashSet::new();
    for edge in all_edges.iter_mut() {
        if !CSHARP_REPOINT_RELATIONS.contains(&edge.relation.as_str()) {
            continue;
        }
        let Some(label) = stub_label.get(&edge.target) else {
            continue;
        };
        let ref_file = edge.source_file.as_str();
        // 1. `using X = N.T;` alias resolves a single type.
        let mut resolved: Option<String> = aliases_by_file
            .get(ref_file)
            .and_then(|a| a.get(label))
            .and_then(|fqn| {
                let (ns, simple) = fqn.rsplit_once('.').unwrap_or(("", fqn.as_str()));
                fqn_to_id.get(&csharp_key(ns, simple))
            })
            .cloned();
        // 2. Namespace-wide `using N;` — accept only a UNIQUE hit across open
        //    namespaces (refuse ambiguity rather than guess).
        if resolved.is_none()
            && let Some(scope) = scope_by_file.get(ref_file)
        {
            let mut cands: Vec<String> = Vec::new();
            for ns in scope {
                if let Some(hit) = fqn_to_id.get(&csharp_key(ns, label))
                    && !cands.contains(hit)
                {
                    cands.push(hit.clone());
                }
            }
            if cands.len() == 1 {
                resolved = Some(cands.remove(0));
            }
        }
        if let Some(r) = resolved
            && r != edge.target
        {
            repointed_from.insert(std::mem::replace(&mut edge.target, r));
        }
    }
    if repointed_from.is_empty() {
        return;
    }

    // Drop shadow stubs that no edge references anymore.
    let still_referenced: HashSet<&str> = all_edges
        .iter()
        .flat_map(|e| [e.source.as_str(), e.target.as_str()])
        .collect();
    all_nodes
        .retain(|n| !repointed_from.contains(&n.id) || still_referenced.contains(n.id.as_str()));
}

/// Per-file C# resolution context: own namespaces, the `using` resolution scope,
/// and `using X = N.T;` aliases, keyed by absolute `source_file` path string.
type CsharpScopes = (
    HashMap<String, Vec<String>>,
    HashMap<String, Vec<String>>,
    HashMap<String, HashMap<String, String>>,
);

/// Parse every `.cs` file and build its namespace/using/alias scope. Returns
/// `None` only when the C# grammar fails to load.
fn build_csharp_scopes(cs_paths: &[PathBuf]) -> Option<CsharpScopes> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_c_sharp::LANGUAGE.into())
        .ok()?;

    let mut own_ns_by_file: HashMap<String, Vec<String>> = HashMap::new();
    let mut scope_by_file: HashMap<String, Vec<String>> = HashMap::new();
    let mut aliases_by_file: HashMap<String, HashMap<String, String>> = HashMap::new();
    for path in cs_paths {
        let Ok(source) = std::fs::read(path) else {
            continue;
        };
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let mut own_ns: Vec<String> = Vec::new();
        let mut usings: Vec<String> = Vec::new();
        let mut aliases: HashMap<String, String> = HashMap::new();
        collect_csharp_scope(
            tree.root_node(),
            &source,
            &mut own_ns,
            &mut usings,
            &mut aliases,
        );
        // scope = dedup((own_ns or [global]) + usings + [global]).
        // Parity dispute (CodeRabbit): a file with multiple `namespace` blocks
        // merges ALL their names into one combined resolution scope, so a bare
        // type declared in block A can in principle resolve via block B. graphify-py
        // `extractors/csharp.py` has the identical imprecision (`scope =
        // dict.fromkeys((own_ns or [""]) + usings + [""])` over an `own_ns` list
        // gathered across every block). A per-block scope (or excluding `own_ns`
        // when len > 1) would resolve fewer/different types than graphify-py and
        // break byte-identical output, so we match it deliberately.
        let mut scope: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let base = if own_ns.is_empty() {
            vec![String::new()]
        } else {
            own_ns.clone()
        };
        for s in base
            .into_iter()
            .chain(usings.iter().cloned())
            .chain(std::iter::once(String::new()))
        {
            if seen.insert(s.clone()) {
                scope.push(s);
            }
        }
        let src = path.to_string_lossy().into_owned();
        own_ns_by_file.insert(src.clone(), own_ns);
        scope_by_file.insert(src.clone(), scope);
        aliases_by_file.insert(src, aliases);
    }
    Some((own_ns_by_file, scope_by_file, aliases_by_file))
}
