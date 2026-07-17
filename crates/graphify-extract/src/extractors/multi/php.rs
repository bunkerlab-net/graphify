//! PHP namespace/`use` type-reference resolution (#1923).
//!
//! Disambiguates PHP `inherits`/`implements`/`mixes_in`/`imports`/`references`
//! targets using each file's `namespace` declaration and `use` imports. Mirrors
//! `_resolve_php_type_references` in graphify-py `extractors/resolution.py`.
//!
//! Unlike the Java/C# resolvers, this pass MUST run BEFORE
//! [`crate::postprocess::rewire_unique_stub_nodes`]: the false edge is
//! manufactured by the rewire itself — a bare `Page` stub collapses onto the
//! only internal class labelled `Page` even though the referencing file `use`d a
//! different namespace (`Filament\Pages\Page` vs `App\Models\Page`). References
//! proven external by a `use` FQN or a qualified name are re-pointed to an
//! FQN-labelled sourceless stub, which the bare-label rewire cannot collapse.
//! References with no namespace facts are left untouched so the unique-label
//! rewire keeps handling plain (non-namespaced) PHP as before.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ids::make_id1;
use crate::types::{Edge, Node};

/// Supertype relations whose raw AST text disambiguates a bare stub target.
const PHP_SUPERTYPE_RELATIONS: &[&str] = &["inherits", "implements", "mixes_in"];
/// Edge relations re-pointed from bare stubs to namespace-resolved targets.
const PHP_REPOINT_RELATIONS: &[&str] = &[
    "inherits",
    "implements",
    "mixes_in",
    "imports",
    "references",
];

/// Namespace/`use`/raw-reference facts collected from one PHP file.
struct PhpFacts {
    /// The file's single namespace (`""` for the global namespace).
    ns: String,
    /// Lowercased alias/simple-name → fully-qualified name from `use` clauses.
    uses: HashMap<String, String>,
    /// `(owning-class-lower, relation, lowercased-bare-name)` → the raw
    /// (possibly qualified) reference text, or `None` when two different raws
    /// share that key (e.g. `implements A\I, B\I` on one class — never guess).
    /// Class-scoping keeps `A extends X\Page` and `B extends Y\Page` in one file
    /// separately resolvable (#1923).
    raws: HashMap<(String, String, String), Option<String>>,
}

/// Resolve a raw (possibly qualified) PHP class reference to a fully-qualified
/// name. PHP class-name resolution:
/// - `\A\B` → absolute: `A\B`
/// - `A\B`  → first segment through the `use` map (group-prefix semantics),
///   else relative to the current namespace
/// - `B`    → `use` map, else current namespace (class names do NOT fall back
///   to the global namespace)
fn php_fqn_from_raw(raw: &str, ns: &str, uses: &HashMap<String, String>) -> String {
    let raw = raw.trim();
    if let Some(stripped) = raw.strip_prefix('\\') {
        return stripped.to_string();
    }
    if let Some((first, rest)) = raw.split_once('\\') {
        if let Some(mapped) = uses.get(&first.to_lowercase()) {
            return format!("{mapped}\\{rest}");
        }
        return if ns.is_empty() {
            raw.to_string()
        } else {
            format!("{ns}\\{raw}")
        };
    }
    if let Some(mapped) = uses.get(&raw.to_lowercase()) {
        return mapped.clone();
    }
    if ns.is_empty() {
        raw.to_string()
    } else {
        format!("{ns}\\{raw}")
    }
}

fn node_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

/// Accumulator threaded through [`walk_php_facts`].
struct FactsBuilder {
    namespaces: Vec<String>,
    uses: HashMap<String, String>,
    raws: HashMap<(String, String, String), Option<String>>,
}

impl FactsBuilder {
    /// Record a raw supertype reference under `(class-lower, relation, bare-lower)`,
    /// marking the key ambiguous (`None`) when a different raw already claimed it.
    /// Scoping by the owning class keeps two classes in one file that reference
    /// distinct FQNs sharing a bare name (`X\Page`, `Y\Page`) separately
    /// resolvable (#1923).
    fn record_raw(&mut self, class: &str, relation: &str, raw: &str) {
        let bare = raw.rsplit('\\').next().unwrap_or(raw).trim().to_lowercase();
        if bare.is_empty() {
            return;
        }
        let key = (class.to_lowercase(), relation.to_string(), bare);
        match self.raws.get(&key) {
            Some(Some(existing)) if existing != raw => {
                self.raws.insert(key, None);
            }
            Some(_) => {}
            None => {
                self.raws.insert(key, Some(raw.to_string()));
            }
        }
    }

    /// Parse one `namespace_use_clause`, recording `alias/simple → FQN` unless it
    /// is a `function`/`const` import.
    fn record_use_clause(&mut self, clause: tree_sitter::Node<'_>, prefix: &str, source: &[u8]) {
        let mut target: Option<String> = None;
        let mut alias: Option<String> = None;
        let mut saw_as = false;
        let mut cursor = clause.walk();
        for c in clause.children(&mut cursor) {
            match c.kind() {
                "function" | "const" => return, // not a class import
                "as" => saw_as = true,
                "qualified_name" | "name" => {
                    if saw_as {
                        alias = Some(node_text(c, source).to_string());
                    } else if target.is_none() {
                        target = Some(node_text(c, source).to_string());
                    }
                }
                _ => {}
            }
        }
        let Some(target) = target else { return };
        let fqn = if prefix.is_empty() {
            target
        } else {
            format!("{prefix}\\{target}")
        };
        let fqn = fqn.trim_start_matches('\\').to_string();
        let key = alias
            .unwrap_or_else(|| fqn.rsplit('\\').next().unwrap_or(&fqn).to_string())
            .trim()
            .to_lowercase();
        if !key.is_empty() {
            self.uses.entry(key).or_insert(fqn);
        }
    }
}

/// Recursively collect namespace/`use`/base-clause facts from a PHP AST.
fn walk_php_facts(node: tree_sitter::Node<'_>, source: &[u8], b: &mut FactsBuilder) {
    match node.kind() {
        "namespace_definition" => {
            // Only a NAMED namespace is recorded; an unnamed global `namespace {}`
            // block is ignored, matching graphify-py `resolution.py:2328-2332`
            // (appends only on a `namespace_name` child). Recording the global
            // block as a distinct namespace would diverge from the reference.
            let mut cursor = node.walk();
            for c in node.children(&mut cursor) {
                if c.kind() == "namespace_name" {
                    b.namespaces.push(node_text(c, source).to_string());
                    break;
                }
            }
        }
        "namespace_use_declaration" => {
            let mut prefix = String::new();
            let mut group: Option<tree_sitter::Node<'_>> = None;
            let mut cursor = node.walk();
            for c in node.children(&mut cursor) {
                match c.kind() {
                    "namespace_name" => prefix = node_text(c, source).to_string(),
                    "namespace_use_group" => group = Some(c),
                    "namespace_use_clause" => b.record_use_clause(c, "", source),
                    _ => {}
                }
            }
            if let Some(group) = group {
                let mut gc = group.walk();
                for c in group.children(&mut gc) {
                    if c.kind() == "namespace_use_clause" {
                        b.record_use_clause(c, &prefix, source);
                    }
                }
            }
            return; // do not recurse into the use declaration
        }
        "class_declaration" => {
            // The class's own simple name scopes its raw supertype refs, so two
            // classes in one file (`A extends X\Page`, `B extends Y\Page`) keep
            // separate qualified raws instead of colliding on the bare `page`
            // key and being marked ambiguous (#1923).
            let class_name = node
                .child_by_field_name("name")
                .map(|c| node_text(c, source).to_string())
                .unwrap_or_default();
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                match child.kind() {
                    "base_clause" => {
                        let mut sc = child.walk();
                        for sub in child.children(&mut sc) {
                            if matches!(sub.kind(), "name" | "qualified_name") {
                                b.record_raw(&class_name, "inherits", node_text(sub, source));
                            }
                        }
                    }
                    "class_interface_clause" => {
                        let mut sc = child.walk();
                        for sub in child.children(&mut sc) {
                            if matches!(sub.kind(), "name" | "qualified_name") {
                                b.record_raw(&class_name, "implements", node_text(sub, source));
                            }
                        }
                    }
                    "declaration_list" => {
                        let mut mc = child.walk();
                        for member in child.children(&mut mc) {
                            if member.kind() != "use_declaration" {
                                continue;
                            }
                            let mut uc = member.walk();
                            for sub in member.children(&mut uc) {
                                if matches!(sub.kind(), "name" | "qualified_name") {
                                    b.record_raw(&class_name, "mixes_in", node_text(sub, source));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_php_facts(child, source, b);
    }
}

/// Parse every PHP file, returning per-file facts keyed by the absolute
/// `source_file` string. A multi-namespace file (PSR-1 violation) is skipped so
/// the legacy unique-label rewire keeps handling it.
fn collect_php_facts(php_paths: &[PathBuf]) -> HashMap<String, PhpFacts> {
    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_php::LANGUAGE_PHP.into())
        .is_err()
    {
        return HashMap::new();
    }
    let mut facts: HashMap<String, PhpFacts> = HashMap::new();
    for path in php_paths {
        let src = path.to_string_lossy().into_owned();
        let Ok(source) = std::fs::read(path) else {
            continue;
        };
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let mut b = FactsBuilder {
            namespaces: Vec::new(),
            uses: HashMap::new(),
            raws: HashMap::new(),
        };
        walk_php_facts(tree.root_node(), &source, &mut b);
        let distinct: HashSet<&String> = b.namespaces.iter().collect();
        if distinct.len() > 1 {
            continue; // multi-namespace file: keep legacy behaviour
        }
        let ns = b.namespaces.first().cloned().unwrap_or_default();
        facts.insert(
            src,
            PhpFacts {
                ns,
                uses: b.uses,
                raws: b.raws,
            },
        );
    }
    facts
}

/// Lowercased FQN → definition node id, for source-backed class definitions in
/// files that produced namespace facts. PHP class names are case-insensitive.
fn build_fqn_index(
    all_nodes: &[Node],
    facts: &HashMap<String, PhpFacts>,
) -> HashMap<String, String> {
    let mut fqn_to_id: HashMap<String, String> = HashMap::new();
    for n in all_nodes {
        if n.label.is_empty() || n.source_file.is_empty() || n.id.is_empty() {
            continue;
        }
        let Some(f) = facts.get(&n.source_file) else {
            continue;
        };
        if n.label.ends_with(')') || n.label.contains('.') {
            continue; // methods / file nodes
        }
        let fqn = if f.ns.is_empty() {
            n.label.clone()
        } else {
            format!("{}\\{}", f.ns, n.label)
        };
        fqn_to_id.entry(fqn.to_lowercase()).or_insert(n.id.clone());
    }
    fqn_to_id
}

/// Re-point dangling PHP `inherits`/`implements`/`mixes_in`/`imports`/
/// `references` edges that bare-name resolution would otherwise collapse onto the
/// wrong same-named class, using each file's namespace + `use` imports (#1923).
///
/// Runs BEFORE [`crate::postprocess::rewire_unique_stub_nodes`], keyed by the
/// absolute `source_file` strings nodes/edges still carry before the closing
/// relativisation pass. `.blade.php` templates are excluded by the caller.
pub(super) fn resolve_php_type_references(
    php_paths: &[PathBuf],
    all_nodes: &mut Vec<Node>,
    all_edges: &mut [Edge],
) {
    let facts = collect_php_facts(php_paths);
    if facts.is_empty() {
        return;
    }
    let fqn_to_id = build_fqn_index(all_nodes, &facts);
    // id → owning class's lowercased simple name, to look up class-scoped raws.
    let id_to_class_lower: HashMap<String, String> = all_nodes
        .iter()
        .map(|n| (n.id.clone(), n.label.to_lowercase()))
        .collect();

    let mut node_ids: HashSet<String> = all_nodes.iter().map(|n| n.id.clone()).collect();
    // Bare sourceless stubs: id → label.
    let stub_label: HashMap<String, String> = all_nodes
        .iter()
        .filter(|n| !n.id.is_empty() && n.source_file.is_empty() && !n.label.is_empty())
        .map(|n| (n.id.clone(), n.label.clone()))
        .collect();

    let mut external_stub_ids: HashMap<String, String> = HashMap::new();
    let mut new_nodes: Vec<Node> = Vec::new();
    let mut repointed_from: HashSet<String> = HashSet::new();

    for edge in all_edges.iter_mut() {
        if !PHP_REPOINT_RELATIONS.contains(&edge.relation.as_str()) {
            continue;
        }
        let Some(f) = facts.get(&edge.source_file) else {
            continue;
        };
        let Some(label) = stub_label.get(&edge.target) else {
            continue;
        };
        let bare = label.trim().to_lowercase();

        let raw: Option<String> = if PHP_SUPERTYPE_RELATIONS.contains(&edge.relation.as_str()) {
            let src_class = id_to_class_lower
                .get(&edge.source)
                .cloned()
                .unwrap_or_default();
            f.raws
                .get(&(src_class, edge.relation.clone(), bare.clone()))
                .and_then(Clone::clone)
        } else {
            None
        };

        let mut explicit = false;
        let fqn = if let Some(raw) = raw.as_ref().filter(|r| r.contains('\\')) {
            explicit = true;
            php_fqn_from_raw(raw, &f.ns, &f.uses)
        } else if let Some(mapped) = f.uses.get(&bare) {
            explicit = true;
            mapped.clone()
        } else if !f.ns.is_empty() {
            format!("{}\\{label}", f.ns)
        } else {
            continue; // no namespace facts: legacy unique-label rewire applies
        };

        match fqn_to_id.get(&fqn.to_lowercase()).cloned() {
            Some(r) if r != edge.target => {
                repointed_from.insert(std::mem::replace(&mut edge.target, r));
            }
            None if explicit => {
                // Proven external: park the edge on an FQN-labelled stub the
                // bare-name rewire cannot collapse (this is the #1923 fix).
                let nid =
                    external_stub(&fqn, &mut external_stub_ids, &mut node_ids, &mut new_nodes);
                repointed_from.insert(std::mem::replace(&mut edge.target, nid));
            }
            _ => {} // non-explicit miss: leave the bare stub for the legacy rewire
        }
    }

    if !new_nodes.is_empty() {
        all_nodes.extend(new_nodes);
    }
    if repointed_from.is_empty() {
        return;
    }

    // Drop bare stubs no edge references anymore.
    let still_referenced: HashSet<&str> = all_edges
        .iter()
        .flat_map(|e| [e.source.as_str(), e.target.as_str()])
        .collect();
    all_nodes
        .retain(|n| !repointed_from.contains(&n.id) || still_referenced.contains(n.id.as_str()));
}

/// Return (creating if needed) the id of an FQN-labelled sourceless stub for an
/// external type. Cached by lowercased FQN; a fresh id is only materialised as a
/// node when it does not already exist in the graph.
fn external_stub(
    fqn: &str,
    cache: &mut HashMap<String, String>,
    node_ids: &mut HashSet<String>,
    new_nodes: &mut Vec<Node>,
) -> String {
    let key = fqn.to_lowercase();
    if let Some(nid) = cache.get(&key) {
        return nid.clone();
    }
    let nid = make_id1(fqn);
    if !node_ids.contains(&nid) {
        new_nodes.push(Node {
            id: nid.clone(),
            label: fqn.to_string(),
            file_type: "code".to_string(),
            source_file: String::new(),
            source_location: Some(String::new()),
            origin_file: None,
            node_type: None,
            metadata: None,
        });
        node_ids.insert(nid.clone());
    }
    cache.insert(key, nid.clone());
    nid
}
