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
mod graph;
mod inherit;
mod js_extra;
mod names;
pub(crate) mod references;
mod ruby;
pub(crate) mod walk;

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
    extract_generic_with_source(path, config, &source)
}

/// [`extract_generic`] parsing `source` instead of reading `path`, while still
/// keying nodes/edges off `path`. Lets container formats (e.g. Vue SFCs) mask the
/// wrapper and parse just the embedded `<script>`. Mirrors Python
/// `_extract_generic(..., source_override=...)`.
#[allow(clippy::too_many_lines)] // single-pass tree-sitter walker — splitting hurts flow
pub(crate) fn extract_generic_with_source(
    path: &Path,
    config: &LangConfig,
    source: &[u8],
) -> FileResult {
    let mut parser = Parser::new();
    if let Err(e) = parser.set_language(&config.language) {
        return FileResult::error(format!(
            "parser language mismatch for {}: {e}",
            path.display()
        ));
    }

    let Some(tree) = parser.parse(source, None) else {
        return FileResult::error(format!("tree-sitter parse failed for {}", path.display()));
    };

    let root = tree.root_node();
    let stem = file_stem(path);
    let str_path = path.to_string_lossy().into_owned();

    let mut nodes: Vec<GNode> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut function_bodies: Vec<(String, tree_sitter::Node<'_>)> = Vec::new();
    let mut pending_listen_edges: Vec<(String, String, u32)> = Vec::new();

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
    // Pre-scan C# files for declared interface names so the inheritance pass can
    // split `inherits` from `implements`. Empty for every other language.
    let csharp_interface_names: HashSet<String> = if config.lang_id == config::LangId::CSharp {
        inherit::csharp_pre_scan_interfaces(root, source)
    } else {
        HashSet::new()
    };

    // Pre-scan Swift files so the inheritance emitter can split `inherits`
    // (base class) from `implements` (protocol conformance). Empty otherwise.
    let (swift_protocol_names, swift_class_names): (HashSet<String>, HashSet<String>) =
        if config.lang_id == config::LangId::Swift {
            inherit::swift_pre_scan(root, source)
        } else {
            (HashSet::new(), HashSet::new())
        };

    let mut cur = root.walk();
    if cur.goto_first_child() {
        let mut walk_ctx = walk::WalkCtx {
            config,
            file_nid: &file_nid,
            stem: &stem,
            str_path: &str_path,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            function_bodies: &mut function_bodies,
            csharp_interface_names: &csharp_interface_names,
            swift_protocol_names: &swift_protocol_names,
            swift_class_names: &swift_class_names,
            pending_listen_edges: &mut pending_listen_edges,
        };
        loop {
            let child = cur.node();
            walk(&mut walk_ctx, child, None, source);
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
    let mut seen_ref_pairs: HashSet<(String, String, String)> = HashSet::new();

    // Ruby: per-method `var -> ClassName` table from `var = Const.new` bindings,
    // populated before walk_calls so member-call raw_calls carry a `receiver_type`
    // for type-based cross-file resolution (#1499). Empty for non-Ruby files.
    let ruby_var_types: HashMap<String, HashMap<String, Option<String>>> =
        if config.lang_id == LangId::Ruby {
            function_bodies
                .iter()
                .map(|(nid, body)| (nid.clone(), ruby::ruby_local_class_bindings(*body, source)))
                .collect()
        } else {
            HashMap::new()
        };

    {
        let mut call_ctx = super::generic::calls::CallWalkCtx {
            config,
            str_path: &str_path,
            label_to_nid: &label_to_nid,
            seen_call_pairs: &mut seen_call_pairs,
            seen_dyn_import_pairs: &mut seen_dyn_import_pairs,
            edges: &mut edges,
            raw_calls: &mut raw_calls,
            seen_ref_pairs: &mut seen_ref_pairs,
            ruby_var_types: &ruby_var_types,
        };
        for (caller_nid, body_node) in &function_bodies {
            walk_calls(&mut call_ctx, *body_node, caller_nid, source);
        }
    }

    // ── PHP event-listener pass ────────────────────────────────────────────────
    // Resolve deferred ($event, $listener) pairs into `listened_by` edges now
    // that every node (and thus `label_to_nid`) exists. Mirrors graphify-py.
    {
        let mut seen_listen_pairs: HashSet<(String, String)> = HashSet::new();
        for (event_name, listener_name, line) in &pending_listen_edges {
            let (Some(event_nid), Some(listener_nid)) = (
                label_to_nid.get(&event_name.to_lowercase()),
                label_to_nid.get(&listener_name.to_lowercase()),
            ) else {
                continue;
            };
            if event_nid == listener_nid
                || !seen_listen_pairs.insert((event_nid.clone(), listener_nid.clone()))
            {
                continue;
            }
            edges.push(Edge {
                external: false,
                source: event_nid.clone(),
                target: listener_nid.clone(),
                relation: "listened_by".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: None,
                confidence_score: Some(1.0),
            });
        }
    }

    // Fold any forward-reference placeholder into its same-file declaration
    // before cleaning, so a type used before it is declared resolves to the
    // real node instead of an orphaned bare-name duplicate.
    crate::forward_refs::reconcile_forward_refs(&mut nodes, &mut edges);

    // ── Clean edges ───────────────────────────────────────────────────────────
    // Cross-module edge relations (`imports`, `imports_from`, `re_exports`)
    // legitimately point at nodes that don't live in this file. Everything
    // else must have both endpoints among the reconciled node ids. Rebuild the
    // valid-id set from the surviving nodes rather than the now-stale
    // `seen_ids`, which still lists any placeholder ids reconcile folded away.
    let valid_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();
    let clean_edges: Vec<Edge> = edges
        .into_iter()
        .filter(|e| {
            valid_ids.contains(&e.source)
                && (valid_ids.contains(&e.target)
                    || matches!(
                        e.relation.as_str(),
                        "imports" | "imports_from" | "re_exports"
                    ))
        })
        .collect();

    FileResult {
        nodes,
        edges: clean_edges,
        raw_calls,
        error: None,
    }
}
