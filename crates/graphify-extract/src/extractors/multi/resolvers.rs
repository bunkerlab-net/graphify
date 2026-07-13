//! Cross-file, language-specific member-call resolver dispatch (#1499).
//!
//! Formalizes the previously hand-wired sequence of suffix-gated resolution
//! passes (`if !swift_paths.is_empty() { resolve_swift(...) }` …) so a new
//! language plugs in by adding one [`LanguageResolver`] to [`default_resolvers`]
//! instead of editing `extract`'s body. Mirrors the observable contract of
//! graphify-py's `resolver_registry`: suffix gating and ordered execution.
//!
//! Divergence from graphify-py: the Python registry wraps each pass in
//! `try/except` and logs-and-continues on failure. The Rust resolvers are
//! infallible by construction — they guard internally (god-node checks,
//! single-definition requirements) and return `()` — so there is no panic to
//! isolate, and no `catch_unwind` is used (a panic here is a bug, not a
//! recoverable per-language failure).

use std::collections::HashSet;
use std::path::PathBuf;

use crate::types::{Edge, Node, RawCall};

/// A resolver pass: reads the corpus `nodes` / `raw_calls` and the input
/// `paths`, mutating `edges` in place. `paths` lets a pass that re-parses source
/// (Swift) find its files; name-only passes (Python, Ruby) ignore it.
type ResolveFn = fn(&[PathBuf], &[Node], &mut Vec<Edge>, &[RawCall]);

/// One cross-file, language-specific resolution pass. `suffixes` (dotted, e.g.
/// `.rb`) gates activation: the pass runs only when the corpus contains at least
/// one file with one of these extensions. Mirrors graphify-py
/// `resolver_registry.LanguageResolver`.
pub(super) struct LanguageResolver {
    /// Dotted file suffixes that activate the pass (e.g. `.rb`).
    pub suffixes: &'static [&'static str],
    /// The pass itself.
    pub resolve: ResolveFn,
}

/// Run every resolver whose suffix appears in `paths`, in registration order.
/// Behaviorally identical to the prior hand-wired sequence of suffix-gated
/// passes: same activation rule (suffix present) and execution order. Mirrors
/// graphify-py `run_language_resolvers`.
pub(super) fn run_language_resolvers(
    paths: &[PathBuf],
    nodes: &[Node],
    edges: &mut Vec<Edge>,
    raw_calls: &[RawCall],
    resolvers: &[LanguageResolver],
) {
    let present: HashSet<String> = paths
        .iter()
        .filter_map(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
        })
        .collect();
    for resolver in resolvers {
        if resolver.suffixes.iter().any(|s| present.contains(*s)) {
            (resolver.resolve)(paths, nodes, edges, raw_calls);
        }
    }
}

/// The default ordered resolver set: Swift (#1356), Python (#1446), Ruby (#1499),
/// Java (#1696), C# (#1609), C++ (#1547), then Objective-C (#1556). Order preserves the
/// prior inlined wiring; the passes are per-language and independently guarded
/// (each C++/ObjC pass also gates on the `RawCall` lang tag for the `.h`
/// overlap), so relative order is not observable.
#[must_use]
pub(super) fn default_resolvers() -> [LanguageResolver; 9] {
    [
        LanguageResolver {
            suffixes: &[".swift"],
            resolve: swift_pass,
        },
        LanguageResolver {
            suffixes: &[".py", ".pyi"],
            resolve: python_pass,
        },
        LanguageResolver {
            suffixes: &[".rb"],
            resolve: ruby_pass,
        },
        LanguageResolver {
            suffixes: &[".java"],
            resolve: java_pass,
        },
        LanguageResolver {
            suffixes: &[".cs"],
            resolve: csharp_pass,
        },
        LanguageResolver {
            suffixes: &[".cpp", ".cc", ".cxx", ".hpp", ".cu", ".cuh", ".metal", ".h"],
            resolve: cpp_pass,
        },
        LanguageResolver {
            suffixes: &[".m", ".mm", ".h"],
            resolve: objc_pass,
        },
        LanguageResolver {
            suffixes: &[".ts", ".tsx", ".js", ".jsx"],
            resolve: typescript_pass,
        },
        LanguageResolver {
            suffixes: &[".pas", ".pp", ".dpr", ".dpk", ".inc"],
            resolve: pascal_pass,
        },
    ]
}

fn swift_pass(paths: &[PathBuf], nodes: &[Node], edges: &mut Vec<Edge>, raw_calls: &[RawCall]) {
    // The Swift resolver re-parses each `.swift` file for its local type table,
    // so it needs the swift-only path subset (not the whole corpus).
    let swift_paths: Vec<PathBuf> = paths
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "swift"))
        .cloned()
        .collect();
    super::swift::resolve_swift_member_calls(&swift_paths, nodes, edges, raw_calls);
}

fn python_pass(_paths: &[PathBuf], nodes: &[Node], edges: &mut Vec<Edge>, raw_calls: &[RawCall]) {
    super::python::resolve_python_member_calls(nodes, edges, raw_calls);
}

fn pascal_pass(_paths: &[PathBuf], nodes: &[Node], edges: &mut Vec<Edge>, raw_calls: &[RawCall]) {
    super::pascal_resolution::resolve_pascal_inherited_calls(nodes, edges, raw_calls);
}

fn ruby_pass(_paths: &[PathBuf], nodes: &[Node], edges: &mut Vec<Edge>, raw_calls: &[RawCall]) {
    super::ruby::resolve_ruby_member_calls(nodes, edges, raw_calls);
}

fn java_pass(_paths: &[PathBuf], nodes: &[Node], edges: &mut Vec<Edge>, raw_calls: &[RawCall]) {
    super::java::resolve_java_member_calls(nodes, edges, raw_calls);
}

fn csharp_pass(_paths: &[PathBuf], nodes: &[Node], edges: &mut Vec<Edge>, raw_calls: &[RawCall]) {
    super::csharp::resolve_csharp_member_calls(nodes, edges, raw_calls);
}

fn cpp_pass(_paths: &[PathBuf], nodes: &[Node], edges: &mut Vec<Edge>, raw_calls: &[RawCall]) {
    super::cpp::resolve_cpp_member_calls(nodes, edges, raw_calls);
}

fn objc_pass(_paths: &[PathBuf], nodes: &[Node], edges: &mut Vec<Edge>, raw_calls: &[RawCall]) {
    super::objc::resolve_objc_member_calls(nodes, edges, raw_calls);
}

fn typescript_pass(
    _paths: &[PathBuf],
    nodes: &[Node],
    edges: &mut Vec<Edge>,
    raw_calls: &[RawCall],
) {
    super::typescript::resolve_typescript_member_calls(nodes, edges, raw_calls);
}
