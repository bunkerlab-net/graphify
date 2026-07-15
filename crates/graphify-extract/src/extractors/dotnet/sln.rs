//! `.sln` solution-file extractor.

use crate::ids::make_id1;
use crate::types::{Edge, FileResult, Node};
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

#[allow(clippy::expect_used)] // literal pattern; build cannot fail
static SLN_PROJECT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"Project\("[^"]*"\)\s*=\s*"([^"]+)"\s*,\s*"([^"]+)"\s*,\s*"([^"]*)""#)
        .expect("static sln project regex")
});

#[allow(clippy::expect_used)]
static SLN_DEP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\{([0-9a-fA-F-]+)\}\s*=\s*\{([0-9a-fA-F-]+)\}")
        .expect("static sln dependency regex")
});

#[allow(clippy::expect_used)]
static SLN_PROJECT_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"Project\("[^"]*"\)\s*=\s*"[^"]+"\s*,\s*"[^"]+"\s*,\s*"\{([^}]+)\}""#)
        .expect("static sln project-line regex")
});

/// Extract project nodes and inter-project dependency edges from a `.sln` file.
///
/// Each `Project(...) = ...` block becomes a node attached to the solution
/// file via `contains`; `ProjectSection(ProjectDependencies)` entries become
/// `imports` edges between projects identified by GUID. Mirrors
/// `graphify-py` `extract_sln`.
#[must_use]
#[allow(clippy::too_many_lines)] // two linear passes over .sln plus node/edge bookkeeping
pub fn extract_sln(path: &Path) -> FileResult {
    let Ok(src) = std::fs::read_to_string(path) else {
        return FileResult::error(format!("cannot read {}", path.display()));
    };

    let str_path = path.to_string_lossy().into_owned();
    let file_nid = make_id1(&str_path);

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
        node_type: None,
    }];
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    seen_ids.insert(file_nid.clone());

    let mut guid_to_nid: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for cap in SLN_PROJECT_RE.captures_iter(&src) {
        let proj_name = cap.get(1).map_or("", |m| m.as_str()).to_string();
        let proj_path = cap.get(2).map_or("", |m| m.as_str()).replace('\\', "/");
        let proj_guid = cap
            .get(3)
            .map_or("", |m| m.as_str())
            .trim_matches(|c| c == '{' || c == '}')
            .to_string();

        let abs_proj = path
            .parent()
            .map(|p| p.join(&proj_path))
            .and_then(|p| p.canonicalize().ok())
            .map_or_else(|| proj_path.clone(), |p| p.to_string_lossy().into_owned());
        let proj_nid = make_id1(&abs_proj);
        if !proj_nid.is_empty() && seen_ids.insert(proj_nid.clone()) {
            nodes.push(Node {
                id: proj_nid.clone(),
                label: proj_name,
                file_type: "code".to_string(),
                source_file: abs_proj.clone(),
                source_location: None,
                metadata: None,
                origin_file: None,
                node_type: None,
            });
            edges.push(Edge {
                external: false,
                source: file_nid.clone(),
                target: proj_nid.clone(),
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                weight: 1.0,
                context: None,
                confidence_score: None,
                deferred: false,
                metadata: None,
            });
        }
        if !proj_guid.is_empty() {
            guid_to_nid.insert(proj_guid.to_lowercase(), proj_nid);
        }
    }

    // Second pass: project-dependency sections. Each block is nested inside
    // a Project(...)/EndProject pair so we track the currently open project's
    // GUID and emit `imports` edges to each declared dependency.
    let mut in_dep_section = false;
    let mut current_proj_guid: Option<String> = None;
    for line in src.lines() {
        if let Some(cap) = SLN_PROJECT_LINE_RE.captures(line) {
            current_proj_guid = cap.get(1).map(|m| m.as_str().to_lowercase());
            continue;
        }
        if line.trim() == "EndProject" {
            current_proj_guid = None;
            continue;
        }
        if line.contains("ProjectSection(ProjectDependencies)") {
            in_dep_section = true;
            continue;
        }
        if in_dep_section && line.contains("EndProjectSection") {
            in_dep_section = false;
            continue;
        }
        if in_dep_section
            && let Some(ref from_guid) = current_proj_guid
            && let Some(dep_cap) = SLN_DEP_RE.captures(line)
        {
            let to_guid = dep_cap.get(1).map_or("", |m| m.as_str()).to_lowercase();
            let from_nid = guid_to_nid.get(from_guid);
            let to_nid = guid_to_nid.get(&to_guid);
            if let (Some(from), Some(to)) = (from_nid, to_nid)
                && from != to
            {
                edges.push(Edge {
                    external: false,
                    source: from.clone(),
                    target: to.clone(),
                    relation: "imports".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: None,
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
        raw_calls: Vec::new(),
        error: None,
    }
}

// ── .slnx ─────────────────────────────────────────────────────────────────
