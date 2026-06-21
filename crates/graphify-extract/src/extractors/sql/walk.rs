//! SQL structural AST walk (tables, views, functions, triggers).

use super::refs::walk_from_refs;
use super::{
    SQL_FB_HDR_RE, SQL_FOR_RE, SQL_FROM_RE, SQL_NON_TABLES, SQL_REF_RE, SQL_UPDATE_RE, obj_name,
    read_text,
};
use crate::ids::make_id;
use crate::types::{Edge, Node};
use std::collections::HashSet;

/// Recursively walk a SQL AST emitting nodes for tables, views, and functions.
///
/// Handles `create_table_statement`, `create_view_statement`, `create_function_statement`,
/// and `create_procedure_statement`. Also records `table_nids` for use by `walk_from_refs`.
/// Mirrors Python `_walk_sql`.
/// Shared state threaded through every [`walk_sql`] recursion.
pub(super) struct SqlWalkCtx<'a> {
    pub(super) str_path: &'a str,
    pub(super) stem: &'a str,
    pub(super) file_nid: &'a str,
    pub(super) nodes: &'a mut Vec<Node>,
    pub(super) edges: &'a mut Vec<Edge>,
    pub(super) seen_ids: &'a mut HashSet<String>,
    pub(super) table_nids: &'a mut std::collections::HashMap<String, String>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over SQL's AST node kinds
pub(super) fn walk_sql(ctx: &mut SqlWalkCtx<'_>, node: tree_sitter::Node<'_>, source: &[u8]) {
    let t = node.kind();
    let line = node.start_position().row + 1;

    // Capture the immutable fields for the closure; the closure receives the
    // mutable accumulators by reference so it does not borrow ctx.
    let str_path = ctx.str_path;
    let file_nid = ctx.file_nid;
    let add_node = |nid: &str,
                    label: &str,
                    ln: usize,
                    nodes: &mut Vec<Node>,
                    edges: &mut Vec<Edge>,
                    seen: &mut HashSet<String>| {
        if seen.insert(nid.to_string()) {
            nodes.push(Node {
                id: nid.to_string(),
                label: label.to_string(),
                file_type: "code".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{ln}")),
                metadata: None,
            });
            edges.push(Edge {
                external: false,
                source: file_nid.to_string(),
                target: nid.to_string(),
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{ln}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
        }
    };

    match t {
        "create_table" => {
            if let Some(name) = obj_name(node, source) {
                let nid = make_id(&[ctx.stem, name]);
                add_node(&nid, name, line, ctx.nodes, ctx.edges, ctx.seen_ids);
                ctx.table_nids.insert(name.to_lowercase(), nid.clone());
                // Foreign key references
                for col in node.children_by_field_name("column", &mut node.walk()) {
                    let _ = col; // handled below
                }
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "column_definitions" {
                            let col_def = cur.node();
                            let has_error = {
                                let mut c = col_def.walk();
                                let mut found = false;
                                if c.goto_first_child() {
                                    loop {
                                        if c.node().kind() == "ERROR" {
                                            found = true;
                                            break;
                                        }
                                        if !c.goto_next_sibling() {
                                            break;
                                        }
                                    }
                                }
                                found
                            };
                            let mut c2 = col_def.walk();
                            let mut seen_refs: HashSet<String> = HashSet::new();
                            if c2.goto_first_child() {
                                loop {
                                    let cd = c2.node();
                                    if cd.kind() == "column_definition" {
                                        let mut found_ref = false;
                                        let mut ref_name: Option<&str> = None;
                                        let mut cc = cd.walk();
                                        if cc.goto_first_child() {
                                            loop {
                                                if cc.node().kind() == "keyword_references" {
                                                    found_ref = true;
                                                } else if found_ref
                                                    && cc.node().kind() == "object_reference"
                                                {
                                                    ref_name = Some(read_text(cc.node(), source));
                                                    break;
                                                }
                                                if !cc.goto_next_sibling() {
                                                    break;
                                                }
                                            }
                                        }
                                        if let Some(rn) = ref_name {
                                            let ref_nid = ctx
                                                .table_nids
                                                .get(&rn.to_lowercase())
                                                .cloned()
                                                .unwrap_or_else(|| make_id(&[ctx.stem, rn]));
                                            ctx.edges.push(Edge {
                                                external: false,
                                                source: nid.clone(),
                                                target: ref_nid,
                                                relation: "references".to_string(),
                                                confidence: "EXTRACTED".to_string(),
                                                source_file: ctx.str_path.to_string(),
                                                source_location: Some(format!("L{line}")),
                                                weight: 1.0,
                                                context: None,
                                                confidence_score: None,
                                            });
                                            seen_refs.insert(rn.to_lowercase());
                                        }
                                    } else if cd.kind() == "constraints" {
                                        let mut cc = cd.walk();
                                        if cc.goto_first_child() {
                                            loop {
                                                if cc.node().kind() == "constraint" {
                                                    let mut found_ref = false;
                                                    let mut ref_name: Option<&str> = None;
                                                    let mut ccc = cc.node().walk();
                                                    if ccc.goto_first_child() {
                                                        loop {
                                                            if ccc.node().kind()
                                                                == "keyword_references"
                                                            {
                                                                found_ref = true;
                                                            } else if found_ref
                                                                && ccc.node().kind()
                                                                    == "object_reference"
                                                            {
                                                                ref_name = Some(read_text(
                                                                    ccc.node(),
                                                                    source,
                                                                ));
                                                                break;
                                                            }
                                                            if !ccc.goto_next_sibling() {
                                                                break;
                                                            }
                                                        }
                                                    }
                                                    if let Some(rn) = ref_name {
                                                        let ref_nid = ctx
                                                            .table_nids
                                                            .get(&rn.to_lowercase())
                                                            .cloned()
                                                            .unwrap_or_else(|| {
                                                                make_id(&[ctx.stem, rn])
                                                            });
                                                        ctx.edges.push(Edge {
                                                            external: false,
                                                            source: nid.clone(),
                                                            target: ref_nid,
                                                            relation: "references".to_string(),
                                                            confidence: "EXTRACTED".to_string(),
                                                            source_file: ctx.str_path.to_string(),
                                                            source_location: Some(format!(
                                                                "L{line}"
                                                            )),
                                                            weight: 1.0,
                                                            context: None,
                                                            confidence_score: None,
                                                        });
                                                        seen_refs.insert(rn.to_lowercase());
                                                    }
                                                }
                                                if !cc.goto_next_sibling() {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    if !c2.goto_next_sibling() {
                                        break;
                                    }
                                }
                            }
                            if has_error {
                                // Regex fallback for REFERENCES in dialect-specific syntax
                                let col_text = read_text(col_def, source);
                                for rm in SQL_REF_RE.captures_iter(col_text) {
                                    let rn = &rm[1];
                                    if !seen_refs.contains(&rn.to_lowercase()) {
                                        let ref_nid = ctx
                                            .table_nids
                                            .get(&rn.to_lowercase())
                                            .cloned()
                                            .unwrap_or_else(|| make_id(&[ctx.stem, rn]));
                                        ctx.edges.push(Edge {
                                            external: false,
                                            source: nid.clone(),
                                            target: ref_nid,
                                            relation: "references".to_string(),
                                            confidence: "EXTRACTED".to_string(),
                                            source_file: ctx.str_path.to_string(),
                                            source_location: Some(format!("L{line}")),
                                            weight: 1.0,
                                            context: None,
                                            confidence_score: None,
                                        });
                                        seen_refs.insert(rn.to_lowercase());
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
        "create_view" => {
            if let Some(name) = obj_name(node, source) {
                let nid = make_id(&[ctx.stem, name]);
                add_node(&nid, name, line, ctx.nodes, ctx.edges, ctx.seen_ids);
                ctx.table_nids.insert(name.to_lowercase(), nid.clone());
                walk_from_refs(node, source, ctx.str_path, ctx.stem, &nid, ctx.edges);
            }
        }
        "create_function" | "create_procedure" => {
            if let Some(name) = obj_name(node, source) {
                let nid = make_id(&[ctx.stem, name]);
                add_node(
                    &nid,
                    &format!("{name}()"),
                    line,
                    ctx.nodes,
                    ctx.edges,
                    ctx.seen_ids,
                );
                walk_from_refs(node, source, ctx.str_path, ctx.stem, &nid, ctx.edges);
            }
        }
        "alter_table" => {
            if let Some(name) = obj_name(node, source) {
                let src_nid = ctx
                    .table_nids
                    .get(&name.to_lowercase())
                    .cloned()
                    .unwrap_or_else(|| {
                        let n = make_id(&[ctx.stem, name]);
                        add_node(&n, name, line, ctx.nodes, ctx.edges, ctx.seen_ids);
                        ctx.table_nids.insert(name.to_lowercase(), n.clone());
                        n
                    });
                let mut cur = node.walk();
                if cur.goto_first_child() {
                    loop {
                        if cur.node().kind() == "add_constraint" {
                            let mut c2 = cur.node().walk();
                            if c2.goto_first_child() {
                                loop {
                                    if c2.node().kind() == "constraint" {
                                        let mut found_ref = false;
                                        let mut ref_name: Option<String> = None;
                                        let mut c3 = c2.node().walk();
                                        if c3.goto_first_child() {
                                            loop {
                                                if c3.node().kind() == "keyword_references" {
                                                    found_ref = true;
                                                } else if found_ref
                                                    && c3.node().kind() == "object_reference"
                                                {
                                                    ref_name = Some(
                                                        read_text(c3.node(), source).to_string(),
                                                    );
                                                    break;
                                                }
                                                if !c3.goto_next_sibling() {
                                                    break;
                                                }
                                            }
                                        }
                                        if let Some(rn) = ref_name {
                                            let ref_nid = ctx
                                                .table_nids
                                                .get(&rn.to_lowercase())
                                                .cloned()
                                                .unwrap_or_else(|| make_id(&[ctx.stem, &rn]));
                                            ctx.edges.push(Edge {
                                                external: false,
                                                source: src_nid.clone(),
                                                target: ref_nid,
                                                relation: "references".to_string(),
                                                confidence: "EXTRACTED".to_string(),
                                                source_file: ctx.str_path.to_string(),
                                                source_location: Some(format!("L{line}")),
                                                weight: 1.0,
                                                context: None,
                                                confidence_score: None,
                                            });
                                        }
                                    }
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
        "create_trigger" => {
            let mut trig_name: Option<String> = None;
            let mut tbl_name: Option<String> = None;
            let mut after_trigger = false;
            let mut after_for = false;
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    let c = cur.node();
                    if c.kind() == "keyword_trigger" {
                        after_trigger = true;
                    } else if after_trigger && trig_name.is_none() && c.kind() == "object_reference"
                    {
                        trig_name = Some(read_text(c, source).to_string());
                    } else if c.kind() == "keyword_for" {
                        after_for = true;
                    } else if after_for && tbl_name.is_none() && c.kind() == "object_reference" {
                        tbl_name = Some(read_text(c, source).to_string());
                    }
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
            if let Some(tn) = trig_name {
                let trig_nid = make_id(&[ctx.stem, &tn]);
                add_node(&trig_nid, &tn, line, ctx.nodes, ctx.edges, ctx.seen_ids);
                if let Some(tbl) = tbl_name {
                    let tbl_nid = ctx
                        .table_nids
                        .get(&tbl.to_lowercase())
                        .cloned()
                        .unwrap_or_else(|| make_id(&[ctx.stem, &tbl]));
                    ctx.edges.push(Edge {
                        external: false,
                        source: trig_nid,
                        target: tbl_nid,
                        relation: "triggers".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: None,
                        confidence_score: None,
                    });
                }
            }
        }
        "fb_proc_or_trigger" => {
            let text = read_text(node, source);
            if let Some(cap) = SQL_FB_HDR_RE.captures(text) {
                let obj_type = cap[1].to_uppercase();
                let obj_name = cap[2].to_string();
                let obj_nid = make_id(&[ctx.stem, &obj_name]);
                let label = if obj_type == "TRIGGER" {
                    obj_name.clone()
                } else {
                    format!("{obj_name}()")
                };
                add_node(&obj_nid, &label, line, ctx.nodes, ctx.edges, ctx.seen_ids);
                if obj_type == "TRIGGER"
                    && let Some(fm) = SQL_FOR_RE.captures(text)
                {
                    let tbl = fm[1].to_string();
                    let tbl_nid = ctx
                        .table_nids
                        .get(&tbl.to_lowercase())
                        .cloned()
                        .unwrap_or_else(|| make_id(&[ctx.stem, &tbl]));
                    ctx.edges.push(Edge {
                        external: false,
                        source: obj_nid.clone(),
                        target: tbl_nid,
                        relation: "triggers".to_string(),
                        confidence: "EXTRACTED".to_string(),
                        source_file: ctx.str_path.to_string(),
                        source_location: Some(format!("L{line}")),
                        weight: 1.0,
                        context: None,
                        confidence_score: None,
                    });
                }
                let mut seen_tbls: HashSet<String> = HashSet::new();
                for rm in SQL_FROM_RE.captures_iter(text) {
                    let tbl = rm[1].to_string();
                    if !SQL_NON_TABLES.contains(tbl.to_lowercase().as_str())
                        && !seen_tbls.contains(&tbl.to_lowercase())
                    {
                        seen_tbls.insert(tbl.to_lowercase());
                        let tbl_nid = ctx
                            .table_nids
                            .get(&tbl.to_lowercase())
                            .cloned()
                            .unwrap_or_else(|| make_id(&[ctx.stem, &tbl]));
                        ctx.edges.push(Edge {
                            external: false,
                            source: obj_nid.clone(),
                            target: tbl_nid,
                            relation: "reads_from".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                    }
                }
                for rm in SQL_UPDATE_RE.captures_iter(text) {
                    let tbl = rm[1].to_string();
                    if !SQL_NON_TABLES.contains(tbl.to_lowercase().as_str())
                        && !seen_tbls.contains(&tbl.to_lowercase())
                    {
                        seen_tbls.insert(tbl.to_lowercase());
                        let tbl_nid = ctx
                            .table_nids
                            .get(&tbl.to_lowercase())
                            .cloned()
                            .unwrap_or_else(|| make_id(&[ctx.stem, &tbl]));
                        ctx.edges.push(Edge {
                            external: false,
                            source: obj_nid.clone(),
                            target: tbl_nid,
                            relation: "reads_from".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: ctx.str_path.to_string(),
                            source_location: Some(format!("L{line}")),
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                    }
                }
            }
        }
        _ => {
            let mut cur = node.walk();
            if cur.goto_first_child() {
                loop {
                    walk_sql(ctx, cur.node(), source);
                    if !cur.goto_next_sibling() {
                        break;
                    }
                }
            }
        }
    }
}
