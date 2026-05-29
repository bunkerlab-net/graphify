//! Dart extractor — regex-based (no tree-sitter-dart on crates.io).

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

static DART_SKIP: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    ["if", "for", "while", "switch", "catch", "return"]
        .into_iter()
        .collect()
});

#[allow(clippy::expect_used)] // literal patterns
static CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:abstract\s+)?(?:class|mixin)\s+(\w+)").expect("static dart class regex")
});

#[allow(clippy::expect_used)]
static FUNC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*(?:static\s+|async\s+)?(?:\w+\s+)+(\w+)\s*\(")
        .expect("static dart func regex")
});

#[allow(clippy::expect_used)]
static IMPORT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^import\s+['"]([^'"]+)['"]"#).expect("static dart import regex")
});

/// Extract classes, mixins, functions, and imports from a `.dart` file using regex.
#[must_use]
// Single-pass regex extractor: the file/class/function/import passes share the
// `defined`/`nodes`/`edges` accumulators, so splitting would fragment that state.
#[allow(clippy::too_many_lines)]
pub fn extract_dart(path: &Path) -> FileResult {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return FileResult::error(e.to_string()),
    };

    let str_path = path.to_string_lossy().into_owned();
    // Use the stem (not str(path)) for child IDs to keep them machine-independent.
    let stem = file_stem(path);

    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut defined: HashSet<String> = HashSet::new();

    let file_nid = make_id1(&str_path);
    defined.insert(file_nid.clone());
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

    // Classes and mixins
    for cap in CLASS_RE.captures_iter(&src) {
        let name = cap.get(1).map_or("", |m| m.as_str());
        let nid = make_id(&[&stem, name]);
        if defined.insert(nid.clone()) {
            nodes.push(Node {
                id: nid.clone(),
                label: name.to_string(),
                file_type: "code".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                metadata: None,
            });
            edges.push(Edge {
                external: false,
                source: file_nid.clone(),
                target: nid,
                relation: "defines".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                weight: 1.0,
                context: None,
                confidence_score: Some(1.0),
            });
        }
    }

    // Functions / methods
    for cap in FUNC_RE.captures_iter(&src) {
        let name = cap.get(1).map_or("", |m| m.as_str());
        if DART_SKIP.contains(name) {
            continue;
        }
        let nid = make_id(&[&stem, name]);
        if defined.insert(nid.clone()) {
            nodes.push(Node {
                id: nid.clone(),
                label: name.to_string(),
                file_type: "code".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                metadata: None,
            });
            edges.push(Edge {
                external: false,
                source: file_nid.clone(),
                target: nid,
                relation: "defines".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                weight: 1.0,
                context: None,
                confidence_score: Some(1.0),
            });
        }
    }

    // Imports
    for cap in IMPORT_RE.captures_iter(&src) {
        let pkg = cap.get(1).map_or("", |m| m.as_str());
        let tgt_nid = make_id1(pkg);
        if defined.insert(tgt_nid.clone()) {
            nodes.push(Node {
                id: tgt_nid.clone(),
                label: pkg.to_string(),
                file_type: "code".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                metadata: None,
            });
        }
        edges.push(Edge {
            external: false,
            source: file_nid.clone(),
            target: tgt_nid,
            relation: "imports".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.clone(),
            source_location: None,
            weight: 1.0,
            context: None,
            confidence_score: Some(1.0),
        });
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}
