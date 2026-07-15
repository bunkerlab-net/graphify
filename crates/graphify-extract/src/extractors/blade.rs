//! Blade (Laravel template) extractor — regex-based.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::ids::make_id1;
use crate::types::{Edge, FileResult, Node};

#[allow(clippy::expect_used)] // literal patterns
static INCLUDE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"@include\(['"]([^'"]+)['"]\)"#).expect("static blade include regex")
});

#[allow(clippy::expect_used)]
static LIVEWIRE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<livewire:([\w.\-]+)").expect("static blade livewire regex"));

#[allow(clippy::expect_used)]
static WIRE_CLICK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"wire:click=["']([^"']+)["']"#).expect("static blade wire-click regex")
});

/// Extract `@include`, `<livewire:>` components, and `wire:click` bindings from Blade templates.
#[must_use]
pub fn extract_blade(path: &Path) -> FileResult {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => return FileResult::error(e.to_string()),
    };

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
        source_location: None,
        metadata: None,
        origin_file: None,
        node_type: None,
    });

    let add_node_edge = |nodes: &mut Vec<Node>,
                         edges: &mut Vec<Edge>,
                         seen_ids: &mut HashSet<String>,
                         nid: String,
                         label: String,
                         relation: &str| {
        if seen_ids.insert(nid.clone()) {
            nodes.push(Node {
                id: nid.clone(),
                label,
                file_type: "code".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                metadata: None,
                origin_file: None,
                node_type: None,
            });
        }
        edges.push(Edge {
            external: false,
            source: file_nid.clone(),
            target: nid,
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.clone(),
            source_location: None,
            weight: 1.0,
            context: None,
            confidence_score: Some(1.0),
            deferred: false,
            metadata: None,
        });
    };

    // @include('path.to.partial')
    for cap in INCLUDE_RE.captures_iter(&src) {
        let partial = cap.get(1).map_or("", |m| m.as_str());
        let tgt = partial.replace('.', "/");
        let tgt_nid = make_id1(&tgt);
        add_node_edge(
            &mut nodes,
            &mut edges,
            &mut seen_ids,
            tgt_nid,
            partial.to_string(),
            "includes",
        );
    }

    // <livewire:component.name>
    for cap in LIVEWIRE_RE.captures_iter(&src) {
        let comp = cap.get(1).map_or("", |m| m.as_str());
        let tgt_nid = make_id1(comp);
        add_node_edge(
            &mut nodes,
            &mut edges,
            &mut seen_ids,
            tgt_nid,
            comp.to_string(),
            "uses_component",
        );
    }

    // wire:click="methodName"
    for cap in WIRE_CLICK_RE.captures_iter(&src) {
        let method = cap.get(1).map_or("", |m| m.as_str());
        let tgt_nid = make_id1(method);
        add_node_edge(
            &mut nodes,
            &mut edges,
            &mut seen_ids,
            tgt_nid,
            method.to_string(),
            "binds_method",
        );
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}
