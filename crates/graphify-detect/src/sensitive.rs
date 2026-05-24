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

/// Match `keyword` against `lowered_bytes` with Python lookaround semantics:
/// - Not preceded by `[a-zA-Z0-9]`
/// - Not followed by `[a-zA-Z]`
///
/// Caller pre-lowercases the text exactly once per filename and passes its
/// bytes. The keyword arrays ([`CREDENTIAL_KEYWORDS`], [`TOKEN_KEYWORDS`])
/// are already lowercase, so no lowercase work happens here at all.
fn word_boundary_match_bytes(lowered_bytes: &[u8], keyword: &str) -> bool {
    let kw_bytes = keyword.as_bytes();
    let kw_len = kw_bytes.len();
    if kw_len == 0 || lowered_bytes.len() < kw_len {
        return false;
    }
    let mut i = 0;
    while i + kw_len <= lowered_bytes.len() {
        if &lowered_bytes[i..i + kw_len] == kw_bytes {
            let pre_ok = i == 0 || !lowered_bytes[i - 1].is_ascii_alphanumeric();
            let post_ok = i + kw_len >= lowered_bytes.len()
                || !lowered_bytes[i + kw_len].is_ascii_alphabetic();
            if pre_ok && post_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Return `true` if this file likely contains secrets and should be skipped.
///
/// Stage 1: any **parent** directory is a known secrets dir (`parts[:-1]`).
/// Stage 2: filename pattern match.
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
    // Stage 2: filename pattern match
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if SIMPLE_PATTERNS.iter().any(|p| p.is_match(name)) {
        return true;
    }
    // Lowercase the filename exactly once and reuse for every keyword scan.
    // The keyword arrays are pre-lowercased compile-time constants, so the
    // hot loop does no String allocation at all.
    let lowered = name.to_ascii_lowercase();
    let lowered_bytes = lowered.as_bytes();
    if CREDENTIAL_KEYWORDS
        .iter()
        .any(|kw| word_boundary_match_bytes(lowered_bytes, kw))
    {
        return true;
    }
    if TOKEN_KEYWORDS
        .iter()
        .any(|kw| word_boundary_match_bytes(lowered_bytes, kw))
    {
        return true;
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
            "__snapshots__",
            "snapshots",
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

/// Return `true` if this directory name looks like a venv, cache, or dep dir.
#[must_use]
pub fn is_noise_dir(name: &str) -> bool {
    if SKIP_DIRS.contains(name) {
        return true;
    }
    if name.ends_with("_venv") || name.ends_with("_env") {
        return true;
    }
    if name.ends_with(".egg-info") {
        return true;
    }
    false
}

#[cfg(test)]
#[path = "sensitive_tests.rs"]
mod sensitive_tests;
