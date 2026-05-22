//! JSON extractor — top-level keys, nested structure, and dependency edges.

use std::collections::HashSet;
use std::path::Path;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

const JSON_MAX_BYTES: usize = 1_048_576; // 1 MiB

static DEP_KEYS: std::sync::LazyLock<HashSet<&'static str>> = std::sync::LazyLock::new(|| {
    [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
        "bundleDependencies",
        "bundledDependencies",
    ]
    .into_iter()
    .collect()
});

/// Return the source bytes covered by `node` as a UTF-8 `&str`, or `""` on bad UTF-8.
fn read_text<'a>(node: tree_sitter::Node<'_>, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// Extract top-level keys, nested structure, and dependency edges from a `.json` file.
#[must_use]
pub fn extract_json(path: &Path) -> FileResult {
    // Bounded read (1 MiB + 1 to detect oversized)
    let source = match std::fs::File::open(path) {
        Ok(mut f) => {
            use std::io::Read;
            let mut buf = vec![0u8; JSON_MAX_BYTES + 1];
            match f.read(&mut buf) {
                Ok(n) => {
                    buf.truncate(n);
                    buf
                }
                Err(e) => {
                    return FileResult {
                        nodes: vec![],
                        edges: vec![],
                        raw_calls: vec![],
                        error: Some(e.to_string()),
                    };
                }
            }
        }
        Err(e) => {
            return FileResult {
                nodes: vec![],
                edges: vec![],
                raw_calls: vec![],
                error: Some(e.to_string()),
            };
        }
    };
    if source.len() > JSON_MAX_BYTES {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("json file too large to index".to_string()),
        };
    }

    let mut parser = tree_sitter::Parser::new();
    if parser
        .set_language(&tree_sitter_json::LANGUAGE.into())
        .is_err()
    {
        return FileResult {
            nodes: vec![],
            edges: vec![],
            raw_calls: vec![],
            error: Some("failed to set json language".to_string()),
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
    });

    // Find root object
    let root = tree.root_node();
    let doc = if root.kind() == "document" && root.child_count() > 0 {
        root.child(0).unwrap_or(root)
    } else {
        root
    };

    if doc.kind() == "object" {
        walk_json_object(
            doc,
            &source,
            &str_path,
            &stem,
            &file_nid,
            &file_nid,
            None,
            0,
            &mut [0usize],
            &mut nodes,
            &mut edges,
            &mut seen_ids,
        );
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}

/// Extract the string content of the key from a JSON `pair` node, stripping enclosing quotes.
fn key_text<'a>(pair_node: tree_sitter::Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    let key_node = pair_node.child_by_field_name("key")?;
    if key_node.kind() == "string" {
        if let Some(content) = key_node.child_by_field_name("string_content") {
            return Some(read_text(content, source));
        }
        let raw = read_text(key_node, source);
        return Some(raw.trim_matches(|c| c == '"' || c == '\''));
    }
    Some(read_text(key_node, source))
}

/// Recursively walk a JSON object node, emitting graph nodes for nested structure.
///
/// Depth-limited to 6 levels to avoid over-expanding deeply nested configs. At each level,
/// `object` pairs become child nodes with `contains` edges to `parent_nid`. Mirrors Python
/// `_walk_json_object`.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn walk_json_object(
    obj_node: tree_sitter::Node<'_>,
    source: &[u8],
    str_path: &str,
    stem: &str,
    file_nid: &str,
    parent_nid: &str,
    parent_key: Option<&str>,
    depth: usize,
    pair_count: &mut [usize],
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
) {
    if depth > 6 {
        return;
    }
    let mut cur = obj_node.walk();
    if !cur.goto_first_child() {
        return;
    }
    loop {
        let child = cur.node();
        if child.kind() != "pair" {
            if !cur.goto_next_sibling() {
                break;
            }
            continue;
        }
        if pair_count[0] >= 500 {
            break;
        }
        pair_count[0] += 1;

        let Some(key) = key_text(child, source) else {
            if !cur.goto_next_sibling() {
                break;
            }
            continue;
        };
        let key_nid = if let Some(pk) = parent_key {
            make_id(&[stem, pk, key])
        } else {
            make_id(&[stem, key])
        };
        if key_nid.is_empty() {
            if !cur.goto_next_sibling() {
                break;
            }
            continue;
        }
        let line = child.start_position().row + 1;
        if seen_ids.insert(key_nid.clone()) {
            nodes.push(Node {
                id: key_nid.clone(),
                label: key.to_string(),
                file_type: "code".to_string(),
                source_file: str_path.to_string(),
                source_location: Some(format!("L{line}")),
            });
        }
        edges.push(Edge {
            source: parent_nid.to_string(),
            target: key_nid.clone(),
            relation: "contains".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
        });

        let val = child.child_by_field_name("value");
        if let Some(val_node) = val {
            match val_node.kind() {
                "object" => {
                    walk_json_object(
                        val_node,
                        source,
                        str_path,
                        stem,
                        file_nid,
                        &key_nid,
                        Some(key),
                        depth + 1,
                        pair_count,
                        nodes,
                        edges,
                        seen_ids,
                    );
                }
                "array" => {
                    // For "extends" arrays: each string element becomes a ref edge
                    let mut ac = val_node.walk();
                    if ac.goto_first_child() {
                        loop {
                            let item = ac.node();
                            if item.kind() == "string" {
                                let content = item.child_by_field_name("string_content");
                                let r = if let Some(c) = content {
                                    read_text(c, source)
                                } else {
                                    read_text(item, source).trim_matches(|c| c == '"' || c == '\'')
                                };
                                if !r.is_empty() {
                                    let ref_nid = make_id(&["ref", r]);
                                    if !ref_nid.is_empty() {
                                        edges.push(Edge {
                                            source: key_nid.clone(),
                                            target: ref_nid,
                                            relation: "extends".to_string(),
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
                            if !ac.goto_next_sibling() {
                                break;
                            }
                        }
                    }
                }
                "string" => {
                    let content = val_node.child_by_field_name("string_content");
                    let val_text = if let Some(c) = content {
                        read_text(c, source)
                    } else {
                        read_text(val_node, source).trim_matches(|c| c == '"' || c == '\'')
                    };
                    if key == "extends" && !val_text.is_empty() {
                        let ref_nid = make_id(&["ref", val_text]);
                        if !ref_nid.is_empty() {
                            edges.push(Edge {
                                source: file_nid.to_string(),
                                target: ref_nid,
                                relation: "extends".to_string(),
                                confidence: "EXTRACTED".to_string(),
                                source_file: str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                weight: 1.0,
                                context: Some("import".to_string()),
                                confidence_score: None,
                            });
                        }
                    } else if key == "$ref" && !val_text.is_empty() {
                        let ref_nid = make_id(&["ref", val_text]);
                        if !ref_nid.is_empty() {
                            edges.push(Edge {
                                source: parent_nid.to_string(),
                                target: ref_nid,
                                relation: "references".to_string(),
                                confidence: "EXTRACTED".to_string(),
                                source_file: str_path.to_string(),
                                source_location: Some(format!("L{line}")),
                                weight: 1.0,
                                context: None,
                                confidence_score: None,
                            });
                        }
                    } else if parent_key.is_some_and(|pk| DEP_KEYS.contains(pk))
                        && !val_text.is_empty()
                    {
                        let dep_nid = make_id1(key);
                        if !dep_nid.is_empty() {
                            edges.push(Edge {
                                source: key_nid.clone(),
                                target: dep_nid,
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
                _ => {}
            }
        }

        if !cur.goto_next_sibling() {
            break;
        }
    }
}
