//! Cross-file JS/TS default-import + barrel re-export resolution.
#![allow(clippy::case_sensitive_file_extension_comparisons)]

use super::{JsDefaultResolution, relativise_under_root};
use crate::import_handlers::make_edge;
use crate::types::{Edge, Node};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// The tree-sitter grammar for a JS/TS file, by extension (vue/others skipped).
fn js_grammar_for(path: &Path) -> Option<tree_sitter::Language> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("ts") => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        Some("tsx") => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        Some("js" | "jsx" | "mjs" | "cjs") => Some(tree_sitter_javascript::LANGUAGE.into()),
        _ => None,
    }
}

/// UTF-8 slice of a node's source span (empty on invalid UTF-8).
pub(super) fn js_node_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Local name of a default export, or `None` for an anonymous default.
///
/// Handles `export default class Foo {}` / `export default function foo() {}`
/// (name on the `declaration` field) and `export default Foo` (identifier on
/// the `value` field). Mirrors graphify-py `_js_default_export_name`.
fn js_default_export_name(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut c = node.walk();
    if !node.children(&mut c).any(|ch| ch.kind() == "default") {
        return None;
    }
    if let Some(decl) = node.child_by_field_name("declaration") {
        return decl
            .child_by_field_name("name")
            .map(|n| js_node_text(n, source).to_string());
    }
    let value = node.child_by_field_name("value")?;
    (value.kind() == "identifier").then(|| js_node_text(value, source).to_string())
}

/// Local binding of a default import — the `Foo` in `import Foo from './x'`
/// (also the leading binding of `import Foo, { Bar } from './x'`). Mirrors
/// graphify-py `_js_default_import_name`.
fn js_default_import_name(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut c = node.walk();
    let clause = node
        .children(&mut c)
        .find(|ch| ch.kind() == "import_clause")?;
    let mut cc = clause.walk();
    clause
        .children(&mut cc)
        .find(|sub| sub.kind() == "identifier")
        .map(|id| js_node_text(id, source).to_string())
}

/// The source-module string literal (`'./x'`) of an import/export statement.
fn js_import_source(node: tree_sitter::Node<'_>, source: &[u8]) -> Option<String> {
    let mut c = node.walk();
    let s = node.children(&mut c).find(|ch| ch.kind() == "string")?;
    Some(
        js_node_text(s, source)
            .trim_matches(|c| c == '\'' || c == '"' || c == '`' || c == ' ')
            .to_string(),
    )
}

/// A default import occurrence: `(file index, local binding, source string, line)`.
type JsDefaultImport = (usize, String, String, u32);

/// Default-export names (by file index) and default imports gathered per file.
struct JsDefaultFacts {
    export_name: HashMap<usize, String>,
    imports: Vec<JsDefaultImport>,
}

/// Parse each JS/TS file once, collecting its default-export name (by file
/// index) and its default imports. Files without a JS/TS grammar or that fail to
/// read/parse are skipped.
fn collect_js_default_facts(paths: &[PathBuf]) -> JsDefaultFacts {
    let mut export_name: HashMap<usize, String> = HashMap::new();
    let mut imports: Vec<JsDefaultImport> = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let Some(lang) = js_grammar_for(path) else {
            continue;
        };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&lang).is_err() {
            continue;
        }
        let Ok(source) = std::fs::read(path) else {
            continue;
        };
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            match node.kind() {
                "export_statement" => {
                    if let Some(name) = js_default_export_name(node, &source) {
                        export_name.entry(i).or_insert(name);
                    }
                }
                "import_statement" => {
                    if let Some(local) = js_default_import_name(node, &source)
                        && let Some(src) = js_import_source(node, &source)
                    {
                        let line = u32::try_from(node.start_position().row)
                            .unwrap_or(0)
                            .saturating_add(1);
                        imports.push((i, local, src, line));
                    }
                }
                _ => {}
            }
            let mut c = node.walk();
            stack.extend(node.children(&mut c));
        }
    }
    JsDefaultFacts {
        export_name,
        imports,
    }
}

/// Resolve JS/TS default imports to the origin symbol of the matching default
/// export across files (#6dc23db).
///
/// graphify-py threads default imports/exports through its
/// `_collect_js_symbol_resolution_facts` pass; the Rust port resolves JS imports
/// per-file, so this adds the cross-file default case as a focused resolver
/// parallel to [`resolve_cross_file_python_imports`] /
/// [`resolve_cross_file_java_imports`]. Runs after id remapping so it works in
/// the final node-id space. `all_nodes` is the post-remap node set.
pub(super) fn resolve_js_default_imports(
    all_nodes: &[Node],
    paths: &[PathBuf],
    root: &Path,
) -> JsDefaultResolution {
    use crate::ids::file_node_id;

    let file_nid_of = |path: &Path| -> String {
        let rel = relativise_under_root(path, root).unwrap_or_else(|| path.to_path_buf());
        file_node_id(&rel)
    };

    // (file_node_id, normalised label) -> node id, so a default-export name
    // resolves to the concrete symbol node in that file. The label is normalised
    // the same way the call resolver normalises call labels (strip a trailing
    // `()` and a leading `.`) so a function export (`makeFoo`, stored as the node
    // label `makeFoo()`) still matches the bare export name.
    let mut by_file_label: HashMap<(String, String), String> = HashMap::new();
    for n in all_nodes {
        if n.source_file.is_empty() || n.label.is_empty() {
            continue;
        }
        let sf = PathBuf::from(&n.source_file);
        let file_nid = if sf.is_absolute() {
            file_nid_of(&sf)
        } else {
            file_node_id(&sf)
        };
        let label = n.label.trim_end_matches("()").trim_start_matches('.');
        if label.is_empty() {
            continue;
        }
        by_file_label
            .entry((file_nid, label.to_string()))
            .or_insert_with(|| n.id.clone());
    }

    // Per file: default-export name + default imports.
    let JsDefaultFacts {
        export_name,
        imports,
    } = collect_js_default_facts(paths);

    // Match each canonicalised path to its index, so a resolved import target
    // maps back to the file whose default export we recorded.
    let mut idx_by_path: HashMap<PathBuf, usize> = HashMap::new();
    for (i, p) in paths.iter().enumerate() {
        idx_by_path.entry(p.clone()).or_insert(i);
        if let Ok(c) = p.canonicalize() {
            idx_by_path.entry(c).or_insert(i);
        }
    }

    let mut edges = Vec::new();
    let mut aliases = HashMap::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for (imp_idx, local, raw, line) in imports {
        let importer = &paths[imp_idx];
        let str_path = importer.to_string_lossy();
        let (_, resolved) = crate::generic::resolve_js_import_target(&raw, &str_path);
        let Some(resolved) = resolved else { continue };
        let tgt_idx = idx_by_path
            .get(&resolved)
            .or_else(|| {
                resolved
                    .canonicalize()
                    .ok()
                    .and_then(|c| idx_by_path.get(&c))
            })
            .copied();
        let Some(tgt_idx) = tgt_idx else { continue };
        let Some(name) = export_name.get(&tgt_idx) else {
            continue;
        };
        let tgt_file_nid = file_nid_of(&paths[tgt_idx]);
        let Some(origin) = by_file_label.get(&(tgt_file_nid, name.clone())) else {
            continue;
        };
        let importer_nid = file_nid_of(importer);
        if seen.insert((importer_nid.clone(), origin.clone())) {
            edges.push(make_edge(
                &importer_nid,
                origin,
                "imports",
                Some("import"),
                &str_path,
                line,
            ));
        }
        aliases.insert((importer_nid, local.to_lowercase()), origin.clone());
    }

    JsDefaultResolution { edges, aliases }
}

/// Per-file JS/TS export/import specifier facts used to resolve barrel
/// re-export chains to their origin symbols (#barrel-resolution). Collected by
/// [`collect_js_reexport_facts`].
#[derive(Default)]
struct JsReexportFile {
    /// `export { S as P } from './x'` → `(public, source_raw, source_name)`.
    reexports: Vec<(String, String, String)>,
    /// `export * from './x'` → `source_raw`.
    star_sources: Vec<String>,
    /// `export { L as P }` (no `from`) → `(public, local)`.
    local_reexports: Vec<(String, String)>,
    /// `export const X = …` → `X` (the public exported binding name).
    exported_const_names: Vec<String>,
    /// `import { I as L } from './x'` → `local → (source_raw, imported)`.
    named_imports: HashMap<String, (String, String)>,
    /// `const B = A` / `export const B = A` (bare-identifier RHS) → `alias → target`.
    local_aliases: HashMap<String, String>,
    /// Named imports as consumer facts: `(local_binding, source_raw, imported, line)`.
    consumer_imports: Vec<(String, String, String, u32)>,
}

/// Extract `(name, alias)` from an `import_specifier` / `export_specifier`.
fn js_spec_name_alias(
    spec: tree_sitter::Node<'_>,
    source: &[u8],
) -> Option<(String, Option<String>)> {
    let name = spec.child_by_field_name("name").or_else(|| {
        let mut c = spec.walk();
        spec.children(&mut c)
            .find(|n| matches!(n.kind(), "identifier" | "property_identifier"))
    })?;
    let alias = spec
        .child_by_field_name("alias")
        .map(|a| js_node_text(a, source).to_string());
    Some((js_node_text(name, source).to_string(), alias))
}

/// Record `const B = A` bare-identifier aliases from a `lexical_declaration`.
fn collect_js_lexical_aliases(node: tree_sitter::Node<'_>, source: &[u8], f: &mut JsReexportFile) {
    let mut cur = node.walk();
    for d in node.children(&mut cur) {
        if d.kind() == "variable_declarator"
            && let Some(name) = d.child_by_field_name("name")
            && let Some(value) = d.child_by_field_name("value")
            && value.kind() == "identifier"
        {
            f.local_aliases.insert(
                js_node_text(name, source).to_string(),
                js_node_text(value, source).to_string(),
            );
        }
    }
}

/// Record named imports (`import { I as L } from './x'`) from an `import_statement`.
fn collect_js_import_stmt(node: tree_sitter::Node<'_>, source: &[u8], f: &mut JsReexportFile) {
    let Some(src) = js_import_source(node, source) else {
        return;
    };
    let line = u32::try_from(node.start_position().row)
        .unwrap_or(0)
        .saturating_add(1);
    let mut cur = node.walk();
    for child in node.children(&mut cur) {
        if child.kind() != "import_clause" {
            continue;
        }
        let mut cc = child.walk();
        for sub in child.children(&mut cc) {
            if sub.kind() != "named_imports" {
                continue;
            }
            let mut nc = sub.walk();
            for spec in sub.children(&mut nc) {
                if spec.kind() == "import_specifier"
                    && let Some((name, alias)) = js_spec_name_alias(spec, source)
                {
                    let local = alias.unwrap_or_else(|| name.clone());
                    f.named_imports
                        .insert(local.clone(), (src.clone(), name.clone()));
                    f.consumer_imports.push((local, src.clone(), name, line));
                }
            }
        }
    }
}

/// Record re-exports / star re-exports / local re-exports / exported consts
/// from an `export_statement`.
fn collect_js_export_stmt(node: tree_sitter::Node<'_>, source: &[u8], f: &mut JsReexportFile) {
    let src = js_import_source(node, source);
    let mut cur = node.walk();
    let children: Vec<tree_sitter::Node<'_>> = node.children(&mut cur).collect();
    let export_clause = children
        .iter()
        .find(|c| c.kind() == "export_clause")
        .copied();
    let has_namespace = children.iter().any(|c| c.kind() == "namespace_export");
    let lexical = children
        .iter()
        .find(|c| c.kind() == "lexical_declaration")
        .copied();

    if let Some(clause) = export_clause {
        let mut cc = clause.walk();
        for spec in clause.children(&mut cc) {
            if spec.kind() == "export_specifier"
                && let Some((name, alias)) = js_spec_name_alias(spec, source)
            {
                let public = alias.unwrap_or_else(|| name.clone());
                match &src {
                    Some(s) => f.reexports.push((public, s.clone(), name)),
                    None => f.local_reexports.push((public, name)),
                }
            }
        }
    } else if let Some(s) = &src {
        if !has_namespace {
            f.star_sources.push(s.clone());
        }
    } else if let Some(lex) = lexical {
        collect_js_lexical_aliases(lex, source, f);
        let mut lc = lex.walk();
        for d in lex.children(&mut lc) {
            if d.kind() == "variable_declarator"
                && let Some(nn) = d.child_by_field_name("name")
            {
                f.exported_const_names
                    .push(js_node_text(nn, source).to_string());
            }
        }
    }
}

/// Parse each JS/TS file once, collecting its barrel re-export facts (indexed by
/// `paths` position). Files without a JS/TS grammar are recorded as empty.
fn collect_js_reexport_facts(paths: &[PathBuf]) -> Vec<JsReexportFile> {
    let mut out: Vec<JsReexportFile> = Vec::with_capacity(paths.len());
    for path in paths {
        let mut f = JsReexportFile::default();
        if let Some(lang) = js_grammar_for(path)
            && let Ok(source) = std::fs::read(path)
        {
            let mut parser = tree_sitter::Parser::new();
            if parser.set_language(&lang).is_ok()
                && let Some(tree) = parser.parse(&source, None)
            {
                let root = tree.root_node();
                let mut cur = root.walk();
                for stmt in root.children(&mut cur) {
                    match stmt.kind() {
                        "export_statement" => collect_js_export_stmt(stmt, &source, &mut f),
                        "import_statement" => collect_js_import_stmt(stmt, &source, &mut f),
                        "lexical_declaration" => collect_js_lexical_aliases(stmt, &source, &mut f),
                        _ => {}
                    }
                }
            }
        }
        out.push(f);
    }
    out
}

/// Re-export chain resolver over the collected [`JsReexportFile`] facts.
struct ReexportResolver<'a> {
    facts: &'a [JsReexportFile],
    idx_by_path: &'a HashMap<PathBuf, usize>,
    paths: &'a [PathBuf],
    file_nids: &'a [String],
    by_file_label: &'a HashMap<(String, String), String>,
}

impl ReexportResolver<'_> {
    /// `true` when `name` is declared as a real symbol node in file `idx`.
    fn is_declared(&self, idx: usize, name: &str) -> bool {
        self.by_file_label
            .contains_key(&(self.file_nids[idx].clone(), name.to_string()))
    }

    /// Resolve an import-source string (`'./x'`) to the `paths` index it targets.
    fn resolve_src(&self, file_idx: usize, src_raw: &str) -> Option<usize> {
        let str_path = self.paths[file_idx].to_string_lossy();
        let (_, resolved) = crate::generic::resolve_js_import_target(src_raw, &str_path);
        let resolved = resolved?;
        self.idx_by_path
            .get(&resolved)
            .or_else(|| {
                resolved
                    .canonicalize()
                    .ok()
                    .and_then(|c| self.idx_by_path.get(&c))
            })
            .copied()
    }

    /// Resolve `name` exported from file `file_idx` to its origin
    /// `(file_idx, declared_name)`, following named/aliased/star re-exports,
    /// local aliases, and named imports. `visited` guards against cycles.
    fn resolve(
        &self,
        file_idx: usize,
        name: &str,
        visited: &mut HashSet<(usize, String)>,
    ) -> Option<(usize, String)> {
        if !visited.insert((file_idx, name.to_string())) {
            return None;
        }
        let f = &self.facts[file_idx];
        for (public, src_raw, src_name) in &f.reexports {
            if public == name
                && let Some(tgt) = self.resolve_src(file_idx, src_raw)
                && let Some(r) = self.resolve(tgt, src_name, visited)
            {
                return Some(r);
            }
        }
        for (public, local) in &f.local_reexports {
            if public == name
                && local != name
                && let Some(r) = self.resolve(file_idx, local, visited)
            {
                return Some(r);
            }
        }
        if let Some(target) = f.local_aliases.get(name)
            && let Some(r) = self.resolve(file_idx, target, visited)
        {
            return Some(r);
        }
        if let Some((src_raw, imported)) = f.named_imports.get(name)
            && let Some(tgt) = self.resolve_src(file_idx, src_raw)
            && let Some(r) = self.resolve(tgt, imported, visited)
        {
            return Some(r);
        }
        for src_raw in &f.star_sources {
            if let Some(tgt) = self.resolve_src(file_idx, src_raw)
                && let Some(r) = self.resolve(tgt, name, visited)
            {
                return Some(r);
            }
        }
        if self.is_declared(file_idx, name) {
            return Some((file_idx, name.to_string()));
        }
        None
    }

    /// File→file `re_exports` edges for every barrel export that resolves to an
    /// origin file other than the barrel itself.
    fn reexport_edges(&self) -> Vec<Edge> {
        let mut edges = Vec::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for (idx, f) in self.facts.iter().enumerate() {
            let barrel_nid = &self.file_nids[idx];
            let str_path = self.paths[idx].to_string_lossy();
            let publics = f
                .reexports
                .iter()
                .map(|(p, _, _)| p)
                .chain(f.local_reexports.iter().map(|(p, _)| p))
                .chain(f.exported_const_names.iter());
            for public in publics {
                let mut visited = HashSet::new();
                if let Some((origin_idx, _)) = self.resolve(idx, public, &mut visited)
                    && origin_idx != idx
                    && seen.insert((barrel_nid.clone(), self.file_nids[origin_idx].clone()))
                {
                    edges.push(make_edge(
                        barrel_nid,
                        &self.file_nids[origin_idx],
                        "re_exports",
                        Some("re-export"),
                        &str_path,
                        1,
                    ));
                }
            }
            for src_raw in &f.star_sources {
                if let Some(tgt) = self.resolve_src(idx, src_raw)
                    && tgt != idx
                    && seen.insert((barrel_nid.clone(), self.file_nids[tgt].clone()))
                {
                    edges.push(make_edge(
                        barrel_nid,
                        &self.file_nids[tgt],
                        "re_exports",
                        Some("re-export"),
                        &str_path,
                        1,
                    ));
                }
            }
        }
        edges
    }

    /// Consumer `imports` edges + call aliases for named imports that travel
    /// through a barrel to an origin symbol in a different file.
    fn consumer_import_edges(&self) -> (Vec<Edge>, HashMap<(String, String), String>) {
        let mut edges = Vec::new();
        let mut aliases: HashMap<(String, String), String> = HashMap::new();
        let mut seen: HashSet<(String, String)> = HashSet::new();
        for (idx, f) in self.facts.iter().enumerate() {
            let consumer_nid = &self.file_nids[idx];
            let str_path = self.paths[idx].to_string_lossy();
            for (local, src_raw, imported, line) in &f.consumer_imports {
                let Some(barrel_idx) = self.resolve_src(idx, src_raw) else {
                    continue;
                };
                let mut visited = HashSet::new();
                let Some((origin_idx, origin_name)) =
                    self.resolve(barrel_idx, imported, &mut visited)
                else {
                    continue;
                };
                // origin == directly-imported file ⇒ plain import handled per-file.
                if origin_idx == barrel_idx {
                    continue;
                }
                let Some(origin_sym) = self
                    .by_file_label
                    .get(&(self.file_nids[origin_idx].clone(), origin_name.clone()))
                else {
                    continue;
                };
                if seen.insert((consumer_nid.clone(), origin_sym.clone())) {
                    edges.push(make_edge(
                        consumer_nid,
                        origin_sym,
                        "imports",
                        Some("import"),
                        &str_path,
                        *line,
                    ));
                }
                aliases.insert(
                    (consumer_nid.clone(), local.to_lowercase()),
                    origin_sym.clone(),
                );
            }
        }
        (edges, aliases)
    }
}

/// Resolve JS/TS named/aliased/star barrel re-export chains to their origin
/// symbols, emitting file→file `re_exports` edges, consumer→origin `imports`
/// edges, and call aliases (so a call through a barrel-imported binding targets
/// the origin symbol). Mirrors the observable output of graphify-py's
/// `_collect_js_symbol_resolution_facts` / `_apply_symbol_resolution_facts`
/// barrel handling, integrated with the existing per-file resolution.
pub(super) fn resolve_js_reexport_imports(
    all_nodes: &[Node],
    paths: &[PathBuf],
    root: &Path,
) -> JsDefaultResolution {
    use crate::ids::file_node_id;

    let file_nid_of = |path: &Path| -> String {
        let rel = relativise_under_root(path, root).unwrap_or_else(|| path.to_path_buf());
        file_node_id(&rel)
    };
    let mut by_file_label: HashMap<(String, String), String> = HashMap::new();
    for n in all_nodes {
        if n.source_file.is_empty() || n.label.is_empty() {
            continue;
        }
        let sf = PathBuf::from(&n.source_file);
        let file_nid = if sf.is_absolute() {
            file_nid_of(&sf)
        } else {
            file_node_id(&sf)
        };
        let label = n.label.trim_end_matches("()").trim_start_matches('.');
        if label.is_empty() {
            continue;
        }
        by_file_label
            .entry((file_nid, label.to_string()))
            .or_insert_with(|| n.id.clone());
    }

    let facts = collect_js_reexport_facts(paths);
    let mut idx_by_path: HashMap<PathBuf, usize> = HashMap::new();
    for (i, p) in paths.iter().enumerate() {
        idx_by_path.entry(p.clone()).or_insert(i);
        if let Ok(c) = p.canonicalize() {
            idx_by_path.entry(c).or_insert(i);
        }
    }
    let file_nids: Vec<String> = paths.iter().map(|p| file_nid_of(p)).collect();
    let resolver = ReexportResolver {
        facts: &facts,
        idx_by_path: &idx_by_path,
        paths,
        file_nids: &file_nids,
        by_file_label: &by_file_label,
    };

    let mut edges = resolver.reexport_edges();
    let (import_edges, aliases) = resolver.consumer_import_edges();
    edges.extend(import_edges);

    JsDefaultResolution { edges, aliases }
}
