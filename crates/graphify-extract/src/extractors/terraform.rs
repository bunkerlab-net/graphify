//! Terraform / HCL extractor via tree-sitter-hcl.
//!
//! Mirrors `graphify-py` `extract_terraform`. Emits nodes for resources, data
//! sources, modules, variables, outputs, providers, and locals; `contains`
//! edges (file → block), `references` edges (block → interpolated blocks), and
//! `depends_on` edges. Node IDs are scoped by the parent **directory** name, not
//! the file stem, so a resource defined in `main.tf` resolves when referenced
//! from a sibling `.tf` file once per-file extractions are merged.

use std::collections::HashSet;
use std::path::Path;

use tree_sitter::Node as TsNode;

use crate::ids::{make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

/// Head tokens in an HCL traversal that are meta/builtins, not references to a
/// block defined in the corpus (`count.index`, `each.key`, `self.*`,
/// `path.module`, …).
const TF_META_HEADS: &[&str] = &["count", "each", "self", "path", "terraform"];

/// Read the source span of `node` as a `&str` (lossy on bad UTF-8).
fn read<'a>(node: TsNode<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Read a node's text, trimmed and unquoted (mirrors `_label_text`).
fn label_text(node: TsNode<'_>, source: &[u8]) -> String {
    read(node, source).trim().trim_matches('"').to_string()
}

/// Mutable bookkeeping for one Terraform file extraction.
struct TfCtx<'a> {
    source: &'a [u8],
    str_path: &'a str,
    scope: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    seen_edges: &'a mut HashSet<(String, String, String)>,
}

impl TfCtx<'_> {
    /// Add a directory-scoped block node (and its `contains` edge) once.
    fn add_node(&mut self, address: &str, label: &str, line: usize) -> String {
        let nid = make_id(&[self.scope, address]);
        if self.seen_ids.insert(nid.clone()) {
            self.nodes.push(Node {
                id: nid.clone(),
                label: label.to_string(),
                file_type: "code".to_string(),
                source_file: self.str_path.to_string(),
                source_location: Some(format!("L{line}")),
                metadata: None,
                origin_file: None,
            });
            self.edges.push(Edge {
                external: false,
                source: self.file_nid.to_string(),
                target: nid.clone(),
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: self.str_path.to_string(),
                source_location: Some(format!("L{line}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
        }
        nid
    }

    /// Add a `references` / `depends_on` edge to a directory-scoped address,
    /// deduped by `(source, target, relation)`. Self-loops are skipped.
    fn add_edge(&mut self, src: &str, address: &str, relation: &str, line: usize) {
        let tgt = make_id(&[self.scope, address]);
        if src == tgt {
            return;
        }
        let key = (src.to_string(), tgt.clone(), relation.to_string());
        if !self.seen_edges.insert(key) {
            return;
        }
        self.edges.push(Edge {
            external: false,
            source: src.to_string(),
            target: tgt,
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
        });
    }

    /// Walk an expression/body subtree emitting `references` (or `depends_on`)
    /// edges from `owner_nid` to every block address it interpolates.
    fn collect_refs(&mut self, node: TsNode<'_>, owner_nid: &str, relation: &str) {
        let mut rel = relation;
        if node.kind() == "attribute" {
            let key_node = node.child_by_field_name("key").or_else(|| node.child(0));
            if key_node.is_some_and(|k| read(k, self.source) == "depends_on") {
                rel = "depends_on";
            }
        }
        if node.kind() == "variable_expr"
            && let Some(addr) = ref_address(node, self.source)
        {
            self.add_edge(owner_nid, &addr, rel, node.start_position().row + 1);
        }
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                let child = cur.node();
                if child.is_named() {
                    self.collect_refs(child, owner_nid, rel);
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }
}

/// Resolve an HCL `variable_expr` (plus its trailing `get_attr` chain) to a
/// block address like `var.region`, `data.aws_ami.ubuntu`, or `aws_instance.web`.
/// Returns `None` for meta heads and incomplete traversals. Mirrors `_ref_address`.
fn ref_address(expr: TsNode<'_>, source: &[u8]) -> Option<String> {
    let head = read(expr, source);
    let mut attrs: Vec<String> = Vec::new();
    if let Some(parent) = expr.parent() {
        let mut seen_self = false;
        let mut cur = parent.walk();
        if cur.goto_first_child() {
            loop {
                let c = cur.node();
                if c.id() == expr.id() {
                    seen_self = true;
                } else if seen_self && c.kind() == "get_attr" {
                    let mut name: Option<String> = None;
                    let mut gc = c.walk();
                    if gc.goto_first_child() {
                        loop {
                            if gc.node().kind() == "identifier" {
                                name = Some(read(gc.node(), source).to_string());
                                break;
                            }
                            if !gc.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                    match name {
                        Some(n) => attrs.push(n),
                        None => break,
                    }
                } else if seen_self {
                    break;
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
    }

    if head.is_empty() || TF_META_HEADS.contains(&head) {
        return None;
    }
    match head {
        "var" => attrs.first().map(|a| format!("var.{a}")),
        "local" => attrs.first().map(|a| format!("local.{a}")),
        "module" => attrs.first().map(|a| format!("module.{a}")),
        "data" => {
            if attrs.len() >= 2 {
                Some(format!("data.{}.{}", attrs[0], attrs[1]))
            } else {
                None
            }
        }
        _ => attrs.first().map(|a| format!("{head}.{a}")),
    }
}

/// Return `(block_type, labels)` for an HCL `block`, reading the leading
/// identifier and string labels up to the block body. Mirrors `_block_parts`.
fn block_parts(block: TsNode<'_>, source: &[u8]) -> (Option<String>, Vec<String>) {
    let mut btype: Option<String> = None;
    let mut labels: Vec<String> = Vec::new();
    let mut cur = block.walk();
    if cur.goto_first_child() {
        loop {
            let c = cur.node();
            if matches!(c.kind(), "block_start" | "body" | "block_end") {
                break;
            }
            if c.kind() == "identifier" && btype.is_none() {
                btype = Some(read(c, source).to_string());
            } else if matches!(c.kind(), "string_lit" | "identifier") {
                labels.push(label_text(c, source));
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    (btype, labels)
}

/// First `body` child of `block`, or `None`.
fn body_of(block: TsNode<'_>) -> Option<TsNode<'_>> {
    let mut cur = block.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().kind() == "body" {
                return Some(cur.node());
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Extract Terraform/HCL blocks and the references between them. Mirrors
/// `graphify-py` `extract_terraform`.
#[must_use]
pub fn extract_terraform(path: &Path) -> FileResult {
    let Ok(source) = std::fs::read(path) else {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some(format!("cannot read {}", path.display())),
        };
    };

    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_hcl::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set hcl language".to_string()),
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

    let str_path = path.to_string_lossy().into_owned();
    let file_nid = make_id1(&str_path);
    // Directory-scoped IDs: resources are module(directory)-scoped, so a
    // definition and its cross-file references share a scope.
    let scope = path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "tf".to_string());

    let mut nodes: Vec<Node> = vec![Node {
        id: file_nid.clone(),
        label: path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: None,
        metadata: None,
        origin_file: None,
    }];
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::from([file_nid.clone()]);
    let mut seen_edges: HashSet<(String, String, String)> = HashSet::new();

    let mut ctx = TfCtx {
        source: &source,
        str_path: &str_path,
        scope: &scope,
        file_nid: &file_nid,
        nodes: &mut nodes,
        edges: &mut edges,
        seen_ids: &mut seen_ids,
        seen_edges: &mut seen_edges,
    };

    let root = tree.root_node();
    // A leading comment means the body is not necessarily root.child(0).
    let body = {
        let mut found = root;
        let mut cur = root.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().kind() == "body" {
                    found = cur.node();
                    break;
                }
                if !cur.goto_next_sibling() {
                    break;
                }
            }
        }
        found
    };

    let mut bcur = body.walk();
    if bcur.goto_first_child() {
        loop {
            let block = bcur.node();
            if block.kind() == "block" {
                handle_block(&mut ctx, block);
            }
            if !bcur.goto_next_sibling() {
                break;
            }
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}

/// Emit the node + reference edges for a single top-level HCL block.
fn handle_block(ctx: &mut TfCtx<'_>, block: TsNode<'_>) {
    let (btype, labels) = block_parts(block, ctx.source);
    let line = block.start_position().row + 1;
    let blk_body = body_of(block);
    let btype = btype.as_deref();

    let owner = match btype {
        Some("resource") if labels.len() >= 2 => {
            let addr = format!("{}.{}", labels[0], labels[1]);
            ctx.add_node(&addr, &addr, line)
        }
        Some("data") if labels.len() >= 2 => {
            let addr = format!("data.{}.{}", labels[0], labels[1]);
            ctx.add_node(&addr, &addr, line)
        }
        Some("module") if !labels.is_empty() => {
            let addr = format!("module.{}", labels[0]);
            ctx.add_node(&addr, &addr, line)
        }
        Some("variable") if !labels.is_empty() => {
            let addr = format!("var.{}", labels[0]);
            ctx.add_node(&addr, &addr, line)
        }
        Some("output") if !labels.is_empty() => {
            let addr = format!("output.{}", labels[0]);
            ctx.add_node(&addr, &addr, line)
        }
        Some("provider") if !labels.is_empty() => {
            let addr = format!("provider.{}", labels[0]);
            ctx.add_node(&addr, &addr, line)
        }
        Some("locals") => {
            if let Some(blk_body) = blk_body {
                let mut cur = blk_body.walk();
                if cur.goto_first_child() {
                    loop {
                        let attr = cur.node();
                        if attr.kind() == "attribute"
                            && let Some(key_node) = attr.child(0)
                        {
                            let key = read(key_node, ctx.source).to_string();
                            let addr = format!("local.{key}");
                            let lnid = ctx.add_node(&addr, &addr, attr.start_position().row + 1);
                            ctx.collect_refs(attr, &lnid, "references");
                        }
                        if !cur.goto_next_sibling() {
                            break;
                        }
                    }
                }
            }
            return;
        }
        _ => return,
    };

    if let Some(blk_body) = blk_body {
        ctx.collect_refs(blk_body, &owner, "references");
    }
}
