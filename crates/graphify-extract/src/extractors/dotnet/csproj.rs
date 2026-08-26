//! `.csproj` / `.fsproj` / `.vbproj` `MSBuild` project-file extractor.

use super::{CSPROJ_MAX_BYTES, attr_ci};
use crate::ids::{make_id, make_id1};
use crate::types::{Edge, FileResult, Node};
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::collections::HashSet;
use std::path::Path;

/// Text events between an opening element tag and its matching close get
/// routed through this enum when the start tag was a `<TargetFramework>` or
/// `<TargetFrameworks>` element.
enum TextCapture {
    None,
    TargetFramework,
    TargetFrameworks,
}

/// Extract packages, project references, target frameworks, and SDK from an
/// `MSBuild` project file (`.csproj` / `.fsproj` / `.vbproj`). Mirrors
/// `graphify-py` `extract_csproj`.
#[must_use]
#[allow(clippy::too_many_lines)] // linear element dispatch, hard to split without losing locality
pub fn extract_csproj(path: &Path) -> FileResult {
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

    let mut reader = Reader::from_reader(&*bytes);
    reader.config_mut().trim_text(true);

    // Root-level SDK attribute (read on the first encountered start tag).
    let mut root_sdk: Option<String> = None;
    let mut root_seen = false;

    // Track text content of `<TargetFramework>` / `<TargetFrameworks>` since
    // quick-xml delivers text in a separate event.
    let mut capture = TextCapture::None;

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Err(e) => {
                return FileResult::error(format!("XML parse error: {e}"));
            }
            Ok(Event::Eof) => break,
            Ok(start @ (Event::Start(_) | Event::Empty(_))) => {
                // A self-closing `<TargetFramework/>` (an `Empty` event) carries no
                // text, so only a real open tag arms the capture; otherwise the flag
                // would dangle and misattribute the next element's text. graphify-py
                // reads `tf.text` (None for self-closing tags), so this matches it.
                let is_empty = matches!(start, Event::Empty(_));
                let (Event::Start(e) | Event::Empty(e)) = &start else {
                    continue;
                };
                if !root_seen {
                    root_seen = true;
                    root_sdk = attr_ci(e, "Sdk");
                }
                let name = e.local_name().into_inner();
                match name {
                    "TargetFramework" if !is_empty => {
                        capture = TextCapture::TargetFramework;
                    }
                    "TargetFrameworks" if !is_empty => {
                        capture = TextCapture::TargetFrameworks;
                    }
                    "PackageReference" => {
                        let Some(pkg_name) = attr_ci(e, "Include") else {
                            continue;
                        };
                        let version = attr_ci(e, "Version").unwrap_or_default();
                        let pkg_nid = make_id(&["nuget", &pkg_name]);
                        if pkg_nid.is_empty() {
                            continue;
                        }
                        let label = if version.is_empty() {
                            pkg_name.clone()
                        } else {
                            format!("{pkg_name} ({version})")
                        };
                        if seen_ids.insert(pkg_nid.clone()) {
                            nodes.push(Node {
                                id: pkg_nid.clone(),
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
                            target: pkg_nid,
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
                    "ProjectReference" => {
                        let Some(ref_path) = attr_ci(e, "Include") else {
                            continue;
                        };
                        let ref_norm = ref_path.replace('\\', "/");
                        let abs_ref = path
                            .parent()
                            .map(|p| p.join(&ref_norm))
                            .and_then(|p| p.canonicalize().ok())
                            .map_or_else(|| ref_norm.clone(), |p| p.to_string_lossy().into_owned());
                        let proj_nid = make_id1(&abs_ref);
                        if proj_nid.is_empty() {
                            continue;
                        }
                        let proj_label = Path::new(&ref_norm)
                            .file_name()
                            .map_or_else(|| ref_norm.clone(), |n| n.to_string_lossy().into_owned());
                        if seen_ids.insert(proj_nid.clone()) {
                            nodes.push(Node {
                                id: proj_nid.clone(),
                                label: proj_label,
                                file_type: "code".to_string(),
                                source_file: abs_ref,
                                source_location: None,
                                metadata: None,
                                origin_file: None,
                                node_type: None,
                            });
                        }
                        edges.push(Edge {
                            external: false,
                            source: file_nid.clone(),
                            target: proj_nid,
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
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let text = &*t;
                match capture {
                    TextCapture::TargetFramework => {
                        let fw = text.trim().to_string();
                        if !fw.is_empty() {
                            add_framework_node(
                                &fw,
                                &str_path,
                                &file_nid,
                                &mut nodes,
                                &mut edges,
                                &mut seen_ids,
                            );
                        }
                    }
                    TextCapture::TargetFrameworks => {
                        for fw_raw in text.trim().split(';') {
                            let fw = fw_raw.trim();
                            if !fw.is_empty() {
                                add_framework_node(
                                    fw,
                                    &str_path,
                                    &file_nid,
                                    &mut nodes,
                                    &mut edges,
                                    &mut seen_ids,
                                );
                            }
                        }
                    }
                    TextCapture::None => {}
                }
                capture = TextCapture::None;
            }
            Ok(Event::End(_)) => {
                capture = TextCapture::None;
            }
            _ => {}
        }
        buf.clear();
    }

    if let Some(sdk) = root_sdk
        && !sdk.is_empty()
    {
        let sdk_nid = make_id(&["sdk", &sdk]);
        if !sdk_nid.is_empty() && seen_ids.insert(sdk_nid.clone()) {
            nodes.push(Node {
                id: sdk_nid.clone(),
                label: sdk,
                file_type: "concept".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                metadata: None,
                origin_file: None,
                node_type: None,
            });
            edges.push(Edge {
                external: false,
                source: file_nid.clone(),
                target: sdk_nid,
                relation: "references".to_string(),
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

fn add_framework_node(
    fw: &str,
    str_path: &str,
    file_nid: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
    seen_ids: &mut HashSet<String>,
) {
    let fw_nid = make_id(&["framework", fw]);
    if fw_nid.is_empty() || !seen_ids.insert(fw_nid.clone()) {
        return;
    }
    nodes.push(Node {
        id: fw_nid.clone(),
        label: fw.to_string(),
        file_type: "concept".to_string(),
        source_file: str_path.to_string(),
        source_location: None,
        metadata: None,
        origin_file: None,
        node_type: None,
    });
    edges.push(Edge {
        external: false,
        source: file_nid.to_string(),
        target: fw_nid,
        relation: "references".to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: str_path.to_string(),
        source_location: None,
        weight: 1.0,
        context: None,
        confidence_score: None,
        deferred: false,
        metadata: None,
    });
}

// ── .razor / .cshtml ────────────────────────────────────────────────────────
