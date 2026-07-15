//! `.slnx` (XML solution) extractor.

use super::{CSPROJ_MAX_BYTES, attr_ci, local_name};
use crate::ids::make_id1;
use crate::types::{Edge, FileResult, Node};
use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use std::collections::HashSet;
use std::path::Path;

/// Shared mutable state threaded through the `.slnx` streaming parse.
struct SlnxCtx<'a> {
    path: &'a Path,
    str_path: &'a str,
    file_nid: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    project_nids: &'a mut HashSet<String>,
    /// Candidate `(from_nid, to_nid)` build dependencies, filtered against
    /// `project_nids` only after the whole document is parsed (a dependency may
    /// reference a project declared later in the file).
    dep_candidates: &'a mut Vec<(String, String)>,
    /// `from_nid`s of currently open `<Project>` elements, so a nested
    /// `<BuildDependency>` attaches to its nearest enclosing project. An empty
    /// string marks an open `<Project>` without a `Path` (keeps push/pop balanced).
    proj_stack: &'a mut Vec<String>,
}

impl SlnxCtx<'_> {
    /// Resolve a project path relative to the solution file, mirroring the
    /// `.sln` resolver: canonicalise when the target exists, otherwise fall
    /// back to the slash-normalised relative path so ids stay deterministic.
    fn resolve(&self, proj_path: &str) -> String {
        let norm = proj_path.replace('\\', "/");
        self.path
            .parent()
            .map(|p| p.join(&norm))
            .and_then(|p| p.canonicalize().ok())
            .map_or(norm, |p| p.to_string_lossy().into_owned())
    }

    /// Handle one `<Project>` / `<BuildDependency>` element. `has_children` is
    /// `true` for a `Start` tag (a matching `End` will pop the stack) and
    /// `false` for a self-closing `Empty` tag.
    fn on_element(&mut self, e: &BytesStart<'_>, has_children: bool) {
        match local_name(e).as_str() {
            "Project" => {
                let path_attr = attr_ci(e, "Path").filter(|s| !s.is_empty());
                let proj_nid = match &path_attr {
                    Some(proj_path) => {
                        let abs = self.resolve(proj_path);
                        let nid = make_id1(&abs);
                        if !nid.is_empty() {
                            if self.seen_ids.insert(nid.clone()) {
                                let label = Path::new(proj_path).file_stem().map_or_else(
                                    || proj_path.clone(),
                                    |s| s.to_string_lossy().into_owned(),
                                );
                                self.nodes.push(Node {
                                    id: nid.clone(),
                                    label,
                                    file_type: "code".to_string(),
                                    source_file: abs.clone(),
                                    source_location: None,
                                    metadata: None,
                                    origin_file: None,
                                    node_type: None,
                                });
                                self.edges.push(Edge {
                                    external: false,
                                    source: self.file_nid.to_string(),
                                    target: nid.clone(),
                                    relation: "contains".to_string(),
                                    confidence: "EXTRACTED".to_string(),
                                    source_file: self.str_path.to_string(),
                                    source_location: None,
                                    weight: 1.0,
                                    context: None,
                                    confidence_score: None,
                                    deferred: false,
                                    metadata: None,
                                });
                            }
                            self.project_nids.insert(nid.clone());
                        }
                        nid
                    }
                    None => String::new(),
                };
                if has_children {
                    self.proj_stack.push(proj_nid);
                }
            }
            "BuildDependency" => {
                if let Some(dep_path) = attr_ci(e, "Project").filter(|s| !s.is_empty()) {
                    let to_nid = make_id1(&self.resolve(&dep_path));
                    if let Some(from) = self.proj_stack.last()
                        && !from.is_empty()
                        && !to_nid.is_empty()
                        && *from != to_nid
                    {
                        self.dep_candidates.push((from.clone(), to_nid));
                    }
                }
            }
            _ => {}
        }
    }
}

/// Extract project nodes and inter-project build-order dependencies from a
/// `.slnx` file — the XML-based replacement for `.sln`.
///
/// `<Project Path="..."/>` elements (anywhere in the tree, including inside
/// `<Folder>`) become nodes attached to the solution via `contains`;
/// `<BuildDependency Project="..."/>` children become `imports` edges between
/// known projects. Unlike `.sln` there are no GUIDs — projects are identified
/// by their resolved path. Mirrors `graphify-py` `extract_slnx`.
#[must_use]
pub fn extract_slnx(path: &Path) -> FileResult {
    let Ok(bytes) = std::fs::read(path) else {
        return FileResult::error(format!("cannot read {}", path.display()));
    };
    if bytes.len() as u64 > CSPROJ_MAX_BYTES {
        return FileResult::error("project file too large");
    }
    if !crate::extractors::project_xml_is_safe(&bytes) {
        return FileResult::error("refusing XML with DOCTYPE/ENTITY declaration");
    }

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

    let mut project_nids: HashSet<String> = HashSet::new();
    let mut dep_candidates: Vec<(String, String)> = Vec::new();
    let mut proj_stack: Vec<String> = Vec::new();

    let mut reader = Reader::from_reader(&*bytes);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    {
        let mut ctx = SlnxCtx {
            path,
            str_path: &str_path,
            file_nid: &file_nid,
            nodes: &mut nodes,
            edges: &mut edges,
            seen_ids: &mut seen_ids,
            project_nids: &mut project_nids,
            dep_candidates: &mut dep_candidates,
            proj_stack: &mut proj_stack,
        };
        loop {
            match reader.read_event_into(&mut buf) {
                Err(e) => return FileResult::error(format!("XML parse error: {e}")),
                Ok(Event::Eof) => break,
                Ok(Event::Start(ref e)) => ctx.on_element(e, true),
                Ok(Event::Empty(ref e)) => ctx.on_element(e, false),
                Ok(Event::End(ref e)) => {
                    // `BytesEnd` is a distinct type from `BytesStart`, so strip
                    // the namespace prefix inline rather than via `local_name`.
                    let name = e.name();
                    let raw = name.as_ref();
                    let local = raw
                        .iter()
                        .rposition(|&b| b == b':')
                        .map_or(raw, |i| &raw[i + 1..]);
                    if local == b"Project" {
                        ctx.proj_stack.pop();
                    }
                }
                _ => {}
            }
            buf.clear();
        }
    }

    // Build-order dependencies between known projects.
    for (from, to) in dep_candidates {
        if project_nids.contains(&to) {
            edges.push(Edge {
                external: false,
                source: from,
                target: to,
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

    FileResult {
        nodes,
        edges,
        raw_calls: Vec::new(),
        error: None,
    }
}

// ── .csproj / .fsproj / .vbproj ─────────────────────────────────────────────
