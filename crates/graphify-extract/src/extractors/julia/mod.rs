//! Julia extractor — custom walk over tree-sitter-julia AST.

mod calls;
mod walk;

use crate::ids::{file_stem, make_id1};
use crate::types::{Edge, FileResult, Node};
use calls::{JuliaCallCtx, walk_calls_julia, walk_calls_julia_children};
use std::collections::HashSet;
use std::path::Path;
use walk::{JuliaWalkCtx, walk_julia};

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract modules, structs, functions, imports, and calls from a `.jl` file.
#[must_use]
// Crossed 100 lines only after each `Node`/`Edge` literal gained a `node_type` /
// `metadata` field (#1562); still a linear per-node-kind extraction.
#[allow(clippy::too_many_lines)]
pub fn extract_julia(path: &Path) -> FileResult {
    let source = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return FileResult {
                nodes: vec![],
                edges: vec![],
                raw_calls: vec![],
                error: Some(e.to_string()),
            };
        }
    };

    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_julia::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set julia language".to_string()),
        };
    }
    let Some(tree) = parser.parse(&source, None) else {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("parse failed".to_string()),
        };
    };

    let stem = file_stem(path);
    let str_path = path.to_string_lossy().into_owned();

    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    // (func_nid, node_start_byte, node_end_byte, is_function_def)
    let mut function_bodies: Vec<(String, usize, usize, bool)> = Vec::new();

    let file_nid = make_id1(&str_path);
    seen_ids.insert(file_nid.clone());
    nodes.push(Node {
        id: file_nid.clone(),
        label: path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: None,
        origin_file: None,
        node_type: None,
    });

    let root = tree.root_node();
    {
        let mut walk_ctx = JuliaWalkCtx {
            str_path: &str_path,
            stem: &stem,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            function_bodies: &mut function_bodies,
        };
        walk_julia(&mut walk_ctx, root, &source, &file_nid);
    }

    // Second pass: call edges
    {
        let mut call_ctx = JuliaCallCtx {
            str_path: &str_path,
            stem: &stem,
            edges: &mut edges,
            seen_ids: &seen_ids,
        };
        for (func_nid, node_start, node_end, is_func_def) in &function_bodies {
            let tree_root = tree.root_node();
            if *is_func_def {
                walk_calls_julia_children(
                    &mut call_ctx,
                    tree_root,
                    &source,
                    func_nid,
                    *node_start,
                    *node_end,
                );
            } else {
                walk_calls_julia(
                    &mut call_ctx,
                    tree_root,
                    &source,
                    func_nid,
                    *node_start,
                    *node_end,
                );
            }
        }
    }

    crate::forward_refs::reconcile_forward_refs(&mut nodes, &mut edges);
    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}
