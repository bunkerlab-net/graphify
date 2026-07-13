//! Sensitive-file detection and noise-directory filtering.
//!
//! Ports `_is_sensitive`, `_is_noise_dir`, `_SKIP_DIRS`, and `_SKIP_FILES`
//! from `graphify-py/graphify/detect.py`.
//!
//! # Lookbehind/lookahead workaround
//!
//! The `regex` crate does not support lookahead/lookbehind assertions. The
//! Python patterns use `(?<![a-zA-Z0-9])..(?![a-zA-Z])` to match keyword
//! stems that are not immediately preceded or followed by alphanumerics.
//! We implement these checks manually via [`word_boundary_match`].

use std::path::Path;

use crate::extensions::{FileType, classify_file};

use regex::Regex;

/// Directory names that always contain personal/secret material.
/// Checked against path ancestors (not the filename itself).
static SENSITIVE_DIRS: std::sync::LazyLock<std::collections::HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            ".ssh",
            ".gnupg",
            ".aws",
            ".gcloud",
            "secrets",
            ".secrets",
            "credentials",
        ]
        .into_iter()
        .collect()
    });

/// Data/serialization extensions that commonly ARE secret stores when their name
/// hits a generic keyword (`credentials.json`, `secrets.yaml`, `token.toml`).
/// These stay subject to the Stage-3 keyword drop even though some route through
/// the CODE path for manifest parsing — only real programming-language source is
/// exempt (#1666). Extensions carry the leading dot to match `Path::extension`
/// after re-prefixing.
static SECRET_PRONE_DATA_EXTS: std::sync::LazyLock<std::collections::HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            ".json",
            ".yaml",
            ".yml",
            ".toml",
            ".ini",
            ".cfg",
            ".conf",
            ".config",
            ".xml",
            ".properties",
            ".env",
            ".txt",
        ]
        .into_iter()
        .collect()
    });

// ── Simple regex patterns (no lookarounds needed) ────────────────────────────

/// Patterns that can use straight regex without lookaround assertions.
static SIMPLE_PATTERNS: std::sync::LazyLock<Vec<Regex>> = std::sync::LazyLock::new(|| {
    let patterns = [
        // .env / .envrc (with optional suffix)
        r"(?i)(^|[\\/])\.(env|envrc)(\.|$)",
        // Key material suffixes
        r"(?i)\.(pem|key|p12|pfx|cert|crt|der|p8)$",
        // SSH key filenames
        r"(id_rsa|id_dsa|id_ecdsa|id_ed25519)(\.pub)?$",
        // Config files that commonly store credentials
        r"(?i)(\.netrc|\.pgpass|\.htpasswd)$",
        // Cloud credential files
        r"(?i)(aws_credentials|gcloud_credentials|service\.account)",
    ];
    #[allow(clippy::expect_used)]
    let compiled = patterns
        .iter()
        .map(|p| Regex::new(p).expect("literal patterns are valid"))
        .collect();
    compiled
});

// ── Word-boundary keyword patterns ───────────────────────────────────────────

/// Keywords where a match preceded by a non-alnum char (or start) and not
/// followed by an alpha char indicates a sensitive file.
///
/// Maps to Python:
/// `(?<![a-zA-Z0-9])(credential|secret|passwd|password|private_key)s?(?![a-zA-Z])`
const CREDENTIAL_KEYWORDS: &[&str] = &[
    "credential",
    "credentials",
    "secret",
    "secrets",
    "passwd",
    "password",
    "passwords",
    "private_key",
    "private_keys",
];

/// Token keywords — kept separate per Python comment to avoid "tokenizer"/"tokenize".
/// Maps to Python: `(?<![a-zA-Z0-9])tokens?(?![a-zA-Z])`
const TOKEN_KEYWORDS: &[&str] = &["token", "tokens"];

/// Scan `lowered_bytes` for any keyword in `group` using Python lookaround
/// semantics (not preceded by `[a-zA-Z0-9]`, not followed by `[a-zA-Z]`),
/// returning `(any_hit, any_match_ends_at_end)`.
///
/// `any_match_ends_at_end` reports whether some accepted match ends exactly at
/// the end of the buffer — i.e. the keyword is the trailing word, which is what
/// distinguishes a credential store (`api_token`) from a topic slug
/// (`token-economics`). The keyword arrays already include plural forms so the
/// regex `(...)s?` suffix is covered.
fn group_match(lowered_bytes: &[u8], group: &[&str]) -> (bool, bool) {
    let n = lowered_bytes.len();
    let mut any_hit = false;
    let mut ends_at_end = false;
    for keyword in group {
        let kw = keyword.as_bytes();
        let kw_len = kw.len();
        if kw_len == 0 || n < kw_len {
            continue;
        }
        let mut i = 0;
        while i + kw_len <= n {
            if &lowered_bytes[i..i + kw_len] == kw {
                let pre_ok = i == 0 || !lowered_bytes[i - 1].is_ascii_alphanumeric();
                let end = i + kw_len;
                let post_ok = end >= n || !lowered_bytes[end].is_ascii_alphabetic();
                if pre_ok && post_ok {
                    any_hit = true;
                    if end == n {
                        ends_at_end = true;
                    }
                }
            }
            i += 1;
        }
    }
    (any_hit, ends_at_end)
}

/// True if a generic secret keyword appears *load-bearing* in `name`.
///
/// Secret-store files name their contents, and in English compounds the content
/// noun is the head, which comes last (`github-personal-access-token`,
/// `api_token`). A keyword that is neither at the end of the stem nor in a short
/// (<=2 word) name is a topic word in a descriptive slug
/// (`token-economics-of-recall.md`, `password-policy-discussion.md`) and must
/// not silently drop the file from the graph (#436, #718).
fn generic_keyword_hit(name: &str) -> bool {
    // Stem = name up to the first dot, ignoring leading dots so dotfiles like
    // `.token` keep their keyword.
    let stem = name.trim_start_matches('.');
    let stem = stem.split('.').next().unwrap_or("");
    if stem.is_empty() {
        return false;
    }
    let lowered = stem.to_ascii_lowercase();
    let bytes = lowered.as_bytes();
    let word_count = stem
        .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
        .filter(|w| !w.is_empty())
        .count();
    for group in [CREDENTIAL_KEYWORDS, TOKEN_KEYWORDS] {
        let (hit, ends_at_end) = group_match(bytes, group);
        if ends_at_end {
            return true; // keyword ends the stem -> names the contents
        }
        if hit && word_count <= 2 {
            return true; // short name like token_config / secret_handler
        }
    }
    false
}

/// Return `true` if this file likely contains secrets and should be skipped.
///
/// Stage 1: any **parent** directory is a known secrets dir (`parts[:-1]`).
/// Stage 2: specific filename patterns (extensions, credential-store names) —
///          always apply.
/// Stage 3: generic keywords (`token`, `secret`, …) — only when load-bearing
///          in the filename, so a topic slug is not mistaken for a credential.
#[must_use]
pub fn is_sensitive(path: &Path) -> bool {
    // Stage 1: check parent directories (all components except the last)
    let components: Vec<_> = path.components().collect();
    let parent_components = if components.len() > 1 {
        &components[..components.len() - 1]
    } else {
        &components[..0]
    };
    for comp in parent_components {
        if let Some(s) = comp.as_os_str().to_str()
            && SENSITIVE_DIRS.contains(s)
        {
            return true;
        }
    }
    // Stage 2: specific filename patterns
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if SIMPLE_PATTERNS.iter().any(|p| p.is_match(name)) {
        return true;
    }
    // Stage 3: generic keywords, only when load-bearing in the name. Do NOT let a
    // bare-name keyword silently drop a genuine programming-language source file:
    // a .rb/.py named `device_token` or `passwords_controller` is a module, not a
    // secret store (#1666). Data/config formats (.json, .yaml, .toml, ...) are
    // deliberately NOT exempt even though .json routes through the CODE path for
    // manifest parsing, because `credentials.json` / `secrets.yaml` are exactly
    // the secret stores this stage must catch. The Stage 2 patterns still apply
    // to everything regardless of extension.
    if generic_keyword_hit(name) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| format!(".{}", e.to_ascii_lowercase()))
            .unwrap_or_default();
        let is_source_code = classify_file(path) == Some(FileType::Code)
            && !SECRET_PRONE_DATA_EXTS.contains(ext.as_str());
        return !is_source_code;
    }
    false
}

/// Directory names to always skip.
pub static SKIP_DIRS: std::sync::LazyLock<std::collections::HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            "venv",
            ".venv",
            "env",
            ".env",
            "node_modules",
            "__pycache__",
            ".git",
            "dist",
            "build",
            "target",
            "out",
            "site-packages",
            "lib64",
            ".pytest_cache",
            ".mypy_cache",
            ".ruff_cache",
            ".tox",
            ".eggs",
            "*.egg-info",
            "graphify-out",
            "coverage",
            "lcov-report",
            "visual-tests",
            "visual-test",
            "__snapshots__", // "snapshots" (bare) is gated in is_noise_dir (#1666)
            "storybook-static",
            "dist-protected",
            ".next",
            ".nuxt",
            ".turbo",
            ".angular",
            ".idea",
            ".cache",
            ".parcel-cache",
            ".svelte-kit",
            ".terraform",
            ".serverless",
            ".graphify",
            ".worktrees",
        ]
        .into_iter()
        .collect()
    });

/// Large generated lock / sum files that are never useful to extract.
pub static SKIP_FILES: std::sync::LazyLock<std::collections::HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            "package-lock.json",
            "yarn.lock",
            "pnpm-lock.yaml",
            "Cargo.lock",
            "poetry.lock",
            "Gemfile.lock",
            "composer.lock",
            "go.sum",
            "go.work.sum",
        ]
        .into_iter()
        .collect()
    });

/// Return `true` if this directory name looks like a venv, cache, dep, or
/// snapshot artifact dir.
///
/// `dir_path` is the directory's own path (when available), used for two rules:
/// a directory named `worktrees` nested directly inside a dotted directory
/// (`.claude/worktrees/`, `.git/worktrees/`) is noise (#1023); and a bare
/// `snapshots` dir is a Jest/Vitest artifact only when it holds `*.snap` files or
/// lives directly under a JS test root — elsewhere it is often real source
/// (`app/services/snapshots/`), so pruning by name silently dropped legitimate
/// code (#1666). `__snapshots__` stays unconditionally pruned via `SKIP_DIRS`.
#[must_use]
pub fn is_noise_dir(name: &str, dir_path: Option<&Path>) -> bool {
    // `SKIP_DIRS` already includes the literal "graphify-out"; also skip a
    // custom output dir so `GRAPHIFY_OUT` is never re-ingested as source (#1423).
    if SKIP_DIRS.contains(name) || name == graphify_security::graphify_out_name() {
        return true;
    }
    let parent_name = dir_path
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|n| n.to_str());
    if name == "snapshots" {
        // Prune only when it looks like an actual JS/Vitest snapshot dir.
        let Some(dir) = dir_path else {
            return false; // cannot verify; keep a possibly-real code dir
        };
        if matches!(parent_name, Some("__tests__" | "__test__")) {
            return true;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry
                    .path()
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("snap"))
                {
                    return true;
                }
            }
        }
        return false;
    }
    if name.ends_with("_venv") || name.ends_with("_env") {
        return true;
    }
    if name.ends_with(".egg-info") {
        return true;
    }
    if name == "worktrees" && parent_name.is_some_and(|p| p.starts_with('.')) {
        return true;
    }
    false
}

#[cfg(test)]
#[path = "sensitive_tests.rs"]
mod sensitive_tests;
