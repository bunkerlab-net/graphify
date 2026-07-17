//! JSONC parsing and tsconfig path-alias resolution.
//!
//! Mirrors Python `_strip_jsonc`, `_read_tsconfig_aliases`, `_load_tsconfig_aliases`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use indexmap::IndexMap;
use regex::Regex;

#[allow(clippy::expect_used)] // literal patterns; build cannot panic
static JSONC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)"(?:\\.|[^"\\])*"|/\*.*?\*/|//[^\n]*"#).expect("static jsonc regex")
});

#[allow(clippy::expect_used)] // literal pattern; build cannot panic
static TRAILING_COMMA_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r",(\s*[}\]])").expect("static trailing comma regex"));

/// Strip `//` line comments, `/* */` block comments, and trailing commas from
/// JSONC. Preserves string literals. Mirrors Python `_strip_jsonc`.
#[must_use]
pub fn strip_jsonc(text: &str) -> String {
    let stripped = JSONC_RE.replace_all(text, |caps: &regex::Captures<'_>| {
        let tok = caps.get(0).map_or("", |m| m.as_str());
        if tok.starts_with('"') {
            tok.to_string()
        } else {
            String::new()
        }
    });
    TRAILING_COMMA_RE.replace_all(&stripped, "$1").into_owned()
}

/// Alias prefix → ordered list of resolved base dirs. tsc tries each target in
/// declared order until one resolves on disk (#1531).
pub type AliasMap = IndexMap<String, Vec<String>>;

/// A matched tsconfig alias: `((rank, -prefix_len), captured_segment, is_wildcard)`.
type AliasMatch = ((i32, i32), String, bool);

// Cache: tsconfig path string → alias map
static ALIAS_CACHE: LazyLock<Mutex<IndexMap<String, AliasMap>>> =
    LazyLock::new(|| Mutex::new(IndexMap::new()));

/// Recursively read path aliases from a tsconfig, following `extends` chains.
///
/// Mirrors Python `_read_tsconfig_aliases`.
pub fn read_tsconfig_aliases<S: std::hash::BuildHasher>(
    tsconfig: &Path,
    base_dir: &Path,
    seen: &mut HashSet<String, S>,
) -> AliasMap {
    let key = tsconfig.to_string_lossy().into_owned();
    if seen.contains(&key) {
        return IndexMap::new();
    }
    seen.insert(key);

    let Ok(raw) = std::fs::read_to_string(tsconfig) else {
        return IndexMap::new();
    };

    let data: serde_json::Value =
        match serde_json::from_str(&raw).or_else(|_| serde_json::from_str(&strip_jsonc(&raw))) {
            Ok(v) => v,
            Err(_) => return IndexMap::new(),
        };

    let mut aliases: AliasMap = IndexMap::new();

    // Follow extends chain. TypeScript 5.0 allows `extends` to be either a
    // string or an array of paths; for an array, parents are processed in
    // order with later entries overriding earlier ones (the extending config's
    // own `paths` still wins over all parents). Ports graphify-py #1017 — the
    // previous string-only handling raised an error on array `extends`, which
    // dropped every file whose imports relied on those aliases.
    let extends_list: Vec<&str> = match data.get("extends") {
        Some(serde_json::Value::String(s)) => vec![s.as_str()],
        Some(serde_json::Value::Array(arr)) => {
            arr.iter().filter_map(serde_json::Value::as_str).collect()
        }
        _ => Vec::new(),
    };
    for ext in extends_list {
        // Skip scoped npm package configs (e.g. `@tsconfig/svelte`) — not on disk.
        if ext.is_empty() || ext.starts_with('@') {
            continue;
        }
        let mut extended = base_dir.join(ext);
        if extended.extension().is_none() {
            extended = extended.with_extension("json");
        }
        if extended.exists() {
            aliases.extend(read_tsconfig_aliases(
                &extended,
                extended.parent().unwrap_or(base_dir),
                seen,
            ));
        }
    }

    // tsconfig `paths` resolve relative to `baseUrl` (itself relative to the
    // tsconfig's directory), not the tsconfig directory directly. Honouring
    // baseUrl is required for the common monorepo / NestJS layout where baseUrl
    // points at a subdirectory (e.g. baseUrl "./src" with "@services/*":
    // ["services/*"] must resolve to <dir>/src/services). Defaults to "." so
    // configs without baseUrl keep the TS 4.1+ behaviour (#ec04152).
    let compiler_options = data.get("compilerOptions");
    let base_url = compiler_options
        .and_then(|o| o.get("baseUrl"))
        .and_then(serde_json::Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(".");
    let paths_base = base_dir.join(base_url);

    if let Some(paths) = compiler_options
        .and_then(|o| o.get("paths"))
        .and_then(serde_json::Value::as_object)
    {
        for (alias, targets) in paths {
            let Some(arr) = targets.as_array() else {
                continue;
            };
            if arr.is_empty() {
                continue;
            }
            // Preserve the `*` token in BOTH the alias key and each target pattern
            // (declared order kept, tsc tries each until one resolves on disk).
            // The resolver substitutes the captured segment, then normalises the
            // concrete path (#927/#1531). Empty / non-string entries are skipped.
            let target_patterns: Vec<String> = arr
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|t| !t.is_empty())
                .map(|t| paths_base.join(t).to_string_lossy().into_owned())
                .collect();
            if !target_patterns.is_empty() {
                aliases.insert(alias.clone(), target_patterns);
            }
        }
    }

    aliases
}

/// Lexically normalise a path (collapse `.`, resolve `..` where possible),
/// mirroring Python's `os.path.normpath`. Used so a `baseUrl` like `./src`
/// does not leave a `.` component in the resolved alias target.
pub(crate) fn normpath(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                // Pop only a real (Normal) trailing component. At the root of an
                // absolute path `..` is a no-op — `os.path.normpath` never lets
                // an absolute path climb above its root, and a bare `out.pop()`
                // here would strip the root and turn `/a/../..` into a stray
                // `..`, corrupting alias resolution. For a relative path with
                // nothing poppable, accumulate the `..`.
                if matches!(out.components().next_back(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !out.is_absolute() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// Walk up from `start_dir` to find `tsconfig.json` and return its path
/// aliases. Result is cached by tsconfig path. Mirrors Python
/// `_load_tsconfig_aliases`.
///
/// # Panics
///
/// Panics if the internal alias cache mutex is poisoned (which would indicate
/// a prior panic in another thread holding the lock).
#[must_use]
pub fn load_tsconfig_aliases(start_dir: &Path) -> AliasMap {
    let current = match start_dir.canonicalize() {
        Ok(c) => c,
        Err(_) => start_dir.to_path_buf(),
    };

    // Walk up directory chain
    let mut candidate_dir: Option<&Path> = Some(&current);
    while let Some(dir) = candidate_dir {
        let tsconfig = dir.join("tsconfig.json");
        if tsconfig.exists() {
            let key = tsconfig.to_string_lossy().into_owned();
            // Check cache first (avoid re-reading)
            {
                #[allow(clippy::expect_used)] // mutex poison = programming error
                let cache = ALIAS_CACHE.lock().expect("alias cache mutex");
                if let Some(aliases) = cache.get(&key) {
                    return aliases.clone();
                }
            }
            let mut seen = HashSet::new();
            let aliases = read_tsconfig_aliases(&tsconfig, dir, &mut seen);
            #[allow(clippy::expect_used)] // mutex poison = programming error
            ALIAS_CACHE
                .lock()
                .expect("alias cache mutex")
                .insert(key, aliases.clone());
            return aliases;
        }
        candidate_dir = dir.parent();
    }
    IndexMap::new()
}

/// `(specificity, captured, is_wildcard)` when `pattern` matches `raw`, else
/// `None`. Specificity orders exact matches (rank 0) before single-`*` wildcards
/// (rank 1, longest literal prefix) before the legacy directory-prefix form
/// (rank 2, longest prefix); the second tuple element is the negated prefix
/// length so a longer prefix sorts first. Mirrors `_match_tsconfig_alias`.
fn match_tsconfig_alias(raw: &str, pattern: &str) -> Option<AliasMatch> {
    let neg = |n: usize| -i32::try_from(n).unwrap_or(i32::MAX);
    if pattern.contains('*') {
        if pattern.matches('*').count() != 1 {
            return None;
        }
        let (prefix, suffix) = pattern.split_once('*')?;
        if !raw.starts_with(prefix) || !raw.ends_with(suffix) {
            return None;
        }
        let end = if suffix.is_empty() {
            raw.len()
        } else {
            raw.len() - suffix.len()
        };
        if end < prefix.len() {
            return None;
        }
        return Some((
            (1, neg(prefix.len())),
            raw[prefix.len()..end].to_string(),
            true,
        ));
    }
    if raw == pattern {
        return Some(((0, neg(pattern.len())), String::new(), false));
    }
    let prefix = pattern.trim_end_matches('/');
    if !prefix.is_empty() && raw.starts_with(&format!("{prefix}/")) {
        return Some((
            (2, neg(prefix.len())),
            raw[prefix.len()..].trim_start_matches('/').to_string(),
            false,
        ));
    }
    None
}

/// Resolve `raw` against tsconfig path aliases (#927/#1531). Pick the single
/// most-specific matching pattern (see [`match_tsconfig_alias`]), then try that
/// pattern's targets in declared order, substituting the captured segment into a
/// wildcard target and returning the first candidate that resolves to a real file
/// on disk (tsc parity). If none exist, return the first candidate (no false edge
/// fabricated). Returns `None` when no pattern matches.
///
/// Mirrors Python `_resolve_tsconfig_alias`.
#[must_use]
pub fn resolve_tsconfig_alias(raw: &str, aliases: &AliasMap) -> Option<PathBuf> {
    let mut best: Option<(AliasMatch, &Vec<String>)> = None;
    for (pattern, targets) in aliases {
        if let Some(m) = match_tsconfig_alias(raw, pattern)
            && best.as_ref().is_none_or(|(bm, _)| m.0 < bm.0)
        {
            best = Some((m, targets));
        }
    }
    let ((_, captured, is_wild), targets) = best?;
    let mut first: Option<PathBuf> = None;
    for target in targets {
        let cand = if is_wild {
            // tsc substitutes the star only when the captured segment is non-empty.
            let substituted = if captured.is_empty() {
                target.clone()
            } else {
                target.replacen('*', &captured, 1)
            };
            normpath(Path::new(&substituted))
        } else if captured.is_empty() {
            PathBuf::from(target)
        } else {
            normpath(&Path::new(target).join(&captured))
        };
        let resolved = resolve_js_module_path(&cand);
        if resolved.is_file() {
            return Some(resolved);
        }
        if first.is_none() {
            first = Some(cand);
        }
    }
    first
}

/// Resolve a JS/TS-style import specifier path to an actual file on disk.
///
/// Mirrors Python `_resolve_js_module_path`.
#[must_use]
pub fn resolve_js_module_path(p: &Path) -> PathBuf {
    if p.is_file() {
        return p.to_path_buf();
    }
    // TS ESM convention: .js → .ts
    if p.extension().is_some_and(|e| e == "js") {
        let c = p.with_extension("ts");
        if c.is_file() {
            return c;
        }
    }
    if p.extension().is_some_and(|e| e == "jsx") {
        let c = p.with_extension("tsx");
        if c.is_file() {
            return c;
        }
    }
    // Try appending extensions
    let exts = [
        ".ts", ".tsx", ".mts", ".cts", ".svelte", ".js", ".jsx", ".mjs", ".cjs",
    ];
    if let Some(name) = p.file_name().map(|n| n.to_string_lossy().into_owned()) {
        for ext in exts {
            let c = p.with_file_name(format!("{name}{ext}"));
            if c.is_file() {
                return c;
            }
        }
    }
    // Directory imports. Mirrors graphify-py `_JS_INDEX_FILES` (which adds
    // index.svelte + index.mjs for Svelte/ESM barrels) but ALSO adds
    // index.mts/index.cts. DIVERGENCE from graphify-py: it lists `.mts`/`.cts`
    // in `_JS_RESOLVE_EXTS` yet omits them from `_JS_INDEX_FILES`, so a directory
    // import can't resolve to a `.mts`/`.cts` barrel even though its `.mjs`
    // counterpart resolves — an obvious oversight fixed here per AGENTS.md (bugs
    // in the reference are not requirements). Order matches `_JS_RESOLVE_EXTS`.
    if p.is_dir() {
        for idx in [
            "index.ts",
            "index.tsx",
            "index.mts",
            "index.cts",
            "index.svelte",
            "index.js",
            "index.jsx",
            "index.mjs",
        ] {
            let c = p.join(idx);
            if c.is_file() {
                return c;
            }
        }
    }
    p.to_path_buf()
}
