//! Rust extractor — custom walk over tree-sitter-rust AST.

mod calls;
mod refs;
mod walk;

use crate::ids::{file_stem, make_id1};
use crate::types::{Edge, FileResult, Node, RawCall};
use calls::{RustCallCtx, walk_calls_rust};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::LazyLock;
use walk::{RustWalkCtx, walk_rust};

/// Common Rust trait/stdlib method names that appear in virtually every codebase.
/// Resolving these cross-file produces spurious INFERRED edges — skip them from
/// the unresolved-call queue entirely.
static RUST_TRAIT_METHOD_BLOCKLIST: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "new",
        "default",
        "parse",
        "from_str",
        "now",
        "clone",
        "into",
        "from",
        "to_string",
        "to_owned",
        "len",
        "is_empty",
        "iter",
        "next",
        "build",
        "start",
        "run",
        "init",
        "app",
        "get",
        "set",
        "push",
        "pop",
        "insert",
        "remove",
        "contains",
        "collect",
        "map",
        "filter",
        "unwrap",
        "expect",
        "ok",
        "err",
        "some",
        "none",
        "send",
        "recv",
        "lock",
        "read",
        "write",
    ]
    .into_iter()
    .collect()
});

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract functions, structs, enums, traits, impl methods, and use declarations from a `.rs` file.
#[must_use]
pub fn extract_rust(path: &Path) -> FileResult {
    let Some((source, tree)) = parse_rust_source(path) else {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("parse failed".to_string()),
        };
    };
    let stem = file_stem(path);
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

    {
        let mut walk_ctx = RustWalkCtx {
            str_path: &str_path,
            stem: &stem,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            function_bodies: &mut function_bodies,
        };
        walk_rust(&mut walk_ctx, tree.root_node(), &source, None);
    }

    let mut label_to_nid: HashMap<String, String> = HashMap::new();
    for n in &nodes {
        let normalised = n.label.trim_end_matches("()").trim_start_matches('.');
        label_to_nid.insert(normalised.to_lowercase(), n.id.clone());
    }

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();
    {
        let mut call_ctx = RustCallCtx {
            str_path: &str_path,
            label_to_nid: &label_to_nid,
            edges: &mut edges,
            seen_call_pairs: &mut seen_call_pairs,
            raw_calls: &mut raw_calls,
        };
        for (caller_nid, body_start, body_end) in &function_bodies {
            walk_calls_rust(
                &mut call_ctx,
                tree.root_node(),
                &source,
                caller_nid,
                *body_start,
                *body_end,
            );
        }
    }

    crate::forward_refs::reconcile_forward_refs(&mut nodes, &mut edges);
    // Validate dangling edges against the reconciled graph rather than the
    // now-stale `seen_ids`, which still lists any placeholder ids reconcile
    // folded away.
    let valid_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let clean_edges: Vec<Edge> = edges
        .into_iter()
        .filter(|e| {
            valid_ids.contains(&e.source)
                && (valid_ids.contains(&e.target)
                    || matches!(e.relation.as_str(), "imports" | "imports_from"))
        })
        .collect();
    FileResult {
        nodes,
        edges: clean_edges,
        raw_calls,
        error: None,
    }
}

/// Read + tree-sitter-parse a `.rs` file. `None` on any I/O or parse error.
fn parse_rust_source(path: &Path) -> Option<(Vec<u8>, tree_sitter::Tree)> {
    let source = std::fs::read(path).ok()?;
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(&source, None)?;
    Some((source, tree))
}
