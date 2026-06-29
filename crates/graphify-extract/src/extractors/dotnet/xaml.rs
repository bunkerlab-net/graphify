//! WPF/XAML structural extractor (#1460, #1473).
//!
//! Mirrors `graphify-py/graphify/extract.py::extract_xaml` and its helpers.
//! Extracts the root element, `x:Class`, named controls + their control types,
//! `{Binding}` paths/commands/converters, bridges the view to its `.xaml.cs`
//! code-behind by resolving event-handler attributes to the matching methods
//! (gated on the .NET handler signature), and resolves the view to its
//! ViewModel via an explicit `DataContext`, a design-time `d:DesignInstance`,
//! the `View`→`ViewModel` naming convention, or Prism `AutoWireViewModel`. Also
//! surfaces CommunityToolkit `[ObservableProperty]`/`[RelayCommand]` generated
//! members. Uses stdlib XML (`quick-xml`) with the same size/DOCTYPE guards as
//! the `.csproj` extractor.

// WPF/XAML domain terms (ViewModel, CommunityToolkit, DataContext, …) read
// naturally in prose; backticking every mention hurts readability here.
#![allow(clippy::doc_markdown)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use quick_xml::events::Event;
use quick_xml::reader::Reader;
use regex::Regex;

use super::CSPROJ_MAX_BYTES;
use crate::extractors::extract_csharp;
use crate::ids::{make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

thread_local! {
    /// The extraction-root boundary for the in-progress `extract()` call, set by
    /// [`with_xaml_extract_root`]. Bounds the ViewModel project-root scan so it
    /// never escapes the corpus the user asked to extract. `None` when
    /// `extract_xaml` is called directly (no surrounding pipeline).
    static XAML_ACTIVE_EXTRACT_ROOT: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// Per-root cache of `ViewModel` class nodes, so a multi-`.xaml` project
    /// scans its `.cs` files once. Cleared implicitly per thread / pipeline run.
    static XAML_CSHARP_CLASS_CACHE: RefCell<HashMap<String, HashMap<String, Vec<Node>>>> =
        RefCell::new(HashMap::new());
}

/// Run `f` with the XAML extract-root boundary set to `root` (resolved), then
/// restore the previous value. Mirrors Python `_safe_extract_with_xaml_root`.
pub(crate) fn with_xaml_extract_root<R>(root: Option<&Path>, f: impl FnOnce() -> R) -> R {
    let resolved = root.map(|r| r.canonicalize().unwrap_or_else(|_| r.to_path_buf()));
    let prev = XAML_ACTIVE_EXTRACT_ROOT.with(|c| c.replace(resolved));
    let result = f();
    XAML_ACTIVE_EXTRACT_ROOT.with(|c| *c.borrow_mut() = prev);
    result
}

fn active_extract_root() -> Option<PathBuf> {
    XAML_ACTIVE_EXTRACT_ROOT.with(|c| c.borrow().clone())
}

#[allow(clippy::expect_used)] // literal patterns; compile is infallible
mod re {
    use super::{LazyLock, Regex};
    /// A .NET event handler has the signature `(object sender, <T>EventArgs e)`.
    pub static EVENT_HANDLER_SIGNATURE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\(\s*object\??\s+\w+\s*,\s*[\w.]*EventArgs(?:<[^>]*>)?\s+\w+\s*\)")
            .expect("event handler signature regex")
    });
    /// A bare identifier (anchored: a "fullmatch" of a method/handler name).
    pub static IDENT_FULL: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^[A-Za-z_]\w*$").expect("ident regex"));
    /// `Type=…` inside a `{d:DesignInstance Type=…}` markup value.
    pub static DESIGN_INSTANCE_TYPE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\bType\s*=\s*(?:\{x:Type\s+)?([\w.:+]+)").expect("design instance regex")
    });
    /// CommunityToolkit field declaration: captures the backing field name.
    pub static TOOLKIT_FIELD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(_?m?_?[A-Za-z_]\w*)\s*(?:=.*)?;").expect("toolkit field regex")
    });
    /// CommunityToolkit method declaration: captures the method name.
    pub static TOOLKIT_METHOD: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"\b([A-Za-z_]\w*)\s*\(").expect("toolkit method regex"));
}

/// XAML attribute names that carry free-form strings and never name an event
/// handler — skipped when matching attribute values to code-behind methods.
static NON_EVENT_ATTRS: &[&str] = &[
    "Name",
    "Content",
    "Text",
    "Title",
    "Tag",
    "ToolTip",
    "Header",
    "Class",
    "Key",
    "Uid",
    "DataContext",
    "Style",
    "Source",
];

/// A parsed XAML element: local tag name, `(local_attr, value)` pairs, and child
/// element indices into the flat DOM vec (mirrors `ElementTree` `iter()`/`list`).
struct XamlElem {
    tag: String,
    attrs: Vec<(String, String)>,
    children: Vec<usize>,
}

/// Local name of an XML name: the segment after the last `:` (quick-xml keeps the
/// `prefix:local` form). Mirrors Python `_xml_local_name` on `{ns}local`.
fn local_name(raw: &[u8]) -> String {
    let local = raw
        .iter()
        .rposition(|&b| b == b':')
        .map_or(raw, |i| &raw[i + 1..]);
    String::from_utf8_lossy(local).into_owned()
}

/// Parse the XAML into a flat element vec; returns `(elems, root_index)`.
fn parse_dom(bytes: &[u8]) -> Option<(Vec<XamlElem>, usize)> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);
    let mut elems: Vec<XamlElem> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut root: Option<usize> = None;
    let mut buf = Vec::new();
    loop {
        let event = reader.read_event_into(&mut buf);
        let start = match &event {
            Ok(Event::Start(e) | Event::Empty(e)) => e,
            Ok(Event::End(_)) => {
                stack.pop();
                buf.clear();
                continue;
            }
            Ok(Event::Eof) => break,
            Ok(_) => {
                buf.clear();
                continue;
            }
            Err(_) => return None,
        };
        let attrs = start
            .attributes()
            .filter_map(Result::ok)
            .map(|a| {
                let key = local_name(a.key.as_ref());
                let value = a
                    .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .map(std::borrow::Cow::into_owned)
                    .unwrap_or_default();
                (key, value)
            })
            .collect();
        let idx = elems.len();
        elems.push(XamlElem {
            tag: local_name(start.name().as_ref()),
            attrs,
            children: Vec::new(),
        });
        if let Some(&parent) = stack.last() {
            elems[parent].children.push(idx);
        } else if root.is_none() {
            root = Some(idx);
        }
        if matches!(event, Ok(Event::Start(_))) {
            stack.push(idx);
        }
        buf.clear();
    }
    root.map(|r| (elems, r))
}

// ── markup-extension helpers ────────────────────────────────────────────────

/// Parse `{Name args}` markup → `(name, args)`; `None` when not a markup value.
fn markup_extension(value: &str) -> Option<(String, String)> {
    let value = value.trim();
    if !(value.starts_with('{') && value.ends_with('}')) {
        return None;
    }
    let inner = value[1..value.len() - 1].trim();
    if inner.is_empty() || inner.starts_with('}') {
        return None;
    }
    let (name, args) = inner.split_once(' ').unwrap_or((inner, ""));
    Some((name.to_string(), args.trim().to_string()))
}

/// Split markup args on top-level commas (respecting nested `{...}`).
fn split_markup_args(args: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    for (idx, ch) in args.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            ',' if depth == 0 => {
                parts.push(args[start..idx].trim().to_string());
                start = idx + 1;
            }
            _ => {}
        }
    }
    let tail = args[start..].trim();
    if !tail.is_empty() {
        parts.push(tail.to_string());
    }
    parts
}

/// Resource key of a `{StaticResource Key}` markup value, if any.
fn static_resource_key(value: &str) -> Option<String> {
    let (name, args) = markup_extension(value)?;
    if name != "StaticResource" {
        return None;
    }
    for part in split_markup_args(&args) {
        match part.split_once('=') {
            None => return Some(part).filter(|p| !p.is_empty()),
            Some((key, resource)) if key.trim() == "ResourceKey" => {
                let r = resource.trim();
                return (!r.is_empty()).then(|| r.to_string());
            }
            Some(_) => {}
        }
    }
    None
}

/// `(binding_path, converter_key)` of a `{Binding …}` markup value.
fn binding_refs(value: &str) -> (Option<String>, Option<String>) {
    let Some((name, args)) = markup_extension(value) else {
        return (None, None);
    };
    if name != "Binding" {
        return (None, None);
    }
    let mut path_ref: Option<String> = None;
    let mut converter_ref: Option<String> = None;
    for part in split_markup_args(&args) {
        if part.is_empty() {
            continue;
        }
        match part.split_once('=') {
            None => {
                if path_ref.is_none() {
                    path_ref = Some(part);
                }
            }
            Some((key, raw_value)) => {
                let (key, raw_value) = (key.trim(), raw_value.trim());
                if key == "Path" {
                    path_ref = Some(raw_value.to_string());
                } else if key == "Converter" {
                    converter_ref = static_resource_key(raw_value);
                }
            }
        }
    }
    if let Some(p) = &path_ref
        && (p.contains('{') || p.contains('}'))
    {
        path_ref = None;
    }
    (
        path_ref.filter(|p| !p.is_empty()),
        converter_ref.filter(|c| !c.is_empty()),
    )
}

/// Simple (unqualified) type name from a `vm:Type` / `Ns.Type` / `x:Type Foo`
/// reference, or `None` when it isn't a bare identifier.
fn type_simple_name(type_ref: &str) -> Option<String> {
    let mut t = type_ref.trim().trim_matches(|c| c == '{' || c == '}');
    t = t.split(',').next().unwrap_or(t).trim();
    if let Some(rest) = t.strip_prefix("x:Type ") {
        t = rest.trim();
    }
    if let Some(i) = t.rfind(':') {
        t = &t[i + 1..];
    }
    if let Some(i) = t.rfind('.') {
        t = &t[i + 1..];
    }
    if let Some(i) = t.rfind('+') {
        t = &t[i + 1..];
    }
    re::IDENT_FULL.is_match(t).then(|| t.to_string())
}

// ── code-behind bridge ──────────────────────────────────────────────────────

/// `.xaml.cs` code-behind path for a `.xaml`, case-insensitively. Mirrors
/// Python `_xaml_codebehind_path`.
fn codebehind_path(path: &Path) -> Option<PathBuf> {
    let mut expected = path.as_os_str().to_os_string();
    expected.push(".cs");
    let expected = PathBuf::from(expected);
    if expected.exists() {
        return Some(expected);
    }
    let want = expected.file_name()?.to_string_lossy().to_lowercase();
    let dir = path.parent()?;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        if entry.file_name().to_string_lossy().to_lowercase() == want {
            return Some(entry.path());
        }
    }
    None
}

/// Code-behind class node, its event-handler methods (`bare_name → node`), and
/// the class→method edges. Mirrors Python `_xaml_codebehind_symbols`.
fn codebehind_symbols(
    path: &Path,
    class_name: Option<&str>,
) -> (Option<Node>, HashMap<String, Node>, Vec<Edge>) {
    let Some(codebehind) = codebehind_path(path) else {
        return (None, HashMap::new(), Vec::new());
    };
    let result = extract_csharp(&codebehind);
    if result.error.is_some() {
        return (None, HashMap::new(), Vec::new());
    }

    let class_simple = class_name.map(|c| c.rsplit('.').next().unwrap_or(c).to_string());
    let class_node = class_simple
        .as_ref()
        .and_then(|cs| result.nodes.iter().find(|n| &n.label == cs).cloned());

    let mut class_method_edges: Vec<Edge> = Vec::new();
    let method_ids: Option<HashSet<String>> = class_node.as_ref().map(|cn| {
        for edge in &result.edges {
            if edge.source == cn.id && edge.relation == "method" {
                class_method_edges.push(edge.clone());
            }
        }
        class_method_edges
            .iter()
            .map(|e| e.target.clone())
            .collect()
    });

    let cb_lines: Vec<String> = std::fs::read(&codebehind)
        .map(|b| {
            String::from_utf8_lossy(&b)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let has_event_handler_signature = |node: &Node| -> bool {
        let Some(loc) = node.source_location.as_deref() else {
            return false;
        };
        let Some(start) = loc.strip_prefix('L').and_then(|n| n.parse::<usize>().ok()) else {
            return false;
        };
        if start == 0 || cb_lines.is_empty() {
            return false;
        }
        let end = (start - 1 + 3).min(cb_lines.len());
        let snippet = cb_lines[start - 1..end].join(" ");
        re::EVENT_HANDLER_SIGNATURE.is_match(&snippet)
    };

    let mut methods: HashMap<String, Node> = HashMap::new();
    for node in &result.nodes {
        if let Some(ids) = &method_ids
            && !ids.contains(&node.id)
        {
            continue;
        }
        let label = node.label.as_str();
        if label.starts_with('.') && label.ends_with("()") && has_event_handler_signature(node) {
            let bare = label
                .trim_matches(|c| c == '(' || c == ')')
                .trim_start_matches('.');
            methods.insert(bare.to_string(), node.clone());
        }
    }
    (class_node, methods, class_method_edges)
}

// ── ViewModel resolution ────────────────────────────────────────────────────

/// `(has_data_context, viewmodel_simple_names)` from explicit `DataContext`
/// elements/attributes. Mirrors Python `_xaml_explicit_viewmodel_names`.
fn explicit_viewmodel_names(elems: &[XamlElem]) -> (bool, Vec<String>) {
    let mut has_data_context = false;
    let mut names: Vec<String> = Vec::new();
    let push = |n: Option<String>, names: &mut Vec<String>| {
        if let Some(n) = n
            && !names.contains(&n)
        {
            names.push(n);
        }
    };
    for elem in elems {
        if elem.tag.ends_with(".DataContext") || elem.tag == "DataContext" {
            has_data_context = true;
            for &child in &elem.children {
                push(type_simple_name(&elems[child].tag), &mut names);
            }
        }
        for (key, value) in &elem.attrs {
            if key != "DataContext" || value.is_empty() {
                continue;
            }
            has_data_context = true;
            if let Some(m) = re::DESIGN_INSTANCE_TYPE.captures(value) {
                push(type_simple_name(&m[1]), &mut names);
            }
        }
    }
    (has_data_context, names)
}

/// Whether any element sets Prism `ViewModelLocator.AutoWireViewModel="True"`.
fn prism_autowire_viewmodel(elems: &[XamlElem]) -> bool {
    elems.iter().any(|elem| {
        elem.attrs.iter().any(|(key, value)| {
            key.ends_with("ViewModelLocator.AutoWireViewModel")
                && value.trim().eq_ignore_ascii_case("true")
        })
    })
}

/// ViewModel names inferred from a view name by the `View`→`ViewModel`
/// convention. Mirrors Python `_xaml_inferred_viewmodel_names`.
fn inferred_viewmodel_names(view_name: Option<&str>) -> Vec<String> {
    let Some(view_name) = view_name else {
        return Vec::new();
    };
    let mut names: Vec<String> = Vec::new();
    let add = |name: String, names: &mut Vec<String>| {
        if name.ends_with("ViewModel") && !names.contains(&name) {
            names.push(name);
        }
    };
    if view_name == "MainWindow" {
        add("MainWindowViewModel".to_string(), &mut names);
        add("MainViewModel".to_string(), &mut names);
    }
    for suffix in ["UserControl", "View", "Page", "Control"] {
        if view_name.ends_with(suffix) && view_name.len() > suffix.len() {
            add(
                format!("{}ViewModel", &view_name[..view_name.len() - suffix.len()]),
                &mut names,
            );
            break;
        }
    }
    names
}

/// Walk up from the `.xaml` to the nearest dir holding a project marker, capped
/// at the active extract root. Mirrors Python `_xaml_project_root`.
fn project_root(path: &Path) -> PathBuf {
    const MARKERS: &[&str] = &["csproj", "fsproj", "vbproj", "sln", "slnx"];
    let start = path.parent().unwrap_or(path);
    let mut root = start.to_path_buf();
    for dir in std::iter::once(start).chain(start.ancestors().skip(1)) {
        let has_marker = std::fs::read_dir(dir).is_ok_and(|entries| {
            entries.flatten().any(|e| {
                e.path()
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| MARKERS.contains(&x))
            })
        });
        if has_marker {
            root = dir.to_path_buf();
            break;
        }
    }
    let Some(boundary) = active_extract_root() else {
        return root;
    };
    let boundary = boundary.canonicalize().unwrap_or(boundary);
    let resolved = root.canonicalize().unwrap_or_else(|_| root.clone());
    if resolved.starts_with(&boundary) {
        root
    } else {
        boundary
    }
}

/// `ViewModel`-suffixed C# class nodes under the project root, keyed by label.
/// Mirrors Python `_xaml_csharp_class_nodes` (incl. `.graphifyignore` + noise
/// dirs + the per-root cache).
fn csharp_class_nodes(path: &Path) -> HashMap<String, Vec<Node>> {
    let root = project_root(path);
    let cache_key = active_extract_root().map(|_| {
        root.canonicalize()
            .unwrap_or_else(|_| root.clone())
            .to_string_lossy()
            .into_owned()
    });
    if let Some(key) = &cache_key
        && let Some(hit) = XAML_CSHARP_CLASS_CACHE.with(|c| c.borrow().get(key).cloned())
    {
        return hit;
    }

    let mut classes: HashMap<String, Vec<Node>> = HashMap::new();
    let patterns = graphify_detect::load_graphifyignore(&root);
    let mut cs_files: Vec<PathBuf> = Vec::new();
    collect_cs_files(&root, &mut cs_files);
    cs_files.sort();
    for cs_path in cs_files {
        let noisy = cs_path.components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|s| graphify_detect::is_noise_dir(s, None))
        });
        if noisy {
            continue;
        }
        if graphify_detect::is_ignored(&cs_path, &root, &patterns) {
            continue;
        }
        let result = extract_csharp(&cs_path);
        if result.error.is_some() {
            continue;
        }
        for node in result.nodes {
            if node.label.ends_with("ViewModel")
                && re::IDENT_FULL.is_match(&node.label)
                && !node.source_file.is_empty()
            {
                classes.entry(node.label.clone()).or_default().push(node);
            }
        }
    }
    if let Some(key) = cache_key {
        XAML_CSHARP_CLASS_CACHE.with(|c| c.borrow_mut().insert(key, classes.clone()));
    }
    classes
}

/// Recursively collect `*.cs` files under `dir` (skipping noise dirs).
fn collect_cs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            let skip = p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|s| graphify_detect::is_noise_dir(s, None));
            if !skip {
                collect_cs_files(&p, out);
            }
        } else if p.extension().and_then(|x| x.to_str()) == Some("cs") {
            out.push(p);
        }
    }
}

// ── CommunityToolkit generated members ──────────────────────────────────────

/// Capitalise a CommunityToolkit backing-field name (`_userName`/`m_userName` →
/// `UserName`). Mirrors Python `_xaml_pascal_name`.
fn pascal_name(name: &str) -> Option<String> {
    let mut n = name.trim().trim_start_matches('_');
    if let Some(rest) = n.strip_prefix("m_") {
        n = rest;
    }
    if !re::IDENT_FULL.is_match(n) {
        return None;
    }
    let mut chars = n.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
}

/// CommunityToolkit `[ObservableProperty]`/`[RelayCommand]` generated members of
/// a ViewModel node: `(label → member node, defines edges)`. Mirrors Python
/// `_xaml_communitytoolkit_members`.
fn communitytoolkit_members(vm_node: &Node) -> (HashMap<String, Node>, Vec<Edge>) {
    if vm_node.source_file.is_empty() || vm_node.id.is_empty() {
        return (HashMap::new(), Vec::new());
    }
    let Ok(bytes) = std::fs::read(&vm_node.source_file) else {
        return (HashMap::new(), Vec::new());
    };
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();

    let mut members: HashMap<String, Node> = HashMap::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut add_member = |label: &str, line_no: usize, context: &str| {
        let nid = make_id(&[&vm_node.id, label]);
        members.insert(
            label.to_string(),
            Node {
                id: nid.clone(),
                label: label.to_string(),
                file_type: "code".to_string(),
                source_file: vm_node.source_file.clone(),
                source_location: Some(format!("L{line_no}")),
                metadata: None,
                origin_file: None,
            },
        );
        edges.push(Edge {
            external: false,
            source: vm_node.id.clone(),
            target: nid,
            relation: "defines".to_string(),
            confidence: "INFERRED".to_string(),
            source_file: vm_node.source_file.clone(),
            source_location: Some(format!("L{line_no}")),
            weight: 1.0,
            context: Some(context.to_string()),
            confidence_score: None,
        });
    };

    let mut pending: Option<(&str, usize)> = None;
    for (idx, raw_line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let remainder = raw_line.split_once(']').map_or("", |(_, r)| r.trim());
        let mut line = *raw_line;
        if line.contains('[') && line.contains("ObservableProperty") {
            pending = Some(("property", line_no));
            if remainder.is_empty() {
                continue;
            }
            line = remainder;
        }
        if line.contains('[') && line.contains("RelayCommand") {
            pending = Some(("command", line_no));
            if remainder.is_empty() {
                continue;
            }
            line = remainder;
        }
        let Some((kind, attr_line)) = pending else {
            continue;
        };
        if line.trim().is_empty() || line.trim_start().starts_with('[') {
            continue;
        }
        pending = None;
        if kind == "property" {
            if let Some(m) = re::TOOLKIT_FIELD.captures(line)
                && let Some(label) = pascal_name(&m[1])
            {
                add_member(&label, attr_line, "communitytoolkit_observable_property");
            }
        } else if let Some(m) = re::TOOLKIT_METHOD.captures(line) {
            let method = m[1].strip_suffix("Async").unwrap_or(&m[1]);
            add_member(
                &format!("{method}Command"),
                attr_line,
                "communitytoolkit_relay_command",
            );
        }
    }
    (members, edges)
}

// ── extract_xaml ─────────────────────────────────────────────────────────────

/// Extract WPF/XAML structure, bindings, `x:Class`, event-handler references,
/// and the resolved ViewModel. Mirrors Python `extract_xaml`.
#[must_use]
#[allow(clippy::too_many_lines)] // single-pass element walk; splitting fragments the logic
pub fn extract_xaml(path: &Path) -> FileResult {
    let Ok(src) = std::fs::read(path) else {
        return FileResult::error(format!("cannot read {}", path.display()));
    };
    if src.len() as u64 > CSPROJ_MAX_BYTES {
        return FileResult::error("xaml file too large");
    }
    if !crate::extractors::project_xml_is_safe(&src) {
        return FileResult::error("refusing XML with DOCTYPE/ENTITY declaration");
    }
    let Some((elems, root_idx)) = parse_dom(&src) else {
        return FileResult::error("XML parse error");
    };

    let text = String::from_utf8_lossy(&src);
    let lines: Vec<&str> = text.lines().collect();
    let str_path = path.to_string_lossy().into_owned();
    let stem = crate::ids::file_stem(path);
    let file_nid = make_id1(&str_path);
    let root_type = elems[root_idx].tag.clone();
    let root_nid = make_id(&[&stem, &root_type]);

    let mut builder = XamlBuilder {
        nodes: Vec::new(),
        edges: Vec::new(),
        seen_ids: HashSet::new(),
        seen_edges: HashSet::new(),
    };
    let line_for = |value: &str| -> u32 {
        if !value.is_empty() {
            for (idx, line) in lines.iter().enumerate() {
                if line.contains(value) {
                    return u32::try_from(idx + 1).unwrap_or(1);
                }
            }
        }
        1
    };
    let file_name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    builder.add_node(&file_nid, &file_name, Some(1), "code", &str_path);
    builder.add_node(&root_nid, &root_type, Some(1), "code", &str_path);
    builder.add_edge(
        &file_nid,
        &root_nid,
        "contains",
        1,
        None,
        "EXTRACTED",
        &str_path,
    );

    // x:Class → bridge to the code-behind partial class.
    let class_name = elems[root_idx]
        .attrs
        .iter()
        .find(|(k, v)| k == "Class" && !v.is_empty())
        .map(|(_, v)| v.trim().to_string());
    let (class_node, codebehind_methods, class_method_edges) =
        codebehind_symbols(path, class_name.as_deref());
    if let Some(class_name) = &class_name {
        let class_nid = if let Some(cn) = &class_node {
            builder.add_existing_node(Some(cn));
            cn.id.clone()
        } else {
            let class_label = class_name.rsplit('.').next().unwrap_or(class_name);
            let nid = make_id(&[&stem, class_label]);
            builder.add_node(
                &nid,
                class_label,
                Some(line_for(class_name)),
                "code",
                &str_path,
            );
            nid
        };
        builder.add_edge(
            &root_nid,
            &class_nid,
            "references",
            line_for(class_name),
            Some("x_class"),
            "EXTRACTED",
            &str_path,
        );
    }

    // ViewModel resolution: explicit DataContext, else inferred by name/Prism.
    let (has_data_context, mut vm_names) = explicit_viewmodel_names(&elems);
    let prism_autowire = prism_autowire_viewmodel(&elems);
    let mut vm_confidence = "EXTRACTED";
    if !has_data_context {
        let view_name = class_name
            .as_deref()
            .map(|c| c.rsplit('.').next().unwrap_or(c).to_string())
            .or_else(|| {
                prism_autowire
                    .then(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
                    .flatten()
            });
        vm_names = inferred_viewmodel_names(view_name.as_deref());
        vm_confidence = "INFERRED";
    }
    let mut generated_members: HashMap<String, Node> = HashMap::new();
    if !vm_names.is_empty() {
        let csharp_classes = csharp_class_nodes(path);
        let mut by_id: HashMap<String, Node> = HashMap::new();
        for vm_name in &vm_names {
            for node in csharp_classes.get(vm_name).into_iter().flatten() {
                if !node.id.is_empty() {
                    by_id.insert(node.id.clone(), node.clone());
                }
            }
        }
        if by_id.len() == 1
            && let Some(vm_node) = by_id.into_values().next()
        {
            builder.add_existing_node(Some(&vm_node));
            builder.add_edge(
                &root_nid,
                &vm_node.id,
                "references",
                line_for(&vm_node.label),
                Some("view_model"),
                vm_confidence,
                &str_path,
            );
            let (members, member_edges) = communitytoolkit_members(&vm_node);
            generated_members = members;
            for member in generated_members.values() {
                builder.add_existing_node(Some(member));
            }
            for member_edge in member_edges {
                builder.add_existing_edge(&member_edge);
            }
        }
    }

    // Walk every element: named controls, event wiring, and bindings.
    for elem in &elems {
        let elem_type = &elem.tag;
        let elem_name = elem
            .attrs
            .iter()
            .find(|(k, v)| k == "Name" && !v.is_empty())
            .map(|(_, v)| v.trim().to_string());
        let mut owner_nid = root_nid.clone();
        if let Some(elem_name) = &elem_name {
            owner_nid = make_id(&[&stem, elem_name]);
            let line = line_for(elem_name);
            builder.add_node(&owner_nid, elem_name, Some(line), "code", &str_path);
            builder.add_edge(
                &root_nid,
                &owner_nid,
                "contains",
                line,
                None,
                "EXTRACTED",
                &str_path,
            );
            let type_nid = make_id(&["xaml", elem_type]);
            builder.add_node(&type_nid, elem_type, Some(line), "concept", &str_path);
            builder.add_edge(
                &owner_nid,
                &type_nid,
                "references",
                line,
                Some("type"),
                "EXTRACTED",
                &str_path,
            );
        }

        for (key, value) in &elem.attrs {
            let attr_local = key.as_str();
            // Event wiring (gated on the .NET handler signature in codebehind_symbols).
            if !NON_EVENT_ATTRS.contains(&attr_local)
                && re::IDENT_FULL.is_match(value)
                && let Some(method) = codebehind_methods.get(value)
            {
                builder.add_existing_node(Some(method));
                builder.add_edge(
                    &owner_nid,
                    &method.id,
                    "references",
                    line_for(value),
                    Some("event"),
                    "EXTRACTED",
                    &str_path,
                );
                if let Some(method_edge) = class_method_edges.iter().find(|e| e.target == method.id)
                {
                    builder.add_existing_node(class_node.as_ref());
                    builder.add_existing_edge(method_edge);
                }
            }
            let (binding_path, binding_converter) = binding_refs(value);
            if let Some(binding_path) = binding_path {
                let bind_nid = make_id(&["binding", &binding_path]);
                let line = line_for(value);
                builder.add_node(&bind_nid, &binding_path, Some(line), "concept", &str_path);
                let binding_context = if attr_local == "Command" || attr_local.ends_with(".Command")
                {
                    "binding_command"
                } else {
                    "binding_path"
                };
                builder.add_edge(
                    &owner_nid,
                    &bind_nid,
                    "references",
                    line,
                    Some(binding_context),
                    "EXTRACTED",
                    &str_path,
                );
                if let Some(member) = generated_members.get(&binding_path) {
                    builder.add_existing_node(Some(member));
                    builder.add_edge(
                        &owner_nid,
                        &member.id,
                        "references",
                        line,
                        Some(binding_context),
                        "INFERRED",
                        &str_path,
                    );
                }
            }
            if let Some(binding_converter) = binding_converter {
                let converter_nid = make_id(&["binding_converter", &binding_converter]);
                let line = line_for(value);
                builder.add_node(
                    &converter_nid,
                    &binding_converter,
                    Some(line),
                    "concept",
                    &str_path,
                );
                builder.add_edge(
                    &owner_nid,
                    &converter_nid,
                    "references",
                    line,
                    Some("binding_converter"),
                    "EXTRACTED",
                    &str_path,
                );
            }
            if elem_type == "Binding" && attr_local == "Path" {
                let direct = value.trim();
                if !direct.is_empty() && !direct.contains('{') && !direct.contains('}') {
                    let bind_nid = make_id(&["binding", direct]);
                    let line = line_for(value);
                    builder.add_node(&bind_nid, direct, Some(line), "concept", &str_path);
                    builder.add_edge(
                        &owner_nid,
                        &bind_nid,
                        "references",
                        line,
                        Some("binding_path"),
                        "EXTRACTED",
                        &str_path,
                    );
                }
            }
            if elem_type == "Binding"
                && attr_local == "Converter"
                && let Some(direct_converter) = static_resource_key(value)
            {
                let converter_nid = make_id(&["binding_converter", &direct_converter]);
                let line = line_for(value);
                builder.add_node(
                    &converter_nid,
                    &direct_converter,
                    Some(line),
                    "concept",
                    &str_path,
                );
                builder.add_edge(
                    &owner_nid,
                    &converter_nid,
                    "references",
                    line,
                    Some("binding_converter"),
                    "EXTRACTED",
                    &str_path,
                );
            }
        }
    }

    FileResult {
        nodes: builder.nodes,
        edges: builder.edges,
        ..FileResult::default()
    }
}

/// Accumulates deduplicated nodes/edges during the XAML walk.
struct XamlBuilder {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    seen_ids: HashSet<String>,
    seen_edges: HashSet<(String, String, String, Option<String>)>,
}

impl XamlBuilder {
    fn add_node(
        &mut self,
        nid: &str,
        label: &str,
        line: Option<u32>,
        file_type: &str,
        source_file: &str,
    ) {
        if !self.seen_ids.insert(nid.to_string()) {
            return;
        }
        self.nodes.push(Node {
            id: nid.to_string(),
            label: label.to_string(),
            file_type: file_type.to_string(),
            source_file: source_file.to_string(),
            source_location: line.map(|l| format!("L{l}")),
            metadata: None,
            origin_file: None,
        });
    }

    fn add_existing_node(&mut self, node: Option<&Node>) {
        if let Some(node) = node
            && !node.id.is_empty()
            && self.seen_ids.insert(node.id.clone())
        {
            self.nodes.push(node.clone());
        }
    }

    #[allow(clippy::too_many_arguments)] // mirrors the Python add_edge keyword args
    fn add_edge(
        &mut self,
        src: &str,
        tgt: &str,
        relation: &str,
        line: u32,
        context: Option<&str>,
        confidence: &str,
        source_file: &str,
    ) {
        let key = (
            src.to_string(),
            tgt.to_string(),
            relation.to_string(),
            context.map(str::to_string),
        );
        if !self.seen_edges.insert(key) {
            return;
        }
        self.edges.push(Edge {
            external: false,
            source: src.to_string(),
            target: tgt.to_string(),
            relation: relation.to_string(),
            confidence: confidence.to_string(),
            source_file: source_file.to_string(),
            source_location: Some(format!("L{line}")),
            weight: 1.0,
            context: context.map(str::to_string),
            confidence_score: None,
        });
    }

    fn add_existing_edge(&mut self, edge: &Edge) {
        let key = (
            edge.source.clone(),
            edge.target.clone(),
            edge.relation.clone(),
            edge.context.clone(),
        );
        if !self.seen_edges.insert(key) {
            return;
        }
        self.edges.push(edge.clone());
    }
}
