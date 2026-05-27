//! SQL extractor — tables, views, functions, triggers, and relationships.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};
use regex::Regex;

/// Matches `REFERENCES <table>` in SQL fragments for FK extraction.
#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static SQL_REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bREFERENCES\s+([\w$]+)").expect("static sql references regex")
});

#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static SQL_FB_HDR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)CREATE\s+(?:OR\s+(?:REPLACE|ALTER)\s+)?(PROCEDURE|TRIGGER|FUNCTION)\s+([\w$]+)",
    )
    .expect("static sql fb-header regex")
});

#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static SQL_FOR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bFOR\s+([\w$]+)").expect("static sql for regex"));

#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static SQL_FROM_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:FROM|JOIN|INTO)\s+([\w$]+)").expect("static sql from regex")
});

#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static SQL_UPDATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bUPDATE\s+([\w$]+)").expect("static sql update regex"));

static SQL_NON_TABLES: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    [
        "select", "where", "set", "dual", "null", "true", "false", "first", "skip", "rows", "next",
        "only", "lateral",
    ]
    .into_iter()
    .collect()
});

#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static SQL_CREATE_TABLE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)CREATE\s+TABLE\s+([\w$]+)\s*\(").expect("static sql create-table regex")
});

#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static SQL_END_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|\n)(?:CREATE|SET\s+TERM|ALTER)\s").expect("static sql end regex")
});

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract the text of the first `object_reference` child of `n`.
///
/// Used to pull table/view names from SQL DDL statement nodes such as
/// `create_table_statement`, `create_view_statement`, etc.
fn obj_name<'a>(n: tree_sitter::Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    let mut cur = n.walk();
    if cur.goto_first_child() {
        loop {
            if cur.node().kind() == "object_reference" {
                return Some(read_text(cur.node(), source));
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
    None
}

/// Extract tables, views, functions, and relationships from `.sql` files via tree-sitter.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn extract_sql(path: &Path) -> FileResult {
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
        .set_language(&tree_sitter_sequel::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set sql language".to_string()),
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
    let mut table_nids: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    let file_nid = make_id1(&str_path);
    seen_ids.insert(file_nid.clone());
    nodes.push(Node {
        id: file_nid.clone(),
        label: path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: None,
        metadata: None,
    });

    let root = tree.root_node();

    // Walk top-level statements
    let mut cur = root.walk();
    if cur.goto_first_child() {
        let mut walk_ctx = SqlWalkCtx {
            str_path: &str_path,
            stem: &stem,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            table_nids: &mut table_nids,
        };
        loop {
            let stmt = cur.node();
            if stmt.kind() == "statement" {
                let mut sc = stmt.walk();
                if sc.goto_first_child() {
                    loop {
                        walk_sql(&mut walk_ctx, sc.node(), &source);
                        if !sc.goto_next_sibling() {
                            break;
                        }
                    }
                }
            } else if matches!(
                stmt.kind(),
                "fb_proc_or_trigger" | "set_term" | "declare_external_function"
            ) {
                walk_sql(&mut walk_ctx, stmt, &source);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }

    // Global regex fallback for REFERENCES not captured by the tree
    let src_text = String::from_utf8_lossy(&source).into_owned();
    let emitted: HashSet<(String, String)> = edges
        .iter()
        .filter(|e| e.relation == "references")
        .map(|e| (e.source.clone(), e.target.clone()))
        .collect();
    let mut emitted = emitted;

    for m in SQL_CREATE_TABLE_RE.find_iter(&src_text) {
        let cap = SQL_CREATE_TABLE_RE
            .captures(&src_text[m.start()..])
            .and_then(|c| c.get(1).map(|g| g.as_str().to_string()));
        let Some(tbl_name) = cap else { continue };
        let Some(tbl_nid) = table_nids.get(&tbl_name.to_lowercase()).cloned() else {
            continue;
        };
        let tbl_line = src_text[..m.start()].chars().filter(|&c| c == '\n').count() + 1;
        let tail = &src_text[m.start()..];
        let block_end = SQL_END_RE
            .find(&tail[1..])
            .map_or(tail.len(), |em| em.start() + 1);
        let block = &tail[..block_end];
        for rm in SQL_REF_RE.find_iter(block) {
            let rcap = SQL_REF_RE
                .captures(&block[rm.start()..])
                .and_then(|c| c.get(1).map(|g| g.as_str().to_string()));
            let Some(ref_name) = rcap else { continue };
            let ref_nid = table_nids
                .get(&ref_name.to_lowercase())
                .cloned()
                .unwrap_or_else(|| make_id(&[&stem, &ref_name]));
            let key = (tbl_nid.clone(), ref_nid.clone());
            if !emitted.contains(&key) {
                emitted.insert(key);
                edges.push(Edge {
                    source: tbl_nid.clone(),
                    target: ref_nid,
                    relation: "references".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{tbl_line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
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

/// Recursively walk a SQL AST emitting nodes for tables, views, and functions.
///
/// Handles `create_table_statement`, `create_view_statement`, `create_function_statement`,
/// and `create_procedure_statement`. Also records `table_nids` for use by `walk_from_refs`.
/// Mirrors Python `_walk_sql`.
/// Shared state threaded through every [`walk_sql`] recursion.
struct SqlWalkCtx<'a> {
    str_path: &'a str,
    stem: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    table_nids: &'a mut std::collections::HashMap<String, String>,
}

#[allow(clippy::too_many_lines)] // linear dispatch over SQL's AST node kinds
fn walk_sql(ctx: &mut SqlWalkCtx<'_>, node: tree_sitter::Node<'_>, source: &[u8]) {
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

/// Recursively walk a SQL AST finding `FROM` and `JOIN` clauses and emitting `references` edges.
///
/// Used to add query-time data-flow edges from functions/views to the tables they read.
/// Mirrors Python `_walk_from_refs`.
fn walk_from_refs(
    node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    caller_nid: &str,
    edges: &mut Vec<Edge>,
) {
    if matches!(node.kind(), "from" | "join") {
        let mut cur = node.walk();
        if cur.goto_first_child() {
            loop {
                if cur.node().kind() == "relation" {
                    let mut rc = cur.node().walk();
                    if rc.goto_first_child() {
                        loop {
                            if rc.node().kind() == "object_reference" {
                                let tbl = read_text(rc.node(), source);
                                let tbl_nid = make_id(&[stem, tbl]);
                                let line = rc.node().start_position().row + 1;
                                edges.push(Edge {
                                    source: caller_nid.to_string(),
                                    target: tbl_nid,
                                    relation: "reads_from".to_string(),
                                    confidence: "EXTRACTED".to_string(),
                                    source_file: str_path.to_string(),
                                    source_location: Some(format!("L{line}")),
                                    weight: 1.0,
                                    context: None,
                                    confidence_score: None,
                                });
                            }
                            if !rc.goto_next_sibling() {
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
    let mut cur = node.walk();
    if cur.goto_first_child() {
        loop {
            walk_from_refs(cur.node(), source, str_path, stem, caller_nid, edges);
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }
}
