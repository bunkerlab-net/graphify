//! JavaScript/TypeScript extra-walk logic.
//!
//! Handles constructs that the generic structural walk cannot see through the
//! standard `function_types` / `class_types` tables:
//! - Arrow-function `const f = () => {}` declarations.
//! - `CommonJS` `require()` imports (both bare and destructured).
//! - Dynamic `import()` call edges.
//! - `resolve_js_import_target` — the public helper for resolving a raw
//!   import specifier to a canonical NID (used by the JS import-handler crate).

// Tree-sitter row numbers are source line indices; files with 2^32+ lines
// do not exist in practice, so usize→u32 truncation is safe.
#![allow(clippy::cast_possible_truncation)]

use std::collections::HashSet;

use tree_sitter::Node;

use crate::ids::{file_stem, make_id, make_id1};
use crate::tsconfig::load_tsconfig_aliases;
use crate::types::Edge;

use super::names::{read_text, read_text_owned};
use super::walk::{add_edge, add_node, first_child_kind};

// ── JS/TS assignment-form helpers (#09da529) ──────────────────────────────────

/// `true` if a node kind is a callable value, for the JS/TS assignment /
/// class-field / function-expression forms. Older tree-sitter-javascript
/// grammars label a function expression `function`; current ones use
/// `function_expression`. Mirrors Python `_JS_FUNCTION_VALUE_TYPES`.
#[must_use]
pub(super) fn is_js_function_value(kind: &str) -> bool {
    matches!(kind, "arrow_function" | "function_expression" | "function")
}

/// Symbol an `assignment_expression` LHS defines when its RHS is a function.
/// Mirrors Python `_js_member_assignment_target`.
pub(super) enum JsAssignTarget {
    /// `this.foo = fn` — owner is the enclosing function/class.
    This(String),
    /// `exports.foo = fn` / `module.exports.foo = fn` — file-contained function.
    Exports(String),
    /// `Foo.prototype.bar = fn` — method `member` owned by `owner`.
    Prototype { owner: String, member: String },
}

/// Classify the symbol an `assignment_expression` LHS defines when its RHS is a
/// function. Returns `None` for an arbitrary `obj.x = fn`, which is skipped —
/// capturing those would reintroduce the bare-named / phantom-god-node class of
/// bug the module-level scope guard (#1077) prevents.
pub(super) fn js_member_assignment_target(left: Node<'_>, source: &[u8]) -> Option<JsAssignTarget> {
    if left.kind() != "member_expression" {
        return None;
    }
    let member_name = read_text(left.child_by_field_name("property")?, source);
    if member_name.is_empty() {
        return None;
    }
    let obj = left.child_by_field_name("object")?;
    match obj.kind() {
        "this" => Some(JsAssignTarget::This(member_name.to_string())),
        "identifier" if read_text(obj, source) == "exports" => {
            Some(JsAssignTarget::Exports(member_name.to_string()))
        }
        "member_expression" => {
            // module.exports.X  or  Foo.prototype.X
            let inner_obj = obj.child_by_field_name("object")?;
            let inner_prop_name = read_text(obj.child_by_field_name("property")?, source);
            if inner_obj.kind() != "identifier" {
                return None;
            }
            let inner_obj_name = read_text(inner_obj, source);
            if inner_obj_name == "module" && inner_prop_name == "exports" {
                Some(JsAssignTarget::Exports(member_name.to_string()))
            } else if inner_prop_name == "prototype" {
                Some(JsAssignTarget::Prototype {
                    owner: inner_obj_name.to_string(),
                    member: member_name.to_string(),
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

// ── JS/TS extra walk ──────────────────────────────────────────────────────────

/// Handle JS/TS nodes the generic structural walk misses.
///
/// Covers arrow / function-expression declarations (`const f = () => {}`,
/// `const f = function(){}`), `CommonJS` `require()` imports, module-level
/// `const` value nodes, `CommonJS` / prototype member assignments
/// (`exports.X = fn`, `module.exports.X = fn`, `Foo.prototype.bar = fn`), and
/// class fields whose
/// value is a function (`class C { handler = () => {} }`). Returns `true` when a
/// construct was handled, so the caller skips generic processing. Mirrors Python
/// `_js_extra_walk`.
#[allow(clippy::too_many_lines)] // linear node-kind dispatch mirroring Python's _js_extra_walk
pub(super) fn js_extra_walk<'tree>(
    ctx: &mut super::walk::WalkCtx<'_, 'tree>,
    node: Node<'tree>,
    source: &[u8],
    parent_class_nid: Option<&str>,
) -> bool {
    let file_nid = ctx.file_nid;
    let stem = ctx.stem;
    let str_path = ctx.str_path;
    let nodes = &mut *ctx.nodes;
    let edges = &mut *ctx.edges;
    let seen_ids = &mut *ctx.seen_ids;
    let function_bodies = &mut *ctx.function_bodies;
    let t = node.kind();

    // CommonJS / prototype member assignments whose value is a function:
    //   exports.X = () => {}      → file-contained function  X()
    //   module.exports.X = fn     → file-contained function  X()
    //   Foo.prototype.bar = fn    → method bar() owned by Foo
    // (`this.X = fn` lives inside a function body, captured at the function.)
    if t == "expression_statement"
        && let Some(assign) = first_child_kind(node, "assignment_expression")
        && let Some(value) = assign.child_by_field_name("right")
        && is_js_function_value(value.kind())
        && let Some(left) = assign.child_by_field_name("left")
        && let Some(target) = js_member_assignment_target(left, source)
    {
        let line = node.start_position().row as u32 + 1;
        let nid = match target {
            JsAssignTarget::Exports(member) => {
                let nid = make_id(&[stem, member.as_str()]);
                add_node(
                    &nid,
                    &format!("{member}()"),
                    line,
                    str_path,
                    nodes,
                    seen_ids,
                );
                add_edge(file_nid, &nid, "contains", line, str_path, None, edges);
                Some(nid)
            }
            JsAssignTarget::Prototype { owner, member } => {
                let owner_nid = make_id(&[stem, owner.as_str()]);
                let nid = make_id(&[owner_nid.as_str(), member.as_str()]);
                add_node(
                    &nid,
                    &format!(".{member}()"),
                    line,
                    str_path,
                    nodes,
                    seen_ids,
                );
                add_edge(&owner_nid, &nid, "method", line, str_path, None, edges);
                Some(nid)
            }
            JsAssignTarget::This(_) => None,
        };
        if let Some(nid) = nid {
            if let Some(body) = value.child_by_field_name("body") {
                function_bodies.push((nid, body));
            }
            return true;
        }
    }

    // Class fields whose value is a function:
    //   class C { handler = () => {} }   → method handler() owned by C
    if let Some(parent_nid) = parent_class_nid
        && matches!(t, "field_definition" | "public_field_definition")
        && let Some(prop) = node
            .child_by_field_name("property")
            .or_else(|| node.child_by_field_name("name"))
        && let Some(value) = node.child_by_field_name("value")
        && is_js_function_value(value.kind())
    {
        let field_name = read_text(prop, source);
        if !field_name.is_empty() {
            let line = node.start_position().row as u32 + 1;
            let nid = make_id(&[parent_nid, field_name]);
            add_node(
                &nid,
                &format!(".{field_name}()"),
                line,
                str_path,
                nodes,
                seen_ids,
            );
            add_edge(parent_nid, &nid, "method", line, str_path, None, edges);
            if let Some(body) = value.child_by_field_name("body") {
                function_bodies.push((nid, body));
            }
            return true;
        }
    }

    if t != "lexical_declaration" && t != "variable_declaration" {
        return false;
    }

    // CJS require imports — emit edges regardless of scope.
    let require_found = require_imports_js(node, source, file_nid, str_path, stem, edges);

    // Scope guard (#1077): only emit nodes for module-level declarations.
    // Without this, a `const x = …` inside an arrow callback (e.g. inside
    // `describe(() => { const set = new Set(…) })`) emits a bare-named node,
    // and the same name collides across unrelated files producing phantom
    // god-nodes. Arrow-function bodies are walked separately via
    // `function_bodies`, so locals never need a node here.
    let is_module_level = node.parent().is_some_and(|parent| {
        parent.kind() == "program"
            || (parent.kind() == "export_statement"
                && parent.parent().is_some_and(|pp| pp.kind() == "program"))
    });

    let mut arrow_found = false;
    let mut const_found = false;

    if t == "lexical_declaration" && is_module_level {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.kind() == "variable_declarator"
                    && let Some(value) = child.child_by_field_name("value")
                {
                    if is_js_function_value(value.kind()) {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            let func_name = read_text_owned(name_node, source);
                            let line = child.start_position().row as u32 + 1;
                            let func_nid = make_id(&[stem, &func_name]);
                            add_node(
                                &func_nid,
                                &format!("{func_name}()"),
                                line,
                                str_path,
                                nodes,
                                seen_ids,
                            );
                            add_edge(file_nid, &func_nid, "contains", line, str_path, None, edges);
                            if let Some(body) = value.child_by_field_name("body") {
                                function_bodies.push((func_nid, body));
                            }
                            arrow_found = true;
                        }
                    } else if matches!(
                        value.kind(),
                        "object" | "array" | "as_expression" | "call_expression" | "new_expression"
                    ) && let Some(name_node) = child.child_by_field_name("name")
                    {
                        let const_name = read_text_owned(name_node, source);
                        let line = child.start_position().row as u32 + 1;
                        let const_nid = make_id(&[stem, &const_name]);
                        add_node(&const_nid, &const_name, line, str_path, nodes, seen_ids);
                        add_edge(
                            file_nid, &const_nid, "contains", line, str_path, None, edges,
                        );
                        const_found = true;
                    }
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    arrow_found || const_found || require_found
}

// ── CJS require helpers ───────────────────────────────────────────────────────

/// Locate the innermost `call_expression` that is a `require()` call, if any.
///
/// Recurses into `member_expression` chains (e.g. `require('./m').default`) to
/// find the base `require` call. Returns `None` if the value is not a require call.
fn find_require_call(value_node: Option<Node<'_>>) -> Option<Node<'_>> {
    let node = value_node?;
    if node.kind() == "call_expression" {
        let fn_node = node.child_by_field_name("function")?;
        if fn_node.kind() == "identifier" {
            return Some(node);
        }
    }
    if node.kind() == "member_expression" {
        let obj = node.child_by_field_name("object")?;
        return find_require_call(Some(obj));
    }
    None
}

/// Scan a JS/TS `lexical_declaration` or `variable_declaration` for `require()` calls.
///
/// Emits `imports_from` edges for every `require('...')` found, plus symbol-level `imports`
/// edges when the binding uses destructuring (`const { a, b } = require('...')`) or
/// member access (`const x = require('./m').y`). Mirrors Python `_require_imports_js`.
/// Returns `true` if at least one require import was processed.
#[allow(clippy::too_many_lines)]
fn require_imports_js(
    node: Node<'_>,
    source: &[u8],
    file_nid: &str,
    str_path: &str,
    _stem: &str,
    edges: &mut Vec<Edge>,
) -> bool {
    if node.kind() != "lexical_declaration" && node.kind() != "variable_declaration" {
        return false;
    }
    let mut found = false;
    let mut cur = node.walk();
    if !cur.goto_first_child() {
        return false;
    }
    loop {
        let child = cur.node();
        if child.kind() == "variable_declarator" {
            let value = child.child_by_field_name("value");
            let Some(call) = find_require_call(value) else {
                if !cur.goto_next_sibling() {
                    break;
                }
                continue;
            };
            let Some(fn_node) = call.child_by_field_name("function") else {
                if !cur.goto_next_sibling() {
                    break;
                }
                continue;
            };
            if read_text(fn_node, source) != "require" {
                if !cur.goto_next_sibling() {
                    break;
                }
                continue;
            }
            let Some(args) = call.child_by_field_name("arguments") else {
                if !cur.goto_next_sibling() {
                    break;
                }
                continue;
            };
            let mut raw: Option<String> = None;
            let mut acur = args.walk();
            if acur.goto_first_child() {
                loop {
                    let arg = acur.node();
                    if arg.kind() == "string" {
                        raw = Some(
                            read_text_owned(arg, source)
                                .trim_matches(|c| c == '\'' || c == '"' || c == '`' || c == ' ')
                                .to_string(),
                        );
                        break;
                    }
                    if !acur.goto_next_sibling() {
                        break;
                    }
                }
            }
            let Some(raw) = raw else {
                if !cur.goto_next_sibling() {
                    break;
                }
                continue;
            };
            let (tgt_nid, resolved_path) = resolve_js_import_target(&raw, str_path);
            let line = node.start_position().row as u32 + 1;
            edges.push(Edge {
                external: false,
                source: file_nid.to_string(),
                target: tgt_nid.clone(),
                relation: "imports_from".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: Some("import".to_string()),
                confidence_score: None,
            });
            found = true;

            // Symbol-level edges
            if let Some(ref rp) = resolved_path {
                let target_stem = file_stem(rp);
                let name_node = child.child_by_field_name("name");
                let mut sym_names: Vec<String> = Vec::new();
                if let Some(nn) = name_node
                    && nn.kind() == "object_pattern"
                {
                    let mut pcur = nn.walk();
                    if pcur.goto_first_child() {
                        loop {
                            let prop = pcur.node();
                            if prop.kind() == "shorthand_property_identifier_pattern" {
                                sym_names.push(read_text_owned(prop, source));
                            } else if prop.kind() == "pair_pattern"
                                && let Some(key) = prop.child_by_field_name("key")
                            {
                                sym_names.push(read_text_owned(key, source));
                            }
                            if !pcur.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                // member access: const x = require('./m').y
                if let Some(v) = value
                    && v.kind() == "member_expression"
                    && let Some(prop) = v.child_by_field_name("property")
                {
                    sym_names.push(read_text_owned(prop, source));
                }
                for sym in &sym_names {
                    edges.push(Edge {
                        external: false,
                        source: file_nid.to_string(),
                        target: make_id(&[&target_stem, sym]),
                        relation: "imports".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: Some("import".to_string()),
                        confidence_score: None,
                    });
                }
            }
        }
        if !cur.goto_next_sibling() {
            break;
        }
    }
    found
}

// ── JS import target resolution ───────────────────────────────────────────────

/// Mirrors Python `_resolve_js_import_target`.
/// Returns `(target_nid, Option<resolved_path>)`.
#[must_use]
pub fn resolve_js_import_target(raw: &str, str_path: &str) -> (String, Option<std::path::PathBuf>) {
    if raw.is_empty() {
        return (String::new(), None);
    }
    if raw.starts_with('.') {
        let parent = std::path::Path::new(str_path)
            .parent()
            .unwrap_or(std::path::Path::new("."));
        let joined = parent.join(raw);
        let resolved_raw = std::path::PathBuf::from(normalize_path(&joined));
        let resolved = crate::tsconfig::resolve_js_module_path(&resolved_raw);
        return (make_id1(&resolved.to_string_lossy()), Some(resolved));
    }
    let start_dir = std::path::Path::new(str_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let aliases = load_tsconfig_aliases(start_dir);
    for (alias_prefix, alias_base) in &aliases {
        if raw == alias_prefix || raw.starts_with(&format!("{alias_prefix}/")) {
            let rest = raw[alias_prefix.len()..].trim_start_matches('/');
            let joined = std::path::Path::new(alias_base).join(rest);
            let resolved_raw = std::path::PathBuf::from(normalize_path(&joined));
            let resolved = crate::tsconfig::resolve_js_module_path(&resolved_raw);
            return (make_id1(&resolved.to_string_lossy()), Some(resolved));
        }
    }
    // Try resolving against a pnpm workspace before falling back to the
    // bare-module hash. Inside a monorepo, `@scope/pkg` should resolve to
    // the package's entry-point file so cross-package edges target real
    // nodes instead of a synthetic ID.
    if let Some(resolved) = crate::workspace::resolve_workspace_import(raw, start_dir) {
        return (make_id1(&resolved.to_string_lossy()), Some(resolved));
    }
    let module_name = raw.split('/').next_back().unwrap_or(raw);
    if module_name.is_empty() {
        return (String::new(), None);
    }
    (make_id1(module_name), None)
}

/// Collapse `.` and `..` path components without requiring the path to exist on disk.
///
/// Used when resolving relative imports so that `./foo/../bar` becomes `./bar`. Mirrors
/// the normalization step in Python `_resolve_js_import_target`.
fn normalize_path(path: &std::path::Path) -> String {
    let mut components: Vec<&std::ffi::OsStr> = Vec::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => {
                // Pop a real component if one exists; otherwise preserve the
                // leading `..` so callers like `../../foo` don't collapse to
                // `../foo`.
                if components
                    .last()
                    .is_some_and(|c| *c != std::ffi::OsStr::new(".."))
                {
                    components.pop();
                } else {
                    components.push(std::ffi::OsStr::new(".."));
                }
            }
            std::path::Component::CurDir => {}
            other => components.push(other.as_os_str()),
        }
    }
    std::path::PathBuf::from_iter(components)
        .to_string_lossy()
        .into_owned()
}

// ── Dynamic import() ─────────────────────────────────────────────────────────

/// Detect and emit an edge for a dynamic `import('...')` call expression.
///
/// Called from `walk_calls` before the generic callee extraction path. Returns `true` when
/// the node is an `import(...)` expression (whether or not a string literal was found), so
/// the caller can skip generic callee handling. Dynamic template literals with substitutions
/// are silently ignored because the target cannot be statically determined.
/// Mirrors Python `_dynamic_import_js`.
pub(super) fn dynamic_import_js(
    node: Node<'_>,
    source: &[u8],
    caller_nid: &str,
    str_path: &str,
    edges: &mut Vec<Edge>,
    seen_dyn_pairs: &mut HashSet<(String, String)>,
) -> bool {
    let func_node = node.child_by_field_name("function").or_else(|| {
        let first = node.child(0)?;
        if read_text(first, source) == "import" {
            Some(first)
        } else {
            None
        }
    });
    let Some(func_node) = func_node else {
        return false;
    };
    if read_text(func_node, source) != "import" {
        return false;
    }
    let Some(args) = node.child_by_field_name("arguments") else {
        return true;
    };
    let mut cur = args.walk();
    if !cur.goto_first_child() {
        return true;
    }
    loop {
        let arg = cur.node();
        let raw: Option<String> = if arg.kind() == "template_string" {
            // Skip dynamic template literals with substitutions. tree-sitter 0.26
            // requires `Node::child` to be called with `u32`; cast the bound.
            let count = u32::try_from(arg.child_count()).unwrap_or(0);
            let has_sub = (0..count).any(|i| {
                arg.child(i)
                    .is_some_and(|c| c.kind() == "template_substitution")
            });
            if has_sub {
                None
            } else {
                Some(read_text_owned(arg, source).trim_matches('`').to_string())
            }
        } else if arg.kind() == "string" {
            Some(
                read_text_owned(arg, source)
                    .trim_matches(|c| c == '\'' || c == '"' || c == ' ')
                    .to_string(),
            )
        } else {
            if !cur.goto_next_sibling() {
                break;
            }
            continue;
        };

        let Some(raw) = raw else { break };
        if raw.is_empty() {
            break;
        }

        let (tgt_nid, _) = resolve_js_import_target(&raw, str_path);
        let pair = (caller_nid.to_string(), tgt_nid.clone());
        if seen_dyn_pairs.insert(pair) {
            let line = node.start_position().row as u32 + 1;
            edges.push(Edge {
                external: false,
                source: caller_nid.to_string(),
                target: tgt_nid,
                relation: "imports_from".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
        }
        break;
    }
    true
}
