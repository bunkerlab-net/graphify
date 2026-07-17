//! SQL extractor — tables, views, functions, triggers, and relationships.

mod refs;
mod walk;

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};
use regex::Regex;
use walk::{SqlWalkCtx, walk_sql};

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

#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static SQL_ERROR_FN_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Schema-qualified names are kept whole (`[\w$.]+` includes `.`), unlike
    // SQL_FB_HDR_RE. Best-effort recovery of CREATE FUNCTION/PROCEDURE objects
    // from tree-sitter ERROR blobs (PL/pgSQL bodies), #1910. Anchored to line
    // start (indentation only) like SQL_END_RE, which drops the common
    // false-positive of a mid-line body statement (`PERFORM 'CREATE FUNCTION
    // fake()'`). It is a HEURISTIC, not a parser: a line-leading `CREATE` inside
    // a block comment or a dollar-quoted body can still match. Full comment/
    // string masking is beyond this fallback's scope (and beyond graphify-py
    // `sql.py:204`, an unanchored `finditer` that also matches mid-line body
    // text); the anchor is a strict improvement over the reference.
    Regex::new(r"(?im)^[ \t]*CREATE\s+(?:OR\s+REPLACE\s+)?(?:FUNCTION|PROCEDURE)\s+([\w$.]+)")
        .expect("static sql error-fn regex")
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
    n.children(&mut cur)
        .find(|c| c.kind() == "object_reference")
        .map(|c| read_text(c, source))
}

/// Extract tables, views, functions, and relationships from `.sql` files via tree-sitter.
#[must_use]
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
    extract_sql_from_source(path, &source)
}

/// Like [`extract_sql`] but parses in-memory `content` while still attributing
/// nodes/edges to `path`. Used by `--postgres` introspection, which reconstructs
/// DDL in memory and attributes it to a virtual `postgresql://host/db` path.
/// Mirrors the `content=` parameter of graphify-py's `extract_sql`.
#[must_use]
pub fn extract_sql_with_content(path: &Path, content: &[u8]) -> FileResult {
    extract_sql_from_source(path, content)
}

/// Shared body of [`extract_sql`] / [`extract_sql_with_content`]: parse `source`
/// and build the graph, attributing every node/edge to `path`.
#[allow(clippy::too_many_lines)]
fn extract_sql_from_source(path: &Path, source: &[u8]) -> FileResult {
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
    let Some(tree) = parser.parse(source, None) else {
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
        origin_file: None,
        node_type: None,
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
                        walk_sql(&mut walk_ctx, sc.node(), source);
                        if !sc.goto_next_sibling() {
                            break;
                        }
                    }
                }
            } else if matches!(
                stmt.kind(),
                "fb_proc_or_trigger" | "set_term" | "declare_external_function" | "ERROR"
            ) {
                walk_sql(&mut walk_ctx, stmt, source);
            }
            if !cur.goto_next_sibling() {
                break;
            }
        }
    }

    // Global regex fallback for REFERENCES not captured by the tree
    let src_text = String::from_utf8_lossy(source).into_owned();
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
                    external: false,
                    source: tbl_nid.clone(),
                    target: ref_nid,
                    relation: "references".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{tbl_line}")),
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                    deferred: false,
                    metadata: None,
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
