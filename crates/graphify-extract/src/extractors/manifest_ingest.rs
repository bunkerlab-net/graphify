//! Deterministic package-manifest ingestion (#1377).
//!
//! Parses `apm.yml`/`apm.yaml`/`pyproject.toml`/`go.mod`/`pom.xml` into ONE
//! canonical package node (keyed by package NAME via `make_id(["pkg", name])`)
//! plus `depends_on` edges, so a package referenced by N manifests collapses to
//! a single hub node. Routed to the AST/CODE path so the LLM never sees them.
//! Mirrors `graphify-py/graphify/manifest_ingest.py`.
//!
//! Divergence from graphify-py: the canonical `type="package"`, `ecosystem`, and
//! `version` attributes are carried in the node's nested `metadata` map — the
//! established Rust convention shared with the MCP extractor's `mcp_kind` — rather
//! than as top-level node keys. The package id (`pkg_<name>`) and `depends_on`
//! edges are byte-identical to graphify-py.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use indexmap::IndexMap;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use regex::Regex;
use serde_json::Value;

use crate::ids::make_id;
use crate::types::{Edge, FileResult, Node};

/// 2 MB cap — manifests are small; this rejects junk. Mirrors `_MAX_MANIFEST_BYTES`.
const MAX_MANIFEST_BYTES: u64 = 2_000_000;

#[allow(clippy::unwrap_used)] // literal regex patterns; cannot fail.
mod re {
    use super::{LazyLock, Regex};
    pub static APM_NAME: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"^name:\s*["']?([^"'\s#]+)"#).unwrap());
    pub static APM_VERSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"^version:\s*["']?([^"'\s#]+)"#).unwrap());
    pub static APM_DEPS_START: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^dependencies:\s*$").unwrap());
    pub static APM_DEP_ITEM: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r#"^\s*-\s*["']?([^"'\s#:]+)"#).unwrap());
    pub static APM_DEP_MAP: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s{2,}([A-Za-z0-9._/@-]+)\s*:").unwrap());
    pub static PEP508_SPLIT: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"[\s<>=!~;\[\(]").unwrap());
    pub static GOMOD_MODULE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^module\s+(\S+)").unwrap());
    pub static GOMOD_REQUIRE_BLOCK: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^require\s*\(").unwrap());
    pub static GOMOD_DEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\S+)\s+v\S+").unwrap());
    pub static GOMOD_REQUIRE_LINE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^require\s+(\S+)\s+v\S+").unwrap());
}

/// Parsed manifest summary: `name`, optional `version`, and dependency names.
struct ManifestInfo {
    name: String,
    version: Option<String>,
    deps: Vec<String>,
}

/// Canonical package node id, keyed by package NAME so every reference to the
/// same package — its own manifest and any dependent's dependency line — maps to
/// one node. Mirrors `_pkg_id`.
fn pkg_id(name: &str) -> String {
    make_id(&["pkg", name])
}

/// Ecosystem tag for a recognized manifest filename (lowercased), via the shared
/// `graphify_detect::PACKAGE_MANIFEST_NAMES` table.
fn ecosystem_for(filename_lc: &str) -> Option<&'static str> {
    graphify_detect::PACKAGE_MANIFEST_NAMES
        .iter()
        .find(|(n, _)| *n == filename_lc)
        .map(|(_, eco)| *eco)
}

/// Parse a package manifest into a canonical package node + `depends_on` edges.
///
/// Returns an empty [`FileResult`] (no nodes/edges) on read error, oversize file,
/// parse failure, or a manifest with no resolvable name — a malformed manifest
/// must never abort extraction. Mirrors `extract_package_manifest`.
#[must_use]
pub fn extract_package_manifest(path: &Path) -> FileResult {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > MAX_MANIFEST_BYTES => {
            return FileResult::error("manifest too large to index");
        }
        Ok(_) => {}
        Err(e) => return FileResult::error(format!("manifest read error: {e}")),
    }
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => return FileResult::error(format!("manifest read error: {e}")),
    };
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    let Some(eco) = ecosystem_for(&filename) else {
        return FileResult::default();
    };
    let info = match eco {
        "apm" => parse_apm(&text),
        "python" => parse_pyproject(&text),
        "go" => parse_gomod(&text),
        "maven" => parse_pom(&text),
        _ => None,
    };
    let Some(info) = info.filter(|i| !i.name.is_empty()) else {
        return FileResult::default();
    };

    let str_path = path.to_string_lossy().into_owned();
    let pkg_nid = pkg_id(&info.name);
    let mut metadata: IndexMap<String, Value> = IndexMap::new();
    // `file_type=code` keeps build.py validation happy; `type` distinguishes packages.
    metadata.insert("type".to_string(), Value::String("package".to_string()));
    metadata.insert("ecosystem".to_string(), Value::String(eco.to_string()));
    if let Some(v) = &info.version {
        metadata.insert("version".to_string(), Value::String(v.clone()));
    }
    let node = Node {
        id: pkg_nid.clone(),
        label: info.name.clone(),
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: Some(metadata),
        origin_file: None,
        node_type: None,
    };

    let mut edges: Vec<Edge> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for dep in &info.deps {
        if dep.is_empty() {
            continue;
        }
        let dep_nid = pkg_id(dep);
        // The edge targets the dependency's canonical package id. If that package's
        // own manifest is in the corpus the edge resolves to its single node; if
        // external, build_from_json prunes the dangling edge. No stub node is
        // emitted — a stub with an empty source_file would risk clobbering the real
        // node's source_file under id-dedup.
        if dep_nid == pkg_nid || !seen.insert(dep_nid.clone()) {
            continue;
        }
        edges.push(Edge {
            external: false,
            source: pkg_nid.clone(),
            target: dep_nid,
            relation: "depends_on".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.clone(),
            source_location: Some("L1".to_string()),
            weight: 1.0,
            context: Some("dependency".to_string()),
            confidence_score: Some(1.0),
            deferred: false,
            metadata: None,
        });
    }

    FileResult {
        nodes: vec![node],
        edges,
        raw_calls: vec![],
        error: None,
    }
}

/// Minimal `apm.yml` line parser: a top-level `name:`/`version:` plus a simple
/// `dependencies:` block (list items or a name->spec map). Mirrors the
/// `_parse_apm`/`_parse_apm_fallback` observable contract without a YAML crate.
fn parse_apm(text: &str) -> Option<ManifestInfo> {
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;
    let mut deps: Vec<String> = Vec::new();
    let mut in_deps = false;
    for line in text.lines() {
        if !in_deps {
            if let Some(c) = re::APM_NAME.captures(line) {
                name = Some(c[1].to_string());
                continue;
            }
            if let Some(c) = re::APM_VERSION.captures(line) {
                version = Some(c[1].to_string());
                continue;
            }
        }
        if re::APM_DEPS_START.is_match(line) {
            in_deps = true;
            continue;
        }
        if in_deps {
            if let Some(c) = re::APM_DEP_ITEM
                .captures(line)
                .or_else(|| re::APM_DEP_MAP.captures(line))
            {
                deps.push(c[1].to_string());
            } else if line.starts_with(|ch: char| !ch.is_whitespace()) {
                in_deps = false; // next top-level key ends the block
            }
        }
    }
    name.map(|name| ManifestInfo {
        name,
        version,
        deps,
    })
}

/// `requests>=2.0` -> `requests`; `pkg[extra]==1; python_version<'3.9'` -> `pkg`.
fn pep508_name(spec: &str) -> String {
    re::PEP508_SPLIT
        .split(spec.trim())
        .next()
        .unwrap_or("")
        .to_string()
}

/// Parse `pyproject.toml`: PEP 621 `[project]` name/version/dependencies plus
/// `[tool.poetry]`. Mirrors `_parse_pyproject`.
fn parse_pyproject(text: &str) -> Option<ManifestInfo> {
    let data: toml::Value = toml::from_str(text).ok()?;
    let project = data.get("project").and_then(toml::Value::as_table);
    let poetry = data
        .get("tool")
        .and_then(toml::Value::as_table)
        .and_then(|t| t.get("poetry"))
        .and_then(toml::Value::as_table);
    let name = project
        .and_then(|p| p.get("name"))
        .and_then(toml::Value::as_str)
        .or_else(|| {
            poetry
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str)
        })?;
    let version = project
        .and_then(|p| p.get("version"))
        .and_then(toml::Value::as_str)
        .or_else(|| {
            poetry
                .and_then(|p| p.get("version"))
                .and_then(toml::Value::as_str)
        })
        .map(str::to_string);
    let mut deps: Vec<String> = Vec::new();
    if let Some(arr) = project
        .and_then(|p| p.get("dependencies"))
        .and_then(toml::Value::as_array)
    {
        for d in arr {
            if let Some(s) = d.as_str() {
                deps.push(pep508_name(s));
            }
        }
    }
    if let Some(tbl) = poetry
        .and_then(|p| p.get("dependencies"))
        .and_then(toml::Value::as_table)
    {
        for k in tbl.keys() {
            if !k.eq_ignore_ascii_case("python") {
                deps.push(k.clone());
            }
        }
    }
    Some(ManifestInfo {
        name: name.to_string(),
        version,
        deps,
    })
}

/// Parse `go.mod`: the `module` path plus `require` block/inline lines. Mirrors
/// `_parse_gomod`.
fn parse_gomod(text: &str) -> Option<ManifestInfo> {
    let mut name: Option<String> = None;
    let mut deps: Vec<String> = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        let s = line.trim();
        if name.is_none()
            && let Some(c) = re::GOMOD_MODULE.captures(s)
        {
            name = Some(c[1].to_string());
            continue;
        }
        if re::GOMOD_REQUIRE_BLOCK.is_match(s) {
            in_block = true;
            continue;
        }
        if in_block {
            if s.starts_with(')') {
                in_block = false;
                continue;
            }
            if let Some(c) = re::GOMOD_DEP.captures(s) {
                deps.push(c[1].to_string());
            }
        } else if let Some(c) = re::GOMOD_REQUIRE_LINE.captures(s) {
            deps.push(c[1].to_string());
        }
    }
    name.map(|name| ManifestInfo {
        name,
        version: None,
        deps,
    })
}

/// Parse `pom.xml`: project `groupId:artifactId`(`:version`) plus each
/// `dependencies/dependency`. Mirrors `_parse_pom`; returns `None` on malformed
/// XML or a missing artifactId. The default namespace is ignored (quick-xml
/// yields the unprefixed local names).
#[allow(clippy::similar_names)] // gid/aid are the canonical Maven coordinate terms
fn parse_pom(text: &str) -> Option<ManifestInfo> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut project_gid: Option<String> = None;
    let mut project_aid: Option<String> = None;
    let mut project_ver: Option<String> = None;
    let mut deps: Vec<(Option<String>, Option<String>)> = Vec::new();
    let mut cur_dep: Option<(Option<String>, Option<String>)> = None;
    loop {
        let Ok(event) = reader.read_event_into(&mut buf) else {
            return None; // malformed XML -> empty result, never crash.
        };
        match event {
            Event::Eof => break,
            Event::Start(e) => {
                let name = e.name().into_inner().to_owned();
                if name == "dependency" && stack.last().map(String::as_str) == Some("dependencies")
                {
                    cur_dep = Some((None, None));
                }
                stack.push(name);
            }
            Event::End(_) => {
                if stack.pop().as_deref() == Some("dependency")
                    && let Some(d) = cur_dep.take()
                {
                    deps.push(d);
                }
            }
            Event::Text(t) => {
                let val = t.trim();
                if !val.is_empty() {
                    let cur = stack.last().map(String::as_str);
                    let parent = stack.len().checked_sub(2).map(|i| stack[i].as_str());
                    if parent == Some("dependency") {
                        if let Some(dep) = &mut cur_dep {
                            match cur {
                                Some("groupId") => dep.0 = Some(val.to_string()),
                                Some("artifactId") => dep.1 = Some(val.to_string()),
                                _ => {}
                            }
                        }
                    } else if parent == Some("project") {
                        match cur {
                            Some("groupId") => project_gid = Some(val.to_string()),
                            Some("artifactId") => project_aid = Some(val.to_string()),
                            Some("version") => project_ver = Some(val.to_string()),
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
        buf.clear();
    }
    let aid = project_aid?;
    let name = match &project_gid {
        Some(gid) => format!("{gid}:{aid}"),
        None => aid,
    };
    let dep_names = deps
        .into_iter()
        .filter_map(|(dg, da)| {
            da.map(|da| match dg {
                Some(dg) => format!("{dg}:{da}"),
                None => da,
            })
        })
        .collect();
    Some(ManifestInfo {
        name,
        version: project_ver,
        deps: dep_names,
    })
}
