//! .NET project-file extractors: `.sln`, `.csproj` / `.fsproj` / `.vbproj`, `.razor` / `.cshtml`.
//!
//! Ports `graphify-py/graphify/extract.py::extract_sln`,
//! `extract_csproj`, and `extract_razor`. The Python originals are three
//! discrete top-level helpers; in Rust they're co-located here because they
//! share the same target ecosystem and small helpers.

use std::collections::HashSet;
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader;
use regex::Regex;
use std::sync::LazyLock;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

/// `MSBuild` project files (`.csproj` / `.fsproj` / `.vbproj`) larger than this
/// are skipped with an error. Real-world projects are well under 2 MiB; the
/// cap protects the extractor against accidentally being pointed at a
/// committed binary or a multi-megabyte generated artefact. Matches the
/// literal 2 MiB constant in `graphify-py` `extract.py::extract_csproj`,
/// so the cap is intentionally not configurable — raising or lowering it
/// across the Python/Rust pair belongs in a separate parity-bumping change.
const CSPROJ_MAX_BYTES: u64 = 2_097_152;

/// Text events between an opening element tag and its matching close get
/// routed through this enum when the start tag was a `<TargetFramework>` or
/// `<TargetFrameworks>` element.
enum TextCapture {
    None,
    TargetFramework,
    TargetFrameworks,
}

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

#[allow(clippy::expect_used)]
static RAZOR_USING_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@using\s+([\w.]+)").expect("static razor @using regex"));
#[allow(clippy::expect_used)]
static RAZOR_INJECT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^@inject\s+([\w.<>\[\]]+)\s+(\w+)").expect("static razor @inject regex")
});
#[allow(clippy::expect_used)]
static RAZOR_INHERITS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^@inherits\s+([\w.<>\[\]]+)").expect("static razor @inherits regex")
});
#[allow(clippy::expect_used)]
static RAZOR_MODEL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@model\s+([\w.<>\[\]]+)").expect("static razor @model regex"));
#[allow(clippy::expect_used)]
static RAZOR_PAGE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^@page\s+"([^"]+)""#).expect("static razor @page regex"));
#[allow(clippy::expect_used)]
static RAZOR_COMPONENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<([A-Z][A-Za-z0-9]+)[\s/>]").expect("static razor component regex")
});
#[allow(clippy::expect_used)]
static RAZOR_CODE_BLOCK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)@code\s*\{").expect("static razor @code regex"));
#[allow(clippy::expect_used)]
static RAZOR_METHOD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:public|private|protected|internal|static|async|override|virtual|abstract)\s+[\w<>\[\],\s]+\s+(\w+)\s*\(",
    )
    .expect("static razor method regex")
});

const RAZOR_HTML_TAGS: &[&str] = &[
    "DOCTYPE", "Html", "Head", "Body", "Div", "Span", "Table", "Form", "Input", "Button", "Select",
    "Option", "Label", "Textarea", "Script", "Style", "Link", "Meta", "Title", "Header", "Footer",
    "Nav", "Main", "Section", "Article", "Aside",
];

// ── .sln ────────────────────────────────────────────────────────────────────

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
            });
            edges.push(Edge {
                source: file_nid.clone(),
                target: proj_nid.clone(),
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                weight: 1.0,
                context: None,
                confidence_score: None,
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
                    source: from.clone(),
                    target: to.clone(),
                    relation: "imports".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: None,
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

// ── .csproj / .fsproj / .vbproj ─────────────────────────────────────────────

/// Strip an XML element's namespace prefix so callers can match on the local
/// tag name. Matches Python's `tag.split('}')[1]` pattern.
fn local_name(start: &BytesStart<'_>) -> String {
    let name = start.name();
    let raw = name.as_ref();
    let local = raw
        .iter()
        .rposition(|&b| b == b':')
        .map_or(raw, |i| &raw[i + 1..]);
    String::from_utf8_lossy(local).into_owned()
}

/// Find `attr` on a `BytesStart`, falling back to its lowercased variant —
/// mirrors Python's case-insensitive `Include`/`include` lookup. Returns
/// `None` when neither attribute is present.
fn attr_ci(start: &BytesStart<'_>, attr: &str) -> Option<String> {
    start
        .try_get_attribute(attr)
        .ok()
        .flatten()
        .or_else(|| {
            start
                .try_get_attribute(attr.to_lowercase().as_str())
                .ok()
                .flatten()
        })
        .and_then(|a| {
            std::str::from_utf8(&a.value)
                .ok()
                .map(std::string::ToString::to_string)
        })
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
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                if !root_seen {
                    root_seen = true;
                    root_sdk = attr_ci(e, "Sdk");
                }
                let name = local_name(e);
                match name.as_str() {
                    "TargetFramework" => {
                        capture = TextCapture::TargetFramework;
                    }
                    "TargetFrameworks" => {
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
                            });
                        }
                        edges.push(Edge {
                            source: file_nid.clone(),
                            target: pkg_nid,
                            relation: "imports".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.clone(),
                            source_location: None,
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
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
                            });
                        }
                        edges.push(Edge {
                            source: file_nid.clone(),
                            target: proj_nid,
                            relation: "imports".to_string(),
                            confidence: "EXTRACTED".to_string(),
                            source_file: str_path.clone(),
                            source_location: None,
                            weight: 1.0,
                            context: None,
                            confidence_score: None,
                        });
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(t)) => {
                let text = match t.decode() {
                    Ok(s) => s.into_owned(),
                    Err(_) => continue,
                };
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
            });
            edges.push(Edge {
                source: file_nid.clone(),
                target: sdk_nid,
                relation: "references".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                weight: 1.0,
                context: None,
                confidence_score: None,
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
    });
    edges.push(Edge {
        source: file_nid.to_string(),
        target: fw_nid,
        relation: "references".to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: str_path.to_string(),
        source_location: None,
        weight: 1.0,
        context: None,
        confidence_score: None,
    });
}

// ── .razor / .cshtml ────────────────────────────────────────────────────────

/// Extract directives, component refs, and `@code` methods from a `.razor` /
/// `.cshtml` file. Mirrors `graphify-py` `extract_razor`.
#[must_use]
#[allow(clippy::too_many_lines)] // linear directive dispatch + component scan + @code body parse
pub fn extract_razor(path: &Path) -> FileResult {
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
    }];
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    seen_ids.insert(file_nid.clone());

    let add_ref = |target_name: &str,
                   relation: &str,
                   line: usize,
                   nodes: &mut Vec<Node>,
                   edges: &mut Vec<Edge>,
                   seen_ids: &mut HashSet<String>| {
        let tgt_nid = make_id1(target_name);
        if tgt_nid.is_empty() {
            return;
        }
        if seen_ids.insert(tgt_nid.clone()) {
            nodes.push(Node {
                id: tgt_nid.clone(),
                label: target_name.to_string(),
                file_type: "code".to_string(),
                source_file: str_path.clone(),
                source_location: Some(format!("L{line}")),
            });
        }
        edges.push(Edge {
            source: file_nid.clone(),
            target: tgt_nid,
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.clone(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
        });
    };

    for (idx, line) in src.lines().enumerate() {
        let i = idx + 1;
        if let Some(cap) = RAZOR_USING_RE.captures(line) {
            if let Some(m) = cap.get(1) {
                add_ref(
                    m.as_str(),
                    "imports",
                    i,
                    &mut nodes,
                    &mut edges,
                    &mut seen_ids,
                );
            }
            continue;
        }
        if let Some(cap) = RAZOR_INJECT_RE.captures(line) {
            if let Some(m) = cap.get(1) {
                add_ref(
                    m.as_str(),
                    "imports",
                    i,
                    &mut nodes,
                    &mut edges,
                    &mut seen_ids,
                );
            }
            continue;
        }
        if let Some(cap) = RAZOR_INHERITS_RE.captures(line) {
            if let Some(m) = cap.get(1) {
                add_ref(
                    m.as_str(),
                    "inherits",
                    i,
                    &mut nodes,
                    &mut edges,
                    &mut seen_ids,
                );
            }
            continue;
        }
        if let Some(cap) = RAZOR_MODEL_RE.captures(line) {
            if let Some(m) = cap.get(1) {
                add_ref(
                    m.as_str(),
                    "references",
                    i,
                    &mut nodes,
                    &mut edges,
                    &mut seen_ids,
                );
            }
            continue;
        }
        if let Some(cap) = RAZOR_PAGE_RE.captures(line)
            && let Some(m) = cap.get(1)
        {
            let route = m.as_str();
            let route_nid = make_id(&["route", route]);
            if !route_nid.is_empty() && seen_ids.insert(route_nid.clone()) {
                nodes.push(Node {
                    id: route_nid.clone(),
                    label: format!("route:{route}"),
                    file_type: "concept".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{i}")),
                });
                edges.push(Edge {
                    source: file_nid.clone(),
                    target: route_nid,
                    relation: "references".to_string(),
                    confidence: "EXTRACTED".to_string(),
                    source_file: str_path.clone(),
                    source_location: None,
                    weight: 1.0,
                    context: None,
                    confidence_score: None,
                });
            }
        }
    }

    // Component references: capitalised tag names that aren't known HTML elements.
    for m in RAZOR_COMPONENT_RE.captures_iter(&src) {
        let Some(name_m) = m.get(1) else { continue };
        let comp_name = name_m.as_str();
        if RAZOR_HTML_TAGS.contains(&comp_name) {
            continue;
        }
        let abs_pos = name_m.start();
        let line_num = src[..abs_pos].chars().filter(|&c| c == '\n').count() + 1;
        add_ref(
            comp_name,
            "calls",
            line_num,
            &mut nodes,
            &mut edges,
            &mut seen_ids,
        );
    }

    // @code { ... } method extraction. Find each `@code {` opening, walk
    // braces to locate the matching close, then scan the block body for
    // method declarations. The brace counter intentionally does NOT
    // track string literals or comments — this matches graphify-py's
    // `extract_razor` byte-for-byte. A C# string like `"}{"` inside a
    // method body could in theory confuse the counter, but the regex
    // method scanner below only fires on lines that look like method
    // declarations, so any false `block_end` would just truncate the
    // search range, not produce spurious nodes. Adding lexer state here
    // would diverge from Python parity.
    let stem = file_stem(path);
    let src_bytes = src.as_bytes();
    for cap in RAZOR_CODE_BLOCK_RE.find_iter(&src) {
        let block_start = cap.end();
        let mut depth: i32 = 1;
        let mut pos = block_start;
        while pos < src_bytes.len() && depth > 0 {
            match src_bytes[pos] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            pos += 1;
        }
        let block_end = if depth == 0 { pos - 1 } else { pos };
        if block_end <= block_start {
            continue;
        }
        let block_body = &src[block_start..block_end];
        for mm in RAZOR_METHOD_RE.captures_iter(block_body) {
            let Some(name_m) = mm.get(1) else { continue };
            let method_name = name_m.as_str();
            let abs_pos = block_start + name_m.start();
            let method_line = src[..abs_pos].chars().filter(|&c| c == '\n').count() + 1;
            let method_nid = make_id(&[&stem, method_name]);
            if method_nid.is_empty() {
                continue;
            }
            if seen_ids.insert(method_nid.clone()) {
                nodes.push(Node {
                    id: method_nid.clone(),
                    label: method_name.to_string(),
                    file_type: "code".to_string(),
                    source_file: str_path.clone(),
                    source_location: Some(format!("L{method_line}")),
                });
            }
            edges.push(Edge {
                source: file_nid.clone(),
                target: method_nid,
                relation: "contains".to_string(),
                confidence: "EXTRACTED".to_string(),
                source_file: str_path.clone(),
                source_location: None,
                weight: 1.0,
                context: None,
                confidence_score: None,
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
