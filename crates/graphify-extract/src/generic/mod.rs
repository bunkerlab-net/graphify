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
mod cpp;
mod csharp;
mod graph;
mod indirect;
mod inherit;
mod java;
mod js_extra;
mod names;
pub(crate) mod references;
mod ruby;
mod ts;
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
    let mut callable_def_nids: HashSet<String> = HashSet::new();
    let mut local_bound_names: HashMap<String, HashSet<String>> = HashMap::new();

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

    // C# namespace/using scope stacks, threaded through the walk so C# type nodes
    // fold the enclosing namespace into their id and carry `namespace`/`scope_chain`
    // metadata (#1562). Empty for every other language.
    let mut csharp_ns_stack: Vec<String> = Vec::new();
    let mut csharp_scope_stack: Vec<String> = Vec::new();

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
            csharp_ns_stack: &mut csharp_ns_stack,
            csharp_scope_stack: &mut csharp_scope_stack,
            callable_def_nids: &mut callable_def_nids,
            local_bound_names: &mut local_bound_names,
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

    // Case-sensitive `label -> nid` map + `nid -> source_file` for the indirect
    // capture: an `indirect_call` reference binds by EXACT name (never the
    // lowercased call map), preserving case-sensitivity hardening (#1581), and the
    // source-file map tells a same-named local non-callable (reject) from an
    // import-surfaced foreign symbol (defer to the cross-file resolver).
    let mut label_to_nid_exact: HashMap<String, String> = HashMap::new();
    let mut nid_to_sf: HashMap<String, String> = HashMap::new();
    for n in &nodes {
        nid_to_sf.insert(n.id.clone(), n.source_file.clone());
        if n.node_type.as_deref() == Some("namespace") {
            continue;
        }
        let key = n
            .label
            .trim_end_matches("()")
            .trim_start_matches('.')
            .to_string();
        label_to_nid_exact.insert(key, n.id.clone());
    }
    let mut seen_indirect_pairs: HashSet<(String, String)> = HashSet::new();

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

    // Java: per-*body* `receiver -> type` table (fields / params / locals, with
    // `this.field`), built before walk_calls so member-call raw_calls carry a
    // `receiver_type` for type-based cross-file resolution (#1696). Keyed per body
    // (aligned with `function_bodies`), not per method NID, so overloaded methods
    // — one NID, distinct bodies — each resolve against their own scope. Empty for
    // non-Java files.
    let java_body_types: Vec<HashMap<String, String>> = if config.lang_id == LangId::Java {
        function_bodies
            .iter()
            .map(|(_, body)| java::build_java_receiver_types_for_body(*body, source))
            .collect()
    } else {
        Vec::new()
    };
    let empty_java: HashMap<String, String> = HashMap::new();
    // C#: a file-wide `name -> Type` table (field / property / param / local),
    // built from the whole tree so class-level members are in scope for every
    // method. Constant across bodies (unlike Java's per-body table). Empty for
    // non-C# files (#1609).
    let csharp_var_types: HashMap<String, String> = if config.lang_id == LangId::CSharp {
        csharp::build_csharp_type_table(root, source)
    } else {
        HashMap::new()
    };
    // C++: a file-wide `var -> ClassName` table from local declarations in every
    // function body, so member-call raw_calls carry a `receiver_type` for
    // type-based cross-file resolution (#1547). First-binding-wins across bodies
    // (a later `Foo f;` doesn't clobber an earlier binding). Empty for non-C++.
    let cpp_var_types: HashMap<String, String> = if config.lang_id == LangId::Cpp {
        let mut table = HashMap::new();
        for (_, body) in &function_bodies {
            cpp::collect_cpp_local_var_types(*body, source, &mut table);
        }
        table
    } else {
        HashMap::new()
    };
    // TS/JS: a file-wide `name -> TypeName` table (constructor-injected
    // `this.field` types, local `new` bindings, typed params), so member-call
    // raw_calls carry a `receiver_type` for cross-file resolution (#1316/#1630).
    // Empty for non-TS/JS files.
    let ts_var_types: HashMap<String, String> = if matches!(
        config.lang_id,
        LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX
    ) {
        ts::build_ts_type_table(root, source)
    } else {
        HashMap::new()
    };
    // JS/TS: ids of the function bodies walked with their own caller, so the
    // closure-descent in `walk_calls` doesn't double-walk a tracked arrow (#1630).
    let tracked_body_ids: std::collections::HashSet<usize> =
        function_bodies.iter().map(|(_, b)| b.id()).collect();

    for (i, (caller_nid, body_node)) in function_bodies.iter().enumerate() {
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
            java_var_types: java_body_types.get(i).unwrap_or(&empty_java),
            csharp_var_types: &csharp_var_types,
            cpp_var_types: &cpp_var_types,
            ts_var_types: &ts_var_types,
            tracked_body_ids: &tracked_body_ids,
            label_to_nid_exact: &label_to_nid_exact,
            nid_to_sf: &nid_to_sf,
            callable_def_nids: &callable_def_nids,
            local_bound_names: &local_bound_names,
            seen_indirect_pairs: &mut seen_indirect_pairs,
        };
        walk_calls(&mut call_ctx, *body_node, caller_nid, source);
    }

    // ── Module-level indirect dispatch (#1566) ─────────────────────────────────
    // A function listed in a TOP-LEVEL dispatch table / bound to a module alias /
    // named by a reflective `getattr` is an indirect dependency of the file node.
    // Function/class bodies are walked above, so this scan stops at their
    // boundaries. Python + JS/TS only.
    if matches!(
        config.lang_id,
        LangId::Python | LangId::JavaScript | LangId::TypeScript | LangId::TypeScriptX
    ) {
        let mut ind = indirect::IndirectState {
            str_path: &str_path,
            label_to_nid_exact: &label_to_nid_exact,
            nid_to_sf: &nid_to_sf,
            callable_def_nids: &callable_def_nids,
            edges: &mut edges,
            raw_calls: &mut raw_calls,
            seen_call_pairs: &seen_call_pairs,
            seen_indirect_pairs: &mut seen_indirect_pairs,
        };
        if config.lang_id == LangId::Python {
            let module_bound = indirect::python_module_bound_names(root, source);
            indirect::scan_module_dispatch(&mut ind, root, &file_nid, &module_bound, source);
        } else {
            let module_bound = indirect::js_module_bound_names(root, source);
            indirect::scan_js_module_dispatch(&mut ind, root, &file_nid, &module_bound, source);
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
                deferred: false,
                metadata: None,
            });
        }
    }

    // Fold any forward-reference placeholder into its same-file declaration
    // before cleaning, so a type used before it is declared resolves to the
    // real node instead of an orphaned bare-name duplicate.
    crate::forward_refs::reconcile_forward_refs(&mut nodes, &mut edges);

    // Stamp the durable `_callable` marker on every function / method / class def
    // node so the cross-file `indirect_call` resolver can gate a callback-by-name
    // against same-named data symbols AFTER id-remap. Rides the AST cache via node
    // metadata; stripped before graph.json (mirrors graphify-py's `_callable`).
    if !callable_def_nids.is_empty() {
        for n in &mut nodes {
            if callable_def_nids.contains(&n.id) {
                n.metadata
                    .get_or_insert_with(indexmap::IndexMap::new)
                    .insert("_callable".to_string(), serde_json::Value::Bool(true));
            }
        }
    }

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
