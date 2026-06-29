//! Go extractor — custom walk over tree-sitter-go AST.

mod calls;
mod refs;
mod walk;

use crate::ids::{file_stem, make_id1};
use crate::types::{Edge, FileResult, Node, RawCall};
use calls::{GoCallCtx, walk_calls_go};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use walk::{GoWalkCtx, walk_go};

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract functions, methods, type declarations, and imports from a `.go` file.
#[must_use]
pub fn extract_go(path: &Path) -> FileResult {
    let Some((source, tree)) = parse_go_source(path) else {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("parse failed".to_string()),
        };
    };

    let stem = file_stem(path);
    let pkg_scope = derive_pkg_scope(path, &stem);
    let str_path = path.to_string_lossy().into_owned();
    let file_nid = make_id1(&str_path);

    let mut nodes: Vec<Node> = vec![Node {
        id: file_nid.clone(),
        label: path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: None,
        origin_file: None,
    }];
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::from([file_nid.clone()]);
    let mut function_bodies: Vec<(String, usize, usize)> = Vec::new();
    let mut go_imported_pkgs: HashSet<String> = HashSet::new();

    {
        let mut walk_ctx = GoWalkCtx {
            str_path: &str_path,
            stem: &stem,
            pkg_scope: &pkg_scope,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            function_bodies: &mut function_bodies,
            go_imported_pkgs: &mut go_imported_pkgs,
        };
        walk_go(&mut walk_ctx, tree.root_node(), &source);
    }

    let label_to_nid = build_go_label_map(&nodes);
    let raw_calls = resolve_go_function_calls(GoResolveArgs {
        tree: &tree,
        source: &source,
        str_path: &str_path,
        function_bodies: &function_bodies,
        label_to_nid: &label_to_nid,
        go_imported_pkgs: &go_imported_pkgs,
        edges: &mut edges,
    });
    crate::forward_refs::reconcile_forward_refs(&mut nodes, &mut edges);
    // Validate dangling edges against the reconciled graph: reconcile may have
    // folded placeholder nodes away, so rebuild the valid-id set from the
    // surviving nodes rather than trusting the now-stale `seen_ids`.
    let valid_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let clean_edges = filter_dangling_edges(edges, &valid_ids);

    FileResult {
        nodes,
        edges: clean_edges,
        raw_calls,
        error: None,
    }
}

/// Read the file and parse with tree-sitter-go. `None` on any I/O or parse error.
fn parse_go_source(path: &Path) -> Option<(Vec<u8>, tree_sitter::Tree)> {
    let source = std::fs::read(path).ok()?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_go::LANGUAGE.into()).ok()?;
    let tree = parser.parse(&source, None)?;
    Some((source, tree))
}

/// Use the directory name as package scope so methods on the same type share a
/// canonical type node across files in the same package.
fn derive_pkg_scope(path: &Path, fallback_stem: &str) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_stem)
        .to_string()
}

/// Build a `normalised_label → nid` map for intra-file call resolution.
fn build_go_label_map(nodes: &[Node]) -> HashMap<String, String> {
    let mut label_to_nid: HashMap<String, String> = HashMap::new();
    for n in nodes {
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        label_to_nid.insert(normalised.to_lowercase(), n.id.clone());
    }
    label_to_nid
}

/// Bundle of shared inputs for [`resolve_go_function_calls`].
struct GoResolveArgs<'a> {
    tree: &'a tree_sitter::Tree,
    source: &'a [u8],
    str_path: &'a str,
    function_bodies: &'a [(String, usize, usize)],
    label_to_nid: &'a HashMap<String, String>,
    go_imported_pkgs: &'a HashSet<String>,
    edges: &'a mut Vec<Edge>,
}

/// Walk each function body to emit call edges and `RawCall` entries.
fn resolve_go_function_calls(args: GoResolveArgs<'_>) -> Vec<RawCall> {
    let GoResolveArgs {
        tree,
        source,
        str_path,
        function_bodies,
        label_to_nid,
        go_imported_pkgs,
        edges,
    } = args;
    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();
    {
        let mut call_ctx = GoCallCtx {
            str_path,
            label_to_nid,
            go_imported_pkgs,
            edges,
            seen_call_pairs: &mut seen_call_pairs,
            raw_calls: &mut raw_calls,
        };
        for (caller_nid, body_start, body_end) in function_bodies {
            walk_calls_go(
                &mut call_ctx,
                tree.root_node(),
                source,
                caller_nid,
                *body_start,
                *body_end,
            );
        }
    }
    raw_calls
}

/// Drop edges whose endpoints aren't in `valid_ids` (except for `imports` edges).
fn filter_dangling_edges(edges: Vec<Edge>, valid_ids: &HashSet<String>) -> Vec<Edge> {
    edges
        .into_iter()
        .filter(|e| {
            valid_ids.contains(&e.source)
                && (valid_ids.contains(&e.target)
                    || matches!(e.relation.as_str(), "imports" | "imports_from"))
        })
        .collect()
}
