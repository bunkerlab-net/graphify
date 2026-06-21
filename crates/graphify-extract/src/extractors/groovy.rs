//! Groovy / Gradle extractor with Spock-test regex fallback.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::generic::extract_generic;
use crate::lang_configs;
use crate::types::FileResult;

#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:[\w@]+\s+)*class\s+(\w+)").expect("static spock class regex")
});
#[allow(clippy::expect_used)]
static FEATURE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^\s*def\s+(?:"([^"]+)"|'([^']+)')\s*\("#).expect("static spock feature regex")
});
#[allow(clippy::expect_used)]
static PLAIN_METHOD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*def\s+(\w+)\s*\(").expect("static spock method regex"));
static SPOCK_KWS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["if", "while", "for", "switch", "catch"]
        .into_iter()
        .collect()
});

/// Extract classes, methods, constructors, and imports from a `.groovy`/`.gradle` file.
/// Falls back to regex-based Spock extractor when needed.
#[must_use]
pub fn extract_groovy(path: &Path) -> FileResult {
    let result = extract_generic(path, &lang_configs::GROOVY);
    if is_spock_file(path) {
        extract_spock_fallback(path, result)
    } else {
        result
    }
}

/// Return `true` if the Groovy file contains Spock-style `def "feature"()` test methods.
///
/// Spock test methods use quoted string names that the generic tree-sitter extractor misses;
/// this heuristic triggers the regex fallback when any line starts with `def "` or `def '`.
fn is_spock_file(path: &Path) -> bool {
    let Ok(src) = std::fs::read_to_string(path) else {
        return false;
    };
    // Check for `def "feature"()` patterns
    src.lines().any(|l| {
        let t = l.trim();
        t.starts_with("def \"") || t.starts_with("def '")
    })
}

/// Extract class and method nodes from a Spock test file using regex scanning.
///
/// The generic tree-sitter pass already ran (`ts_result`) but cannot handle Spock's quoted
/// method names. This function discards the tree-sitter node/method edges, keeps the file
/// node and import edges, then re-scans line-by-line with three regexes:
/// `class`, `def "feature"()`, and `def plainMethod()`. Mirrors Python `_extract_spock_fallback`.
#[allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
// ↑ literal regex patterns; function is a direct port; row→u32 is safe
fn extract_spock_fallback(path: &Path, ts_result: FileResult) -> FileResult {
    use crate::ids::{file_stem, make_id, make_id1};
    use crate::types::{Edge, Node};

    let Ok(source) = std::fs::read_to_string(path) else {
        return ts_result;
    };
    let str_path = path.to_string_lossy().into_owned();
    let stem = file_stem(path);

    // Keep file node + import edges from tree-sitter pass
    let file_node = ts_result
        .nodes
        .iter()
        .find(|n| {
            path.file_name()
                .is_some_and(|f| f.to_string_lossy() == n.label)
        })
        .cloned();
    let mut nodes: Vec<Node> = file_node.into_iter().collect();
    let mut edges: Vec<Edge> = ts_result
        .edges
        .into_iter()
        .filter(|e| e.context.as_deref() == Some("import"))
        .collect();
    let mut seen_ids: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();

    let file_nid = make_id1(&str_path);
    if !seen_ids.contains(&file_nid) {
        nodes.push(Node {
            id: file_nid.clone(),
            label: path
                .file_name()
                .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
            file_type: "code".to_string(),
            source_file: str_path.clone(),
            source_location: Some("L1".to_string()),
            metadata: None,
        });
        seen_ids.insert(file_nid.clone());
    }

    let mut current_class_nid: Option<String> = None;

    for (lineno, line) in source.lines().enumerate() {
        let lineno = lineno + 1;
        if let Some(cap) = CLASS_RE.captures(line) {
            let class_name = cap.get(1).map_or("", |m| m.as_str());
            let class_nid = make_id(&[&stem, class_name]);
            if !seen_ids.contains(&class_nid) {
                seen_ids.insert(class_nid.clone());
                nodes.push(Node {
                    id: class_nid.clone(),
                    label: class_name.to_string(),
                    file_type: "code".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{lineno}")),
                    metadata: None,
                });
            }
            edges.push(Edge {
                external: false,
                source: file_nid.clone(),
                target: class_nid.clone(),
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: Some(format!("L{lineno}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
            current_class_nid = Some(class_nid);
            continue;
        }

        let Some(ref class_nid) = current_class_nid else {
            continue;
        };

        if let Some(cap) = FEATURE_RE.captures(line) {
            let method_name = cap.get(1).or_else(|| cap.get(2)).map_or("", |m| m.as_str());
            let method_label = format!("\"{method_name}\"");
            let method_nid = make_id(&[class_nid, method_name]);
            if !seen_ids.contains(&method_nid) {
                seen_ids.insert(method_nid.clone());
                nodes.push(Node {
                    id: method_nid.clone(),
                    label: method_label,
                    file_type: "code".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{lineno}")),
                    metadata: None,
                });
            }
            edges.push(Edge {
                external: false,
                source: class_nid.clone(),
                target: method_nid,
                relation: "method".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: Some(format!("L{lineno}")),
                weight: 1.0,
                context: None,
                confidence_score: None,
            });
            continue;
        }

        if let Some(cap) = PLAIN_METHOD_RE.captures(line) {
            let method_name = cap.get(1).map_or("", |m| m.as_str());
            if !SPOCK_KWS.contains(method_name) {
                let method_label = format!(".{method_name}()");
                let method_nid = make_id(&[class_nid, method_name]);
                if !seen_ids.contains(&method_nid) {
                    seen_ids.insert(method_nid.clone());
                    nodes.push(Node {
                        id: method_nid.clone(),
                        label: method_label,
                        file_type: "code".to_string(),
                        source_file: str_path.clone(),
                        source_location: Some(format!("L{lineno}")),
                        metadata: None,
                    });
                }
                edges.push(Edge {
                    external: false,
                    source: class_nid.clone(),
                    target: method_nid,
                    relation: "method".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{lineno}")),
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
        raw_calls: Vec::new(),
        error: None,
    }
}
