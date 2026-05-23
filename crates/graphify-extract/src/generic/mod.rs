//! Generic tree-sitter extractor driven by `LangConfig`.
//!
//! This module is the public face of the generic extraction pipeline.
//! It re-exports the `LangConfig` / `LangId` type API from `config`, the
//! name-resolution helpers from `names`, and the JS import resolver from
//! `js_extra`.  The `extract_generic` entry point orchestrates the two-pass
//! walk (structural pass → call-graph pass) defined in `walk`.
//!
//! Submodule layout:
//! - `config`   — `LangConfig`, `LangId`, function-pointer typedefs.
//! - `names`    — `get_c_func_name`, `get_cpp_func_name`, text helpers.
//! - `inherit`  — per-language inheritance-edge emitters.
//! - `js_extra` — JS/TS arrow-function, CJS require, dynamic import logic.
//! - `walk`     — main structural walk + call-graph pass.

mod calls;
pub mod config;
mod inherit;
mod js_extra;
mod names;
mod walk;

pub use config::{ImportHandlerFn, LangConfig, LangId, ResolveFnNameFn};
pub use js_extra::resolve_js_import_target;
pub use names::{get_c_func_name, get_cpp_func_name};

use std::collections::{HashMap, HashSet};
use std::path::Path;

use tree_sitter::Parser;

use crate::ids::{file_stem, make_id1};
use crate::types::{Edge, FileResult, Node as GNode, RawCall};

use calls::walk_calls;
use walk::{add_node, walk};

/// Extract nodes and edges from `path` using the given language configuration.
///
/// Mirrors Python `_extract_generic(path, config)`.
#[must_use]
pub fn extract_generic(path: &Path, config: &LangConfig) -> FileResult {
    let source = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            return FileResult::error(format!("io error reading {}: {e}", path.display()));
        }
    };

    let mut parser = Parser::new();
    if let Err(e) = parser.set_language(&config.language) {
        return FileResult::error(format!(
            "parser language mismatch for {}: {e}",
            path.display()
        ));
    }

    let Some(tree) = parser.parse(&source, None) else {
        return FileResult::error(format!("tree-sitter parse failed for {}", path.display()));
    };

    let root = tree.root_node();
    let stem = file_stem(path);
    let str_path = path.to_string_lossy().into_owned();

    let mut nodes: Vec<GNode> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut function_bodies: Vec<(String, tree_sitter::Node<'_>)> = Vec::new();

    let file_nid = make_id1(&str_path);
    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    add_node(
        &file_nid,
        &filename,
        1,
        &str_path,
        &mut nodes,
        &mut seen_ids,
    );

    // ── Structural walk ───────────────────────────────────────────────────────
    let mut cur = root.walk();
    if cur.goto_first_child() {
        let mut walk_ctx = super::generic::walk::WalkCtx {
            config,
            file_nid: &file_nid,
            stem: &stem,
            str_path: &str_path,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            function_bodies: &mut function_bodies,
        };
        loop {
            let child = cur.node();
            walk(&mut walk_ctx, child, None, &source);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }

    // ── Call-graph pass ───────────────────────────────────────────────────────
    let label_to_nid: HashMap<String, String> = nodes
        .iter()
        .map(|n| {
            let key = n
                .label
                .trim_start_matches('.')
                .trim_end_matches("()")
                .to_lowercase();
            (key, n.id.clone())
        })
        .collect();

    let mut seen_call_pairs: HashSet<(String, String)> = HashSet::new();
    let mut seen_dyn_import_pairs: HashSet<(String, String)> = HashSet::new();
    let mut raw_calls: Vec<RawCall> = Vec::new();

    {
        let mut call_ctx = super::generic::calls::CallWalkCtx {
            config,
            str_path: &str_path,
            label_to_nid: &label_to_nid,
            seen_call_pairs: &mut seen_call_pairs,
            seen_dyn_import_pairs: &mut seen_dyn_import_pairs,
            edges: &mut edges,
            raw_calls: &mut raw_calls,
        };
        for (caller_nid, body_node) in &function_bodies {
            walk_calls(&mut call_ctx, *body_node, caller_nid, &source);
        }
    }

    // ── Clean edges ───────────────────────────────────────────────────────────
    let clean_edges: Vec<Edge> = edges
        .into_iter()
        .filter(|e| {
            seen_ids.contains(&e.source)
                && (seen_ids.contains(&e.target)
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
