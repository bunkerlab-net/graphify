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
    // Schema-qualified names are kept whole — `[\w$]+(?:\.[\w$]+)*` accepts dotted
    // paths but a component must follow each `.`, so a capture never ends in `.`.
    // Best-effort recovery of CREATE FUNCTION/PROCEDURE objects from tree-sitter
    // ERROR blobs (PL/pgSQL bodies), #1910. Anchored to line start (indentation
    // only) like SQL_END_RE, which drops the common false-positive of a mid-line
    // body statement (`PERFORM 'CREATE FUNCTION fake()'`). A dedicated lexical
    // pass (`mask_sql_noise`, applied in `walk.rs` before this regex) blanks
    // comments and string/dollar-quoted bodies, so a line-leading `CREATE` inside
    // a block comment or a `$$…$$` body no longer matches. This goes beyond
    // graphify-py `sql.py:204` (an unanchored `finditer` that matches mid-line
    // body text); anchor + masking are a strict improvement over the reference.
    //
    // Quoted qualified names (`app."MixedName"`) are NOT recovered from an ERROR
    // blob: the masker blanks double-quoted delimited identifiers (a multi-line
    // one could otherwise embed a line-leading CREATE and false-positive), so the
    // `"MixedName"` component is already spaces here. The trailing `.` left behind
    // is not consumed (a component must follow it), so nothing past `app` is
    // captured — no garbage `app.` name. Recovering the quoted identifier would
    // need header-position-aware masking state, disproportionate for a heuristic
    // that fires ONLY on malformed SQL; well-formed quoted DDL is parsed by
    // tree-sitter and its name read from the AST (this path never runs).
    // graphify-py `sql.py:204` recovers no quoted names either. (Disputes
    // CodeRabbit's "recover quoted qualified routine declarations" finding.)
    Regex::new(
        r"(?im)^[ \t]*CREATE\s+(?:OR\s+REPLACE\s+)?(?:FUNCTION|PROCEDURE)\s+([\w$]+(?:\.[\w$]+)*)",
    )
    .expect("static sql error-fn regex")
});

/// Blank SQL comments and string/dollar-quoted literals to spaces (newlines
/// preserved) so [`SQL_ERROR_FN_RE`] cannot recover a line-leading `CREATE`
/// that lives inside one. A single lexical pass tracks `--` line comments,
/// nested `/* … */` block comments, `'…'` and `"…"` literals (honouring the
/// doubled-quote escape), and `$tag$ … $tag$` dollar-quoted bodies. Newlines
/// are kept in place and each blanked char is replaced by as many spaces as its
/// UTF-8 byte length, so the result is byte-for-byte the same length as `text`
/// and a surviving match's offsets/line number map back to the source.
// A linear single-pass SQL lexer; the per-state arms belong together for
// readability, so splitting them into fragments would obscure the flow.
#[allow(clippy::too_many_lines)]
pub(super) fn mask_sql_noise(text: &str) -> String {
    enum St {
        Normal,
        Line,
        Block(u32),
        /// `true` when the opener was an `E'…'` escape string (backslash escapes).
        Single(bool),
        Double,
        Dollar(Vec<char>),
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut out = chars.clone();
    let blank = |out: &mut [char], k: usize| {
        if out[k] != '\n' {
            out[k] = ' ';
        }
    };
    let mut st = St::Normal;
    let mut i = 0;
    while i < n {
        match &st {
            St::Normal => {
                if chars[i] == '$'
                    && let Some(open_end) = dollar_tag_end(&chars, i)
                {
                    let tag = chars[i..=open_end].to_vec();
                    for k in i..=open_end {
                        blank(&mut out, k);
                    }
                    st = St::Dollar(tag);
                    i = open_end + 1;
                } else if chars[i] == '-' && i + 1 < n && chars[i + 1] == '-' {
                    st = St::Line;
                    i += 1;
                } else if chars[i] == '/' && i + 1 < n && chars[i + 1] == '*' {
                    blank(&mut out, i);
                    blank(&mut out, i + 1);
                    st = St::Block(1);
                    i += 2;
                } else if chars[i] == '\'' {
                    // A leading `E`/`e` at a token boundary marks a PostgreSQL
                    // escape string, where `\` escapes the next char (incl. `\'`).
                    let escapes = i > 0
                        && matches!(chars[i - 1], 'E' | 'e')
                        && (i < 2
                            || !(chars[i - 2].is_ascii_alphanumeric() || chars[i - 2] == '_'));
                    blank(&mut out, i);
                    st = St::Single(escapes);
                    i += 1;
                } else if chars[i] == '"' {
                    blank(&mut out, i);
                    st = St::Double;
                    i += 1;
                } else {
                    i += 1;
                }
            }
            St::Line => {
                if chars[i] == '\n' {
                    st = St::Normal;
                } else {
                    blank(&mut out, i);
                }
                i += 1;
            }
            St::Block(depth) => {
                let depth = *depth;
                if chars[i] == '/' && i + 1 < n && chars[i + 1] == '*' {
                    blank(&mut out, i);
                    blank(&mut out, i + 1);
                    st = St::Block(depth + 1);
                    i += 2;
                } else if chars[i] == '*' && i + 1 < n && chars[i + 1] == '/' {
                    blank(&mut out, i);
                    blank(&mut out, i + 1);
                    st = if depth <= 1 {
                        St::Normal
                    } else {
                        St::Block(depth - 1)
                    };
                    i += 2;
                } else {
                    blank(&mut out, i);
                    i += 1;
                }
            }
            St::Single(escapes) => {
                if *escapes && chars[i] == '\\' && i + 1 < n {
                    // Escape string: `\x` is one escaped char (covers `\'`, `\\`).
                    blank(&mut out, i);
                    blank(&mut out, i + 1);
                    i += 2;
                } else if chars[i] == '\'' {
                    blank(&mut out, i);
                    if i + 1 < n && chars[i + 1] == '\'' {
                        blank(&mut out, i + 1); // doubled quote: stay in the literal
                        i += 2;
                    } else {
                        st = St::Normal;
                        i += 1;
                    }
                } else {
                    blank(&mut out, i);
                    i += 1;
                }
            }
            St::Double => {
                // Double-quoted identifiers: no backslash escapes; `""` is a
                // doubled-quote literal that stays inside.
                if chars[i] == '"' {
                    blank(&mut out, i);
                    if i + 1 < n && chars[i + 1] == '"' {
                        blank(&mut out, i + 1);
                        i += 2;
                    } else {
                        st = St::Normal;
                        i += 1;
                    }
                } else {
                    blank(&mut out, i);
                    i += 1;
                }
            }
            St::Dollar(tag) => {
                if chars[i] == '$' && i + tag.len() <= n && chars[i..i + tag.len()] == tag[..] {
                    for k in i..i + tag.len() {
                        blank(&mut out, k);
                    }
                    i += tag.len();
                    st = St::Normal;
                } else {
                    blank(&mut out, i);
                    i += 1;
                }
            }
        }
    }
    // Rebuild the masked text: a kept char contributes its own bytes; a blanked
    // char (`out[k]` now differs from the source) contributes as many spaces as
    // its UTF-8 byte length, so the masked string is byte-for-byte the same
    // length as the source and every match offset maps back to the original.
    let mut masked = String::with_capacity(text.len());
    for k in 0..n {
        if out[k] == chars[k] {
            masked.push(chars[k]);
        } else {
            for _ in 0..chars[k].len_utf8() {
                masked.push(' ');
            }
        }
    }
    masked
}

/// If `chars[start] == '$'`, index of the closing `$` of the dollar tag
/// (`$$` → `start + 1`; `$name$` → the trailing `$`), else `None`. A `$1`
/// parameter, a lone `$`, or a `$` glued to a preceding identifier character
/// is not a tag.
fn dollar_tag_end(chars: &[char], start: usize) -> Option<usize> {
    // A `$` immediately preceded by an identifier character is part of that
    // identifier — PostgreSQL allows `$` inside identifiers, so `a$$` is the
    // identifier `a$$`, not an empty dollar-quote opener.
    if start > 0 && is_pg_ident_char(chars[start - 1]) {
        return None;
    }
    // `$$` — empty tag.
    let first = *chars.get(start + 1)?;
    if first == '$' {
        return Some(start + 1);
    }
    // `$name$` — a PostgreSQL dollar-quote tag follows the unquoted-identifier
    // rules (minus `$`): an ident-start char, then ident-continuation chars.
    // A digit-first run (`$1`) is a positional parameter, not a tag.
    if !is_pg_ident_start(first) {
        return None;
    }
    let mut j = start + 2;
    while j < chars.len() && is_pg_ident_cont(chars[j]) {
        j += 1;
    }
    (j < chars.len() && chars[j] == '$').then_some(j)
}

// PostgreSQL's lexer (scan.l) defines identifiers as `[A-Za-z\200-\377_]` then
// `[A-Za-z\200-\377_0-9$]`: any non-ASCII byte counts, with no Unicode XID
// validation, so combining marks and CJK letters are all accepted verbatim.

/// A `PostgreSQL` identifier-start char: ASCII letter, `_`, or any non-ASCII char.
fn is_pg_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || !c.is_ascii()
}

/// A `PostgreSQL` identifier-continuation char, excluding `$` (a dollar-quote
/// tag cannot contain `$`): ASCII alphanumeric, `_`, or any non-ASCII char.
fn is_pg_ident_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || !c.is_ascii()
}

/// A `PostgreSQL` identifier char (any position): [`is_pg_ident_cont`] plus `$`.
fn is_pg_ident_char(c: char) -> bool {
    is_pg_ident_cont(c) || c == '$'
}

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
