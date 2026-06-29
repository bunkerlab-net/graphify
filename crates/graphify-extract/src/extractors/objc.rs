//! Objective-C extractor — custom walk over tree-sitter-objc AST.

use std::collections::HashSet;
use std::path::Path;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Collect every `type_identifier` name under a property's type node, descending
/// through `generic_specifier`/`type_name` so `NSArray<Product *>` yields both
/// `NSArray` and the element type `Product` (#1475).
fn collect_objc_type_names<'a>(
    node: tree_sitter::Node<'_>,
    source: &'a [u8],
    out: &mut Vec<&'a str>,
) {
    if node.kind() == "type_identifier" {
        out.push(read_text(node, source));
        return;
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            collect_objc_type_names(cur.node(), source, out);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}

/// Extract interfaces, implementations, protocols, methods, and imports from `.m`/`.mm`/`.h` files.
#[must_use]
pub fn extract_objc(path: &Path) -> FileResult {
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
        .set_language(&tree_sitter_objc::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set objc language".to_string()),
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
    let mut method_bodies: Vec<(String, usize, usize)> = Vec::new();

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
    });

    let root = tree.root_node();
    {
        let mut walk_ctx = ObjcWalkCtx {
            str_path: &str_path,
            stem: &stem,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            method_bodies: &mut method_bodies,
        };
        walk_objc(&mut walk_ctx, root, &source, None);
    }

    // Second pass: calls inside method bodies
    let all_method_nids: HashSet<String> = nodes
        .iter()
        .filter(|n| n.id != file_nid)
        .map(|n| n.id.clone())
        .collect();
    let mut seen_calls: HashSet<(String, String)> = HashSet::new();
    {
        let mut call_ctx = ObjcCallCtx {
            str_path: &str_path,
            all_method_nids: &all_method_nids,
            edges: &mut edges,
            seen_calls: &mut seen_calls,
        };
        for (caller_nid, body_start, body_end) in &method_bodies {
            walk_calls_objc(
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
    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}

/// Recursively walk an `ObjC` AST emitting nodes for interfaces, implementations, and methods.
///
/// Handles `@interface`, `@implementation`, `@protocol`, instance/class method declarations
/// and definitions, and `#import` / `@import` directives. Mirrors Python `_walk_objc`.
/// Shared state threaded through every [`walk_objc`] recursion.
struct ObjcWalkCtx<'a> {
    str_path: &'a str,
    stem: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    method_bodies: &'a mut Vec<(String, usize, usize)>,
}

impl ObjcWalkCtx<'_> {
    /// Return the NID for a named type, creating a SOURCELESS placeholder stub
    /// when no file-qualified node exists. Mirrors Python objc `ensure_named_node`
    /// (extract.py): the stub carries no `source_file` so a real cross-file
    /// definition can be rewired onto it (#1402, the phantom-duplicate fix);
    /// the referencing file is recorded as `origin_file` to disambiguate
    /// same-label stubs (#1462), matching the generic `ensure_named_node`.
    fn ensure_named_node(&mut self, name: &str) -> String {
        let nid1 = make_id(&[self.stem, name]);
        if self.seen_ids.contains(&nid1) {
            return nid1;
        }
        let nid2 = make_id1(name);
        if self.seen_ids.insert(nid2.clone()) {
            self.nodes.push(Node {
                id: nid2.clone(),
                label: name.to_string(),
                file_type: "code".to_string(),
                source_file: String::new(),
                source_location: Some(String::new()),
                metadata: None,
                origin_file: Some(self.str_path.to_string()),
            });
        }
        nid2
    }
}

#[allow(clippy::too_many_lines)] // linear dispatch over Objective-C's AST node kinds
fn walk_objc(
    ctx: &mut ObjcWalkCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    parent_nid: Option<&str>,
) {
    let t = node.kind();
    let line = node.start_position().row + 1;

    match t {
        "preproc_include" => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.kind() == "system_lib_string" {
                        let raw = read_text(child, source).trim_matches(|c| c == '<' || c == '>');
                        let module = raw.split('/').next_back().unwrap_or("").replace(".h", "");
                        if !module.is_empty() {
                            let tgt_nid = make_id1(&module);
                            ctx.edges.push(Edge {
                                external: false,
                                source: ctx.file_nid.to_string(),
                                target: tgt_nid,
                                relation: "imports".to_string(),
                                confidence: "EXTRACTED".to_string(),
                                source_file: ctx.str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                weight: 1.0,
                                context: Some("import".to_string()),
                                confidence_score: None,
                            });
                        }
                    } else if child.kind() == "string_literal" {
                        let mut sc = child.walk();
                        if sc.goto_first_child() {
                            loop {
                                if sc.node().kind() == "string_content" {
                                    let raw = read_text(sc.node(), source);
                                    let module =
                                        raw.split('/').next_back().unwrap_or("").replace(".h", "");
                                    if !module.is_empty() {
                                        let tgt_nid = make_id1(&module);
                                        ctx.edges.push(Edge {
                                            external: false,
                                            source: ctx.file_nid.to_string(),
                                            target: tgt_nid,
                                            relation: "imports".to_string(),
                                            confidence: "EXTRACTED".to_string(),
                                            source_file: ctx.str_path.to_string(),
                                            source_location: Some(format!("L{line}")),
                                            weight: 1.0,
                                            context: Some("import".to_string()),
                                            confidence_score: None,
                                        });
                                    }
                                }
                                if !sc.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "module_import" => {
            // @import Foundation;  /  @import Foundation.NSString;
            if let Some(path_node) = node.child_by_field_name("path") {
                let module = read_text(path_node, source)
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !module.is_empty() {
                    ctx.edges.push(Edge {
                        external: false,
                        source: ctx.file_nid.to_string(),
                        target: make_id1(module),
                        relation: "imports".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: Some("import".to_string()),
                        confidence_score: None,
                    });
                }
            }
        }
        "class_interface" => {
            let identifiers: Vec<tree_sitter::Node<'_>> = {
                let mut ids = vec![];
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "identifier" {
                            ids.push(cur.node());
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
                ids
            };
            if identifiers.is_empty() {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        walk_objc(ctx, cur.node(), source, parent_nid);
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
                return;
            }
            let name = read_text(identifiers[0], source);
            let cls_nid = make_id(&[ctx.stem, name]);
            if ctx.seen_ids.insert(cls_nid.clone()) {
                ctx.nodes.push(Node {
                    id: cls_nid.clone(),
                    label: name.to_string(),
                    file_type: "code".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    metadata: None,
                    origin_file: None,
                });
            }
            ctx.edges.push(Edge {
                external: false,
                source: ctx.file_nid.to_string(),
                target: cls_nid.clone(),
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: ctx.str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
            // Superclass and protocol adoption
            let mut colon_seen = false;
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let child = cur.node();
                    if child.kind() == ":" {
                        colon_seen = true;
                    } else if colon_seen && child.kind() == "identifier" {
                        let super_nid = ctx.ensure_named_node(read_text(child, source));
                        ctx.edges.push(Edge {
                            external: false,
                            source: cls_nid.clone(),
                            target: super_nid,
                            relation: "inherits".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                        colon_seen = false;
                    } else if child.kind() == "parameterized_arguments" {
                        // Protocols adopted: @interface Foo : Bar <Proto1, Proto2>
                        let mut pc = child.walk();
                        if pc.goto_first_child() {
                            loop {
                                if pc.node().kind() == "type_name" {
                                    let mut tc = pc.node().walk();
                                    if tc.goto_first_child() {
                                        loop {
                                            if tc.node().kind() == "type_identifier" {
                                                let proto_nid = ctx.ensure_named_node(read_text(
                                                    tc.node(),
                                                    source,
                                                ));
                                                ctx.edges.push(Edge {
                                                    external: false,
                                                    source: cls_nid.clone(),
                                                    target: proto_nid,
                                                    relation: "implements".to_string(),
                                                    confidence: "EXTRACTED".to_string(),
                                                    source_file: ctx.str_path.to_string(),
                                                    source_location: Some(format!("L{line}")),
                                                    weight: 1.0,
                                                    context: None,
                                                    confidence_score: None,
                                                });
                                            }
                                            if !tc.goto_next_sibling() {
                                                break;
                                            }
                                        }
                                    }
                                }
                                if !pc.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                    } else if child.kind() == "property_declaration" {
                        // @property type → references[field] from the class. The
                        // type is a direct type_identifier (`NSString *x`) or
                        // wrapped in a generic_specifier (`NSArray<Product *> *xs`);
                        // walk every type name, skipping the declarator (the field
                        // name), so generic collections are no longer invisible.
                        let prop_line = child.start_position().row + 1;
                        let mut sc = child.walk();
                        if sc.goto_first_child() {
                            loop {
                                if sc.node().kind() == "struct_declaration" {
                                    let mut seen_types: HashSet<&str> = HashSet::new();
                                    let mut dc = sc.node().walk();
                                    if dc.goto_first_child() {
                                        loop {
                                            let s = dc.node();
                                            if !matches!(s.kind(), "struct_declarator" | ";") {
                                                let mut names: Vec<&str> = Vec::new();
                                                collect_objc_type_names(s, source, &mut names);
                                                for tname in names {
                                                    if !seen_types.insert(tname) {
                                                        continue;
                                                    }
                                                    let type_nid = ctx.ensure_named_node(tname);
                                                    ctx.edges.push(Edge {
                                                        external: false,
                                                        source: cls_nid.clone(),
                                                        target: type_nid,
                                                        relation: "references".to_string(),
                                                        confidence: "EXTRACTED".to_string(),
                                                        source_file: ctx.str_path.to_string(),
                                                        source_location: Some(format!(
                                                            "L{prop_line}"
                                                        )),
                                                        weight: 1.0,
                                                        context: Some("field".to_string()),
                                                        confidence_score: None,
                                                    });
                                                }
                                            }
                                            if !dc.goto_next_sibling() {
                                                break;
                                            }
                                        }
                                    }
                                }
                                if !sc.goto_next_sibling() {
                                    break;
                                }
                            }
                        }
                    } else if child.kind() == "method_declaration" {
                        walk_objc(ctx, child, source, Some(&cls_nid));
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
        "class_implementation" => {
            let name = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if cur.node().kind() == "identifier" {
                            f = Some(read_text(cur.node(), source).to_string());
                            break;
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                    f
                } else {
                    None
                }
            };
            if let Some(n) = name {
                let impl_nid = make_id(&[ctx.stem, &n]);
                if ctx.seen_ids.insert(impl_nid.clone()) {
                    ctx.nodes.push(Node {
                        id: impl_nid.clone(),
                        label: n,
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                        origin_file: None,
                    });
                    ctx.edges.push(Edge {
                        external: false,
                        source: ctx.file_nid.to_string(),
                        target: impl_nid.clone(),
                        relation: "contains".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: None,
                        confidence_score: None,
                    });
                }
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "implementation_definition" {
                            let mut c2 = cur.node().walk();
                            if c2.goto_first_child() {
                                loop {
                                    walk_objc(ctx, c2.node(), source, Some(&impl_nid));
                                    if !c2.goto_next_sibling() {
                                        break;
                                    }
                                }
                            }
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "protocol_declaration" => {
            let name = {
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    let mut f = None;
                    loop {
                        if cur.node().kind() == "identifier" {
                            f = Some(read_text(cur.node(), source).to_string());
                            break;
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                    f
                } else {
                    None
                }
            };
            if let Some(n) = name {
                let proto_nid = make_id(&[ctx.stem, &n]);
                if ctx.seen_ids.insert(proto_nid.clone()) {
                    ctx.nodes.push(Node {
                        id: proto_nid.clone(),
                        label: format!("<{n}>"),
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                        origin_file: None,
                    });
                }
                ctx.edges.push(Edge {
                    external: false,
                    source: ctx.file_nid.to_string(),
                    target: proto_nid.clone(),
                    relation: "contains".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        walk_objc(ctx, cur.node(), source, Some(&proto_nid));
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
        }
        "method_declaration" | "method_definition" => {
            let container = parent_nid.unwrap_or(ctx.file_nid);
            // Class methods start with '+', instance methods with '-' (the grammar
            // emits the sigil as a child). The selector is the concatenation of the
            // direct identifier children: one for a simple selector, several for a
            // compound one (-tableView:numberOfRowsInSection:) (#1475).
            let mut prefix = "-";
            let mut prefix_found = false;
            let mut parts: Vec<&str> = Vec::new();
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let kind = cur.node().kind();
                    if !prefix_found && (kind == "+" || kind == "-") {
                        prefix = kind;
                        prefix_found = true;
                    } else if kind == "identifier" {
                        parts.push(read_text(cur.node(), source));
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            if !parts.is_empty() {
                let method_name = parts.concat();
                let method_nid = make_id(&[container, &method_name]);
                if ctx.seen_ids.insert(method_nid.clone()) {
                    ctx.nodes.push(Node {
                        id: method_nid.clone(),
                        label: format!("{prefix}{method_name}"),
                        file_type: "code".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        metadata: None,
                        origin_file: None,
                    });
                }
                ctx.edges.push(Edge {
                    external: false,
                    source: container.to_string(),
                    target: method_nid.clone(),
                    relation: "method".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: ctx.str_path.to_string(),
                    source_location: Some(format!("L{line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
                if t == "method_definition" {
                    ctx.method_bodies
                        .push((method_nid, node.start_byte(), node.end_byte()));
                }
            }
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_objc(ctx, cur.node(), source, parent_nid);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}

/// Collect `calls` edges within an `ObjC` method body.
///
/// Recurses through the body AST, emitting `calls` edges for `message_expression` nodes whose
/// selector matches a known method NID. Mirrors Python `_walk_calls_objc`.
/// Shared state threaded through every [`walk_calls_objc`] recursion.
struct ObjcCallCtx<'a> {
    str_path: &'a str,
    all_method_nids: &'a HashSet<String>,
    edges: &'a mut Vec<Edge>,
    seen_calls: &'a mut HashSet<(String, String)>,
}

fn walk_calls_objc(
    ctx: &mut ObjcCallCtx<'_>,
    node: tree_sitter::Node<'_>,
    source: &[u8],
    caller_nid: &str,
    body_start: usize,
    body_end: usize,
) {
    let str_path = ctx.str_path;
    let all_method_nids = ctx.all_method_nids;
    let edges = &mut *ctx.edges;
    let seen_calls = &mut *ctx.seen_calls;
    if node.start_byte() >= body_end || node.end_byte() <= body_start {
        return;
    }
    if node.kind() == "message_expression" {
        // `[receiver sel]` and `[receiver kw1:a kw2:b]` both parse to a
        // message_expression whose selector parts carry the field name "method"
        // (one for a simple selector, several for a compound one); the receiver
        // carries field name "receiver". Reconstruct the selector from every
        // "method" child so self/super/ClassName receivers are never mistaken for
        // a selector, and compound sends resolve too (#1475).
        let mut sel: Vec<&str> = Vec::new();
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.field_name() == Some("method") && cur.node().kind() == "identifier" {
                    sel.push(read_text(cur.node(), source));
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        let method_name = sel.concat();
        if !method_name.is_empty() {
            // Match against all method nids by suffix
            let suffix_key = make_id1(&method_name);
            for candidate in all_method_nids {
                if candidate.ends_with(suffix_key.trim_start_matches('_')) {
                    let pair = (caller_nid.to_string(), candidate.clone());
                    if !seen_calls.contains(&pair) && caller_nid != candidate {
                        seen_calls.insert(pair);
                        let line = node.start_position().row + 1;
                        edges.push(Edge {
                            external: false,
                            source: caller_nid.to_string(),
                            target: candidate.clone(),
                            relation: "calls".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: Some("call".to_string()),
                            confidence_score: None,
                        });
                    }
                }
            }
        }
    }
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_calls_objc(ctx, cur.node(), source, caller_nid, body_start, body_end);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
