//! Dart extractor — regex-based (no tree-sitter-dart on crates.io).
//!
//! Modernised port of `graphify-py`'s `extract_dart`: comment/string stripping,
//! `part of` redirection, class/mixin/enum/extension-type declarations with
//! inheritance + generics + mixins + interfaces, Bloc/Riverpod/Navigator
//! patterns, annotations, typedefs, extensions, variables, methods,
//! imports/exports, and generic type-lookup invocations.
//!
//! Nodes that Python marks `source_file=None` (global references such as base
//! classes, imported packages, and annotations) carry an empty `source_file`
//! string here, matching the established `generic/inherit.rs` convention — the
//! Rust `Node` type uses a non-optional `String` where Python uses `None`.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};

// ── Regex table ─────────────────────────────────────────────────────────────
// Every pattern is a literal known-good regex; the build cannot panic.

#[allow(clippy::expect_used)]
static COMMENT_STRING_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Match strings (kept) and comments (stripped) so URLs/paths inside string
    // literals are never mistaken for comments. Order matters: triple-quoted
    // strings first, then single-quoted, then block, then line comments.
    let pat = [
        r#""""(?:\\.|[\s\S])*?""""#,
        r"'''(?:\\.|[\s\S])*?'''",
        r#""(?:\\.|[^"\\])*""#,
        r"'(?:\\.|[^'\\])*'",
        r"/\*[\s\S]*?\*/",
        r"//[^\n]*",
    ]
    .join("|");
    Regex::new(&pat).expect("static dart comment/string regex")
});

#[allow(clippy::expect_used)]
static PART_OF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^\s*part\s+of\s+['"]([^'"]+)['"]"#).expect("static dart part-of regex")
});

#[allow(clippy::expect_used)]
static CLASS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s*(?:(?:abstract|sealed|base|interface|final|mixin)\s+)*(?:class|mixin|enum|extension\s+type)\s+(\w+)",
    )
    .expect("static dart class regex")
});

#[allow(clippy::expect_used)]
static EXTENDS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:extends|on)\s+([a-zA-Z0-9_.]+)").expect("dart extends"));

#[allow(clippy::expect_used)]
static WITH_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*with\s+").expect("dart with"));

#[allow(clippy::expect_used)]
static IMPLEMENTS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*implements\s+").expect("dart implements"));

#[allow(clippy::expect_used)]
static ON_EVENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bon<(\w+)>\s*\(").expect("dart on-event"));

#[allow(clippy::expect_used)]
static EMIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:emit|yield)\s*\(?\s*(?:const\s+)?([A-Z]\w*)\b").expect("dart emit")
});

#[allow(clippy::expect_used)]
static BLOC_ADD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:\w*[Bb]loc\w*|context\.read<\w+>\(\))\.add\(\s*(?:const\s+)?([A-Z]\w*)\b")
        .expect("dart bloc-add")
});

#[allow(clippy::expect_used)]
static RIVERPOD_REF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bref\.(?:watch|read|listen)\s*\(\s*(\w+)\b").expect("dart riverpod-ref")
});

#[allow(clippy::expect_used)]
static BLOC_BUILDER_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\bBloc(?:Builder|Listener|Consumer|Provider|Selector)\s*<\s*([a-zA-Z0-9_]+)\b")
        .expect("dart bloc-builder")
});

#[allow(clippy::expect_used)]
static BLOC_LOOKUP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:read|watch|select|of)\s*<([a-zA-Z0-9_]+)>").expect("dart bloc-lookup")
});

#[allow(clippy::expect_used)]
static ANNOTATION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"@(\w+)(?:\([^)]*\))?").expect("dart annotation"));

#[allow(clippy::expect_used)]
static ANNOTATION_FUNC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s*(?:factory\s+|static\s+|async\s+|external\s+|abstract\s+)?(?:\([^)]+\)|[a-zA-Z0-9_<>,.?]+)(?:\s+[a-zA-Z0-9_<>,.?]+){0,3}\s+(\w+)\s*\(",
    )
    .expect("dart annotation-func")
});

#[allow(clippy::expect_used)]
static TYPEDEF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s*typedef\s+(\w+)\s*(?:<[^>]+>)?\s*=\s*([a-zA-Z0-9_<>,.?\s]+);")
        .expect("dart typedef")
});

#[allow(clippy::expect_used)]
static EXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^\s{0,4}extension\s+(\w+)?(?:<[^>]+>)?\s+on\s+(\w+)").expect("dart extension")
});

#[allow(clippy::expect_used)]
static VAR_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s{0,2}(?:late\s+)?(?:(?:final|const|var)\s+)?(?:\([^)]+\)\s+|([a-zA-Z0-9_<>,.?]+(?:\s+[a-zA-Z0-9_<>,.?]+){0,3})\s+)?(?:(\w+)|(?:\w+\s*)?\(([^)]+)\))\s*(?:=|$|;)",
    )
    .expect("dart variable")
});

#[allow(clippy::expect_used)]
static VAR_DECL_KEYWORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:late|final|const|var)\b").expect("dart var-keyword"));

#[allow(clippy::expect_used)]
static METHOD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^\s{0,2}(?:factory\s+|static\s+|async\s+|external\s+|abstract\s+)?(?:\([^)]+\)|[a-zA-Z0-9_<>,.?]+)(?:\s+[a-zA-Z0-9_<>,.?]+){0,3}\s+(\w+(?:\.\w+)?)\s*\(",
    )
    .expect("dart method")
});

#[allow(clippy::expect_used)]
static IMPORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^\s*import\s+['"]([^'"]+)['"]"#).expect("dart import"));

#[allow(clippy::expect_used)]
static EXPORT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?m)^\s*export\s+['"]([^'"]+)['"]"#).expect("dart export"));

#[allow(clippy::expect_used)]
static NAV_PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\b(?:go|push|goNamed|pushNamed|replace|replaceNamed)\s*\(\s*(?:context\s*,\s*)?['"]([a-zA-Z0-9_/?=&%-]+)['"]"#,
    )
    .expect("dart nav-path")
});

#[allow(clippy::expect_used)]
static NAV_CONST_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:go|push|goNamed|pushNamed|replace|replaceNamed)\s*\(\s*(?:context\s*,\s*)?([A-Z][a-zA-Z0-9_]*\.[a-zA-Z0-9_]+)",
    )
    .expect("dart nav-const")
});

#[allow(clippy::expect_used)]
static NAV_OBJ_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:push|replace)\s*\(\s*(?:context\s*,\s*)?.*?\b([A-Z]\w*(?:Route|Screen|Page))\b",
    )
    .expect("dart nav-obj")
});

#[allow(clippy::expect_used)]
static GENERIC_CALL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b\w+<([a-zA-Z0-9_.]+(?:<[a-zA-Z0-9_.,\s<>]+>)?)\s*>\s*\(")
        .expect("dart generic-call")
});

// ── Primitive filter sets ───────────────────────────────────────────────────

const GENERICS_BLACKLIST: &[&str] = &[
    "String", "int", "double", "bool", "num", "dynamic", "Object", "void",
];
const STATE_BLACKLIST: &[&str] = &["String", "List", "Map", "Set", "Future", "Stream", "Object"];
const BLOC_BLACKLIST: &[&str] = &[
    "String", "int", "double", "bool", "num", "dynamic", "Object", "void",
];
const VARTYPE_BLACKLIST: &[&str] = &[
    "String", "int", "double", "bool", "num", "dynamic", "Object", "List", "Map", "Set", "void",
];
const TYPEDEF_BLACKLIST: &[&str] = &[
    "String", "int", "double", "bool", "num", "dynamic", "Object", "List", "Map", "Set", "void",
    "Function",
];
const GENERIC_CALL_BLACKLIST: &[&str] = &[
    "String", "int", "double", "bool", "num", "dynamic", "Object", "List", "Map", "Set", "Future",
    "Stream", "void",
];
const ANNOTATION_SKIP: &[&str] = &[
    "override",
    "deprecated",
    "required",
    "protected",
    "mustCallSuper",
];
const METHOD_NAME_SKIP: &[&str] = &[
    "if", "for", "while", "switch", "catch", "return", "void", "dynamic", "final", "const", "get",
    "set",
];
const STMT_KEYWORD_SKIP: &[&str] = &["if", "for", "while", "switch", "catch", "return"];

// ── Byte/char helpers ───────────────────────────────────────────────────────

/// Largest char-boundary offset `<= i` (mirrors slicing without panicking).
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// `s[start .. start+len]` clamped to char boundaries (Python's `s[a:a+len]`).
fn window(s: &str, start: usize, len: usize) -> &str {
    let lo = floor_char_boundary(s, start.min(s.len()));
    let hi = floor_char_boundary(s, start.saturating_add(len).min(s.len()));
    &s[lo..hi]
}

/// First byte index of `pat` at or after `from` (Python `s.find(pat, from)`).
fn find_from(s: &str, from: usize, pat: char) -> Option<usize> {
    s.get(from..)
        .and_then(|sub| sub.find(pat))
        .map(|p| from + p)
}

/// First byte index of substring `pat` at or after `from` (Python `s.find`).
fn find_substr_from(s: &str, from: usize, pat: &str) -> Option<usize> {
    s.get(from..)
        .and_then(|sub| sub.find(pat))
        .map(|p| from + p)
}

/// First index of `needle` in `haystack[from..]`.
fn find_bytes(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from > haystack.len() || needle.is_empty() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// Split a comma-separated type list, respecting `<>` nesting depth.
#[must_use]
fn split_types(text: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut depth: i32 = 0;
    for ch in text.chars() {
        match ch {
            '<' => {
                depth += 1;
                current.push(ch);
            }
            '>' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        parts.push(current.trim().to_string());
    }
    parts.into_iter().filter(|p| !p.is_empty()).collect()
}

/// Byte index just past the `}` matching the first `{` at/after `start_pos`,
/// skipping string literals and escapes. Returns `text.len()` if unbalanced.
fn find_matching_brace(text: &str, start_pos: usize) -> usize {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let Some(first_brace) = find_from(text, start_pos, '{') else {
        return n;
    };
    let mut brace_count: i32 = 1;
    let mut in_double = false;
    let mut in_single = false;
    let mut escape = false;
    let mut i = first_brace + 1;
    while i < n {
        let c = bytes[i];
        if escape {
            escape = false;
            i += 1;
            continue;
        }
        if c == b'\\' {
            escape = true;
            i += 1;
            continue;
        }
        if !in_single && bytes[i..].starts_with(b"\"\"\"") {
            i += 3;
            i = find_bytes(bytes, i, b"\"\"\"").map_or(n, |end| end + 3);
            continue;
        }
        if !in_double && bytes[i..].starts_with(b"'''") {
            i += 3;
            i = find_bytes(bytes, i, b"'''").map_or(n, |end| end + 3);
            continue;
        }
        if c == b'"' && !in_single {
            in_double = !in_double;
        } else if c == b'\'' && !in_double {
            in_single = !in_single;
        } else if !in_double && !in_single {
            if c == b'{' {
                brace_count += 1;
            } else if c == b'}' {
                brace_count -= 1;
                if brace_count == 0 {
                    return i + 1;
                }
            }
        }
        i += 1;
    }
    n
}

/// Skip a balanced `<...>` or `(...)` run starting at the first `open` in
/// `rest`; returns the byte index just past the matching close.
fn skip_balanced(rest: &str, open: u8, close: u8) -> usize {
    let bytes = rest.as_bytes();
    let Some(offset) = find_from(rest, 0, open as char) else {
        return rest.len();
    };
    let mut depth: i32 = 1;
    let mut i = offset + 1;
    while i < bytes.len() && depth > 0 {
        if bytes[i] == open {
            depth += 1;
        } else if bytes[i] == close {
            depth -= 1;
        }
        i += 1;
    }
    i
}

/// First-segment-before-`<` of a type, trimmed (`Bloc<X>` → `Bloc`).
fn strip_generic(name: &str) -> &str {
    name.split('<').next().unwrap_or(name).trim()
}

/// `source_file` provenance for an emitted node.
#[derive(Clone, Copy)]
enum NodeSrc {
    /// `source_file = str(path)` — the file under extraction.
    File,
    /// `source_file = None` (serialised as empty string here).
    Global,
}

// ── Extractor state ─────────────────────────────────────────────────────────

struct DartExtractor {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    defined: HashSet<String>,
    /// `str(path)` — the file under extraction (edge `source_file`).
    str_path: String,
    /// Possibly redirected to the parent library for a `part of` file.
    stem: String,
    /// Possibly redirected to the parent library for a `part of` file.
    file_nid: String,
}

impl DartExtractor {
    fn add_node(&mut self, nid: &str, label: &str, ftype: &str, src: NodeSrc) {
        if self.defined.insert(nid.to_string()) {
            let source_file = match src {
                NodeSrc::File => self.str_path.clone(),
                NodeSrc::Global => String::new(),
            };
            self.nodes.push(Node {
                id: nid.to_string(),
                label: label.to_string(),
                file_type: ftype.to_string(),
                source_file,
                source_location: None,
                metadata: None,
            });
        }
    }

    fn add_edge(&mut self, src: &str, tgt: &str, relation: &str, context: Option<&str>) {
        self.edges.push(Edge {
            external: false,
            source: src.to_string(),
            target: tgt.to_string(),
            relation: relation.to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: self.str_path.clone(),
            source_location: None,
            weight: 1.0,
            context: context.map(str::to_string),
            confidence_score: Some(1.0),
        });
    }

    // ── Body scanners (per-pattern, called in section-specific order) ────────

    fn scan_on_event(&mut self, owner: &str, body: &str) {
        for caps in ON_EVENT_RE.captures_iter(body) {
            let name = &caps[1];
            let nid = make_id1(name);
            self.add_node(&nid, name, "code", NodeSrc::Global);
            self.add_edge(owner, &nid, "calls", Some("bloc_event"));
        }
    }

    fn scan_emit(&mut self, owner: &str, body: &str) {
        for caps in EMIT_RE.captures_iter(body) {
            let name = &caps[1];
            if STATE_BLACKLIST.contains(&name) {
                continue;
            }
            let nid = make_id1(name);
            self.add_node(&nid, name, "code", NodeSrc::Global);
            self.add_edge(owner, &nid, "calls", Some("emit_state"));
        }
    }

    fn scan_bloc_add(&mut self, owner: &str, body: &str) {
        for caps in BLOC_ADD_RE.captures_iter(body) {
            let name = &caps[1];
            if STATE_BLACKLIST.contains(&name) {
                continue;
            }
            let nid = make_id1(name);
            self.add_node(&nid, name, "code", NodeSrc::Global);
            self.add_edge(owner, &nid, "calls", Some("bloc_add_event"));
        }
    }

    fn scan_riverpod_ref(&mut self, owner: &str, body: &str) {
        for caps in RIVERPOD_REF_RE.captures_iter(body) {
            let name = &caps[1];
            let nid = make_id1(name);
            self.add_node(&nid, name, "code", NodeSrc::Global);
            self.add_edge(owner, &nid, "references", Some("riverpod_reference"));
        }
    }

    fn scan_bloc_builder(&mut self, owner: &str, body: &str) {
        for caps in BLOC_BUILDER_RE.captures_iter(body) {
            let name = &caps[1];
            if BLOC_BLACKLIST.contains(&name) {
                continue;
            }
            let nid = make_id1(name);
            self.add_node(&nid, name, "code", NodeSrc::Global);
            self.add_edge(owner, &nid, "references", Some("bloc_widget_binding"));
        }
    }

    fn scan_bloc_lookup(&mut self, owner: &str, body: &str) {
        for caps in BLOC_LOOKUP_RE.captures_iter(body) {
            let name = &caps[1];
            if BLOC_BLACKLIST.contains(&name) {
                continue;
            }
            let nid = make_id1(name);
            self.add_node(&nid, name, "code", NodeSrc::Global);
            self.add_edge(owner, &nid, "references", Some("bloc_lookup"));
        }
    }

    fn scan_navigation(&mut self, owner: &str, body: &str) {
        for caps in NAV_PATH_RE.captures_iter(body) {
            let route_path = &caps[1];
            let cleaned = route_path.replace(['/', '?', '=', '&'], "_");
            let nid = make_id(&["route", &cleaned]);
            self.add_node(
                &nid,
                &format!("Route {route_path}"),
                "concept",
                NodeSrc::Global,
            );
            self.add_edge(owner, &nid, "navigates", Some("route_path"));
        }
        for caps in NAV_CONST_RE.captures_iter(body) {
            let route_const = &caps[1];
            let nid = make_id(&["route", &route_const.replace('.', "_")]);
            self.add_node(&nid, route_const, "concept", NodeSrc::Global);
            self.add_edge(owner, &nid, "navigates", Some("route_const"));
        }
        for caps in NAV_OBJ_RE.captures_iter(body) {
            let route_class = &caps[1];
            let nid = make_id1(route_class);
            self.add_node(&nid, route_class, "code", NodeSrc::Global);
            self.add_edge(owner, &nid, "navigates", Some("route_object"));
        }
    }
}

// ── Section processors ──────────────────────────────────────────────────────

impl DartExtractor {
    /// Section 1: class/mixin/enum/extension-type declarations with inheritance,
    /// generics, mixins, interfaces, and per-class-body Bloc/Riverpod scanning.
    fn process_classes(&mut self, src: &str) {
        let file_nid = self.file_nid.clone();
        let stem = self.stem.clone();
        for caps in CLASS_RE.captures_iter(src) {
            let (Some(m), Some(name_m)) = (caps.get(0), caps.get(1)) else {
                continue;
            };
            let class_name = name_m.as_str().to_string();
            let class_nid = make_id(&[&stem, &class_name]);
            self.add_node(&class_nid, &class_name, "code", NodeSrc::File);
            self.add_edge(&file_nid, &class_nid, "defines", None);

            // Header: text after the class name, with generic params and any
            // primary constructor skipped, up to `{` or `;`.
            let mut rest = window(src, m.end(), 500).to_string();
            if rest.trim_start().starts_with('<') {
                rest = rest[skip_balanced(&rest, b'<', b'>')..].to_string();
            }
            if rest.trim_start().starts_with('(') {
                rest = rest[skip_balanced(&rest, b'(', b')')..].to_string();
            }
            let header_end = rest
                .find('{')
                .or_else(|| rest.find(';'))
                .unwrap_or(rest.len());
            let header = &rest[..header_end];
            let (base_class, generics, mixins_list, interfaces_list) = parse_class_header(header);

            if let Some(base) = base_class {
                let base_nid = make_id1(&base);
                self.add_node(&base_nid, &base, "code", NodeSrc::Global);
                self.add_edge(&class_nid, &base_nid, "inherits", None);
                if let Some(g) = generics {
                    for arg in split_types(&g) {
                        let gen_clean = strip_generic(&arg);
                        if !GENERICS_BLACKLIST.contains(&gen_clean) {
                            let gen_nid = make_id1(gen_clean);
                            self.add_node(&gen_nid, gen_clean, "code", NodeSrc::Global);
                            self.add_edge(&class_nid, &gen_nid, "references", None);
                        }
                    }
                }
            }
            for mixin in &mixins_list {
                let mc = strip_generic(mixin);
                let nid = make_id1(mc);
                self.add_node(&nid, mc, "code", NodeSrc::Global);
                self.add_edge(&class_nid, &nid, "implements", None);
            }
            for interface in &interfaces_list {
                let ic = strip_generic(interface);
                let nid = make_id1(ic);
                self.add_node(&nid, ic, "code", NodeSrc::Global);
                self.add_edge(&class_nid, &nid, "implements", None);
            }

            if let Some(body) = class_body(src, m.start()) {
                self.scan_on_event(&class_nid, &body);
                self.scan_emit(&class_nid, &body);
                self.scan_bloc_add(&class_nid, &body);
                self.scan_riverpod_ref(&class_nid, &body);
                self.scan_bloc_builder(&class_nid, &body);
                self.scan_bloc_lookup(&class_nid, &body);
            }
        }
    }

    /// Section 2: annotations linked to the next class/function declaration.
    fn process_annotations(&mut self, src: &str) {
        let stem = self.stem.clone();
        for caps in ANNOTATION_RE.captures_iter(src) {
            let (Some(m), Some(name_m)) = (caps.get(0), caps.get(1)) else {
                continue;
            };
            let annotation_name = name_m.as_str();
            if ANNOTATION_SKIP.contains(&annotation_name) {
                continue;
            }
            let intervening = window(src, m.end(), 300);
            let class_caps = CLASS_RE.captures(intervening);
            let func_caps = ANNOTATION_FUNC_RE.captures(intervening);
            let class_start = class_caps
                .as_ref()
                .and_then(|c| c.get(0))
                .map(|x| x.start());
            let func_start = func_caps.as_ref().and_then(|c| c.get(0)).map(|x| x.start());

            // Pick the earliest of class/function as the annotation target.
            let (target_name, target_is_class) = match (class_start, func_start) {
                (Some(cs), Some(fs)) if cs < fs => (
                    class_caps
                        .as_ref()
                        .and_then(|c| c.get(1))
                        .map(|g| g.as_str()),
                    true,
                ),
                (Some(_), Some(_)) => (
                    func_caps
                        .as_ref()
                        .and_then(|c| c.get(1))
                        .map(|g| g.as_str()),
                    false,
                ),
                (Some(_), None) => (
                    class_caps
                        .as_ref()
                        .and_then(|c| c.get(1))
                        .map(|g| g.as_str()),
                    true,
                ),
                (None, Some(_)) => (
                    func_caps
                        .as_ref()
                        .and_then(|c| c.get(1))
                        .map(|g| g.as_str()),
                    false,
                ),
                (None, None) => (None, false),
            };
            let Some(target_name) = target_name else {
                continue;
            };
            let target_nid = make_id(&[&stem, target_name]);

            // Only configure when the annotation directly precedes the
            // declaration — no statement terminator in between.
            let cut = class_start.unwrap_or(300).min(func_start.unwrap_or(300));
            let actual = window(intervening, 0, cut);
            if actual.contains(';') || actual.contains('}') || actual.contains('{') {
                continue;
            }

            let annotation_nid = make_id(&["annotation", &annotation_name.to_lowercase()]);
            let label = format!("@{annotation_name}");
            self.add_node(&annotation_nid, &label, "concept", NodeSrc::Global);
            self.add_edge(&target_nid, &annotation_nid, "configures", None);

            // Riverpod codegen: a `@riverpod` class/function defines a provider.
            if annotation_name.eq_ignore_ascii_case("riverpod") {
                let provider_name = if target_is_class {
                    format!("{}Provider", lower_first_camel(target_name))
                } else {
                    format!("{target_name}Provider")
                };
                let provider_nid = make_id1(&provider_name);
                self.add_node(&provider_nid, &provider_name, "concept", NodeSrc::File);
                self.add_edge(
                    &target_nid,
                    &provider_nid,
                    "defines",
                    Some("riverpod_provider"),
                );
            }
        }
    }

    /// Section 2.5: typedef (type alias) declarations.
    fn process_typedefs(&mut self, src: &str) {
        let file_nid = self.file_nid.clone();
        let stem = self.stem.clone();
        for caps in TYPEDEF_RE.captures_iter(src) {
            let (Some(name_m), Some(target_m)) = (caps.get(1), caps.get(2)) else {
                continue;
            };
            let typedef_name = name_m.as_str();
            let target_type = last_segment(strip_generic(target_m.as_str()));
            if TYPEDEF_BLACKLIST.contains(&target_type) {
                continue;
            }
            let typedef_nid = make_id(&[&stem, typedef_name]);
            self.add_node(&typedef_nid, typedef_name, "code", NodeSrc::File);
            self.add_edge(&file_nid, &typedef_nid, "defines", None);
            let target_nid = make_id1(target_type);
            self.add_node(&target_nid, target_type, "code", NodeSrc::Global);
            self.add_edge(&typedef_nid, &target_nid, "references", Some("typedef"));
        }
    }

    /// Section 3: `extension Name on Target` declarations.
    fn process_extensions(&mut self, src: &str) {
        let file_nid = self.file_nid.clone();
        let stem = self.stem.clone();
        for caps in EXT_RE.captures_iter(src) {
            let Some(target_m) = caps.get(2) else {
                continue;
            };
            let target_class = target_m.as_str();
            let name_opt = caps.get(1).map(|x| x.as_str());
            let ext_name =
                name_opt.map_or_else(|| format!("{stem}_anonymous_extension"), str::to_string);
            let label =
                name_opt.map_or_else(|| format!("Extension on {target_class}"), str::to_string);
            let ext_nid = make_id(&[&stem, &ext_name]);
            self.add_node(&ext_nid, &label, "code", NodeSrc::File);
            self.add_edge(&file_nid, &ext_nid, "defines", None);
            let target_nid = make_id1(target_class);
            self.add_node(&target_nid, target_class, "code", NodeSrc::Global);
            self.add_edge(&ext_nid, &target_nid, "extends", None);
        }
    }

    /// Section 4: top-level / class-level variable declarations (single and
    /// record/object-destructured), with optional type references.
    fn process_variables(&mut self, src: &str) {
        let file_nid = self.file_nid.clone();
        let stem = self.stem.clone();
        for caps in VAR_RE.captures_iter(src) {
            let whole = caps.get(0).map_or("", |x| x.as_str());
            let var_type = caps.get(1).map(|x| x.as_str());
            // A bare type-led match with no `late|final|const|var` keyword is a
            // false positive (e.g. a statement) unless a type was captured.
            if !VAR_DECL_KEYWORD_RE.is_match(whole) && var_type.is_none() {
                continue;
            }
            if let Some(single) = caps.get(2).map(|x| x.as_str()) {
                if STMT_KEYWORD_SKIP.contains(&single) {
                    continue;
                }
                let var_nid = make_id(&[&stem, single]);
                self.add_node(&var_nid, single, "code", NodeSrc::File);
                self.add_edge(&file_nid, &var_nid, "defines", None);
                if let Some(vt) = var_type
                    && !VARTYPE_BLACKLIST.contains(&vt)
                {
                    let clean_type = last_segment(strip_generic(vt));
                    let type_nid = make_id1(clean_type);
                    self.add_node(&type_nid, clean_type, "code", NodeSrc::Global);
                    self.add_edge(&file_nid, &type_nid, "references", Some("variable_type"));
                }
            } else if let Some(destructured) = caps.get(3).map(|x| x.as_str()) {
                for raw in destructured.split(',') {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    // `name: value` patterns bind the value identifier, not the key.
                    let name = if trimmed.contains(':') {
                        last_segment_sep(trimmed, ':')
                    } else {
                        trimmed
                    };
                    if is_lower_ident(name) && !STMT_KEYWORD_SKIP.contains(&name) {
                        let var_nid = make_id(&[&stem, name]);
                        self.add_node(&var_nid, name, "code", NodeSrc::File);
                        self.add_edge(&file_nid, &var_nid, "defines", None);
                    }
                }
            }
        }
    }

    /// Section 5: top-level / member functions and methods, with per-body
    /// Riverpod/Bloc reference and navigation scanning.
    fn process_methods(&mut self, src: &str) {
        let file_nid = self.file_nid.clone();
        let stem = self.stem.clone();
        for caps in METHOD_RE.captures_iter(src) {
            let (Some(m), Some(name_m)) = (caps.get(0), caps.get(1)) else {
                continue;
            };
            let name = last_segment(name_m.as_str());
            if METHOD_NAME_SKIP.contains(&name)
                || name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            {
                continue;
            }
            let nid = make_id(&[&stem, name]);
            self.add_node(&nid, name, "code", NodeSrc::File);
            self.add_edge(&file_nid, &nid, "defines", None);

            if let Some(body) = method_body(src, m.start()) {
                self.scan_riverpod_ref(&nid, &body);
                self.scan_bloc_add(&nid, &body);
                self.scan_bloc_lookup(&nid, &body);
                self.scan_navigation(&nid, &body);
            }
        }
    }

    /// Section 6: `import '...'` and `export '...'` directives.
    fn process_imports_exports(&mut self, src: &str) {
        let file_nid = self.file_nid.clone();
        for caps in IMPORT_RE.captures_iter(src) {
            if let Some(pkg) = caps.get(1).map(|x| x.as_str()) {
                let nid = make_id1(pkg);
                self.add_node(&nid, pkg, "code", NodeSrc::Global);
                self.add_edge(&file_nid, &nid, "imports", None);
            }
        }
        for caps in EXPORT_RE.captures_iter(src) {
            if let Some(pkg) = caps.get(1).map(|x| x.as_str()) {
                let nid = make_id1(pkg);
                self.add_node(&nid, pkg, "code", NodeSrc::Global);
                self.add_edge(&file_nid, &nid, "exports", None);
            }
        }
    }

    /// Section 7: generic invocations / type lookups (`method<Type>(...)`).
    fn process_generic_calls(&mut self, src: &str) {
        let file_nid = self.file_nid.clone();
        for caps in GENERIC_CALL_RE.captures_iter(src) {
            let Some(g1) = caps.get(1).map(|x| x.as_str()) else {
                continue;
            };
            let clean = strip_generic(last_segment(g1));
            if !GENERIC_CALL_BLACKLIST.contains(&clean) {
                let nid = make_id1(clean);
                self.add_node(&nid, clean, "code", NodeSrc::Global);
                self.add_edge(&file_nid, &nid, "references", Some("type_lookup"));
            }
        }
    }
}

/// Parse the inheritance clause of a class header into
/// `(base, generics, mixins, interfaces)`.
fn parse_class_header(header: &str) -> (Option<String>, Option<String>, Vec<String>, Vec<String>) {
    let mut base_class: Option<String> = None;
    let mut generics: Option<String> = None;
    let mut mixins_list: Vec<String> = Vec::new();
    let mut interfaces_list: Vec<String> = Vec::new();
    let mut header = header.to_string();

    if let Some(caps) = EXTENDS_RE.captures(&header) {
        let end = caps.get(0).map_or(0, |x| x.end());
        base_class = caps.get(1).map(|x| x.as_str().to_string());
        let rest_header = header[end..].to_string();
        if rest_header.trim_start().starts_with('<') {
            let bytes = rest_header.as_bytes();
            if let Some(start) = find_from(&rest_header, 0, '<') {
                let mut depth: i32 = 1;
                let mut i = start + 1;
                let mut captured = false;
                while i < bytes.len() && depth > 0 {
                    if bytes[i] == b'<' {
                        depth += 1;
                    } else if bytes[i] == b'>' {
                        depth -= 1;
                        if depth == 0 {
                            generics = Some(rest_header[start + 1..i].to_string());
                            header = rest_header[(i + 1).min(rest_header.len())..].to_string();
                            captured = true;
                            break;
                        }
                    }
                    i += 1;
                }
                if !captured {
                    header = rest_header;
                }
            } else {
                header = rest_header;
            }
        } else {
            header = rest_header;
        }
    }

    if let Some(caps) = WITH_RE.captures(&header) {
        let end = caps.get(0).map_or(0, |x| x.end());
        let rest_header = header[end..].to_string();
        if let Some(impl_idx) = rest_header.find("implements") {
            mixins_list = split_types(&rest_header[..impl_idx]);
            header = rest_header[impl_idx..].to_string();
        } else {
            mixins_list = split_types(&rest_header);
            header = String::new();
        }
    }

    if let Some(caps) = IMPLEMENTS_RE.captures(&header) {
        let end = caps.get(0).map_or(0, |x| x.end());
        interfaces_list = split_types(&header[end..]);
    }

    (base_class, generics, mixins_list, interfaces_list)
}

/// Extract a class body `{...}` starting from the class-match start, or `None`
/// when the declaration has no body (e.g. `class Foo;` / abstract stub).
fn class_body(src: &str, start_idx: usize) -> Option<String> {
    body_from(src, start_idx, false)
}

/// Extract a method body `{...}`, skipping `=>` arrow bodies and `;` stubs.
fn method_body(src: &str, start_idx: usize) -> Option<String> {
    body_from(src, start_idx, true)
}

/// Shared body extraction. A `;` before the `{` means no body; for methods, an
/// `=>` arrow before the `{` also means no block body.
fn body_from(src: &str, start_idx: usize, check_arrow: bool) -> Option<String> {
    let brace_pos = find_from(src, start_idx, '{')?;
    if let Some(semi) = find_from(src, start_idx, ';')
        && semi < brace_pos
    {
        return None;
    }
    if check_arrow
        && let Some(arrow) = find_substr_from(src, start_idx, "=>")
        && arrow < brace_pos
    {
        return None;
    }
    let end_pos = find_matching_brace(src, start_idx);
    Some(src[brace_pos..end_pos.max(brace_pos)].to_string())
}

/// First char lowercased, rest unchanged (`MyNotifier` → `myNotifier`).
fn lower_first_camel(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| {
        format!("{}{}", first.to_lowercase(), chars.as_str())
    })
}

/// Segment after the last `.` (`foo.Bar` → `Bar`), trimmed.
fn last_segment(s: &str) -> &str {
    last_segment_sep(s, '.')
}

/// Segment after the last `sep`, trimmed.
fn last_segment_sep(s: &str, sep: char) -> &str {
    s.rsplit(sep).next().unwrap_or(s).trim()
}

/// `^[a-z_]` followed by `\w*` (Python `^[a-zA-Z_]\w*$` minus `^[A-Z]`).
fn is_lower_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_lowercase()) {
        return false;
    }
    chars.all(|c| c == '_' || c.is_alphanumeric())
}

/// Extract classes, mixins, enums, extensions, generic invocations, and
/// annotations from a `.dart` file using regex.
#[must_use]
pub fn extract_dart(path: &Path) -> FileResult {
    let src = match std::fs::read(path) {
        // Mirror Python's `read_text(errors="replace")`: invalid UTF-8 is
        // replaced rather than aborting the extraction.
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => return FileResult::error(format!("cannot read {}", path.display())),
    };

    let str_path = path.to_string_lossy().into_owned();

    // Strip comments while leaving string literals intact so URLs/paths inside
    // strings are never mistaken for comments.
    let src_clean = COMMENT_STRING_RE
        .replace_all(&src, |caps: &regex::Captures| {
            let token = &caps[0];
            if token.starts_with('/') {
                String::new()
            } else {
                token.to_string()
            }
        })
        .into_owned();

    // `part of '<parent>.dart'` redirects child IDs to the parent library and
    // suppresses the child's own file node.
    let mut stem = file_stem(path);
    let mut file_nid = make_id1(&str_path);
    let mut is_part = false;
    if let Some(caps) = PART_OF_RE.captures(&src_clean)
        && let Some(parent_ref) = caps.get(1).map(|x| x.as_str())
        // A Dart `part of` directive references a lowercase `.dart` file by
        // convention; a case-insensitive match would treat an unrelated
        // `.DART` literal as a redirect and diverge from the source semantics.
        && {
            #[allow(clippy::case_sensitive_file_extension_comparisons)]
            parent_ref.ends_with(".dart")
        }
        && let Some(parent_dir) = path.parent()
        // `canonicalize` resolves symlinks AND requires existence, matching
        // Python's `resolve()` + `exists()` guard.
        && let Ok(resolved) = parent_dir.join(parent_ref).canonicalize()
    {
        stem = file_stem(&resolved);
        file_nid = make_id1(&resolved.to_string_lossy());
        is_part = true;
    }

    let mut ext = DartExtractor {
        nodes: Vec::new(),
        edges: Vec::new(),
        defined: HashSet::new(),
        str_path: str_path.clone(),
        stem,
        file_nid: file_nid.clone(),
    };

    if !is_part {
        ext.nodes.push(Node {
            id: file_nid,
            label: path
                .file_name()
                .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
            file_type: "code".to_string(),
            source_file: str_path,
            source_location: None,
            metadata: None,
        });
    }

    ext.process_classes(&src_clean);
    ext.process_annotations(&src_clean);
    ext.process_typedefs(&src_clean);
    ext.process_extensions(&src_clean);
    ext.process_variables(&src_clean);
    ext.process_methods(&src_clean);
    ext.process_imports_exports(&src_clean);
    ext.process_generic_calls(&src_clean);

    FileResult {
        nodes: ext.nodes,
        edges: ext.edges,
        raw_calls: vec![],
        error: None,
    }
}
