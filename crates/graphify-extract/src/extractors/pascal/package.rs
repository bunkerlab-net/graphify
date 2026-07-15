//! Lazarus `.lpk` XML package-metadata extractor.

use super::pascal_resolve_unit;
use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;

/// Extract package metadata from a Lazarus `.lpk` package file (XML).
#[must_use]
#[allow(clippy::too_many_lines, clippy::missing_panics_doc)]
pub fn extract_lazarus_package(path: &Path) -> FileResult {
    // Check the on-disk size before reading so an oversized file can't force a
    // multi-megabyte allocation just to be rejected.
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > crate::extractors::PROJECT_XML_MAX_BYTES => {
            return FileResult::error("package file too large");
        }
        Ok(_) => {}
        Err(e) => return FileResult::error(e.to_string()),
    }
    let raw = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return FileResult::error(e.to_string()),
    };
    if raw.len() as u64 > crate::extractors::PROJECT_XML_MAX_BYTES {
        return FileResult::error("package file too large");
    }
    if !crate::extractors::project_xml_is_safe(&raw) {
        return FileResult::error("refusing XML with DOCTYPE/ENTITY declaration");
    }
    let text = String::from_utf8_lossy(&raw).into_owned();

    let str_path = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    let make_node = |nid: &str, label: &str, str_path: &str| -> Node {
        Node {
            id: nid.to_string(),
            label: label.to_string(),
            file_type: "code".to_string(),
            source_file: str_path.to_string(),
            source_location: Some("L1".to_string()),
            metadata: None,
            origin_file: None,
            node_type: None,
        }
    };

    let file_nid = make_id1(&str_path);
    seen_ids.insert(file_nid.clone());
    nodes.push(make_node(
        &file_nid,
        &path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        &str_path,
    ));

    // Simple XML parse for .lpk using regex (avoid pulling in an XML crate)
    #[allow(clippy::expect_used)]
    let pkg_name_re =
        Regex::new(r#"(?i)<Name\s+Value="([^"]+)""#).expect("literal pattern is valid");
    #[allow(clippy::expect_used)]
    let dep_name_re =
        Regex::new(r#"(?i)<PackageName\s+Value="([^"]+)""#).expect("literal pattern is valid");
    #[allow(clippy::expect_used)]
    let unit_name_re =
        Regex::new(r#"(?i)<UnitName\s+Value="([^"]+)""#).expect("literal pattern is valid");

    let pkg_name = pkg_name_re
        .captures(&text)
        .and_then(|c| c.get(1))
        .map_or_else(
            || {
                path.file_stem()
                    .map_or("unknown".to_string(), |s| s.to_string_lossy().into_owned())
            },
            |m| m.as_str().to_string(),
        );
    let pkg_nid = make_id(&[&stem, &pkg_name]);
    if seen_ids.insert(pkg_nid.clone()) {
        nodes.push(make_node(&pkg_nid, &pkg_name, &str_path));
    }
    edges.push(Edge {
        external: false,
        source: file_nid.clone(),
        target: pkg_nid.clone(),
        relation: "contains".to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        weight: 1.0,
        context: None,
        confidence_score: None,
        deferred: false,
        metadata: None,
    });

    // Required packages → imports edges
    for cap in dep_name_re.captures_iter(&text) {
        let dep_name = cap.get(1).map_or("", |m| m.as_str());
        if dep_name.is_empty() {
            continue;
        }
        let dep_nid = make_id1(dep_name);
        if seen_ids.insert(dep_nid.clone()) {
            nodes.push(make_node(&dep_nid, dep_name, &str_path));
        }
        edges.push(Edge {
            external: false,
            source: pkg_nid.clone(),
            target: dep_nid,
            relation: "imports".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.clone(),
            source_location: Some("L1".to_string()),
            weight: 1.0,
            context: Some("import".to_string()),
            confidence_score: None,
            deferred: false,
            metadata: None,
        });
    }

    // Listed units → contains edges
    for cap in unit_name_re.captures_iter(&text) {
        let unit_name = cap.get(1).map_or("", |m| m.as_str());
        if unit_name.is_empty() {
            continue;
        }
        let unit_nid = pascal_resolve_unit(path, unit_name);
        if seen_ids.insert(unit_nid.clone()) {
            nodes.push(make_node(&unit_nid, unit_name, &str_path));
        }
        edges.push(Edge {
            external: false,
            source: pkg_nid.clone(),
            target: unit_nid,
            relation: "contains".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.clone(),
            source_location: Some("L1".to_string()),
            weight: 1.0,
            context: None,
            confidence_score: None,
            deferred: false,
            metadata: None,
        });
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}
