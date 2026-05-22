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
    // SAFETY: all patterns above are known-good literals.
    #[allow(clippy::unwrap_used)]
    patterns.iter().map(|p| Regex::new(p).unwrap()).collect()
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

/// Return `true` if `text` contains `keyword` at a word boundary matching the
/// Python lookaround semantics:
/// - Not preceded by `[a-zA-Z0-9]`
/// - Not followed by `[a-zA-Z]`
///
/// This is a case-insensitive check.
fn word_boundary_match(text: &str, keyword: &str) -> bool {
    let lower = text.to_lowercase();
    let kw = keyword.to_lowercase();
    let kw_len = kw.len();
    let bytes = lower.as_bytes();
    let kw_bytes = kw.as_bytes();

    let mut i = 0;
    while i + kw_len <= bytes.len() {
        if bytes[i..i + kw_len] == *kw_bytes {
            // Check lookbehind: char before match must NOT be [a-zA-Z0-9]
            let pre_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
            // Check lookahead: char after match must NOT be [a-zA-Z]
            let post_ok = i + kw_len >= bytes.len() || !bytes[i + kw_len].is_ascii_alphabetic();
            if pre_ok && post_ok {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn has_credential_keyword(name: &str) -> bool {
    CREDENTIAL_KEYWORDS
        .iter()
        .any(|kw| word_boundary_match(name, kw))
}

fn has_token_keyword(name: &str) -> bool {
    TOKEN_KEYWORDS
        .iter()
        .any(|kw| word_boundary_match(name, kw))
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
    if has_credential_keyword(name) {
        return true;
    }
    if has_token_keyword(name) {
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
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn sensitive_flags_api_token_txt() {
        assert!(is_sensitive(Path::new("api_token.txt")));
    }

    #[test]
    fn sensitive_flags_oauth_token_json() {
        assert!(is_sensitive(Path::new("oauth_token.json")));
    }

    #[test]
    fn sensitive_flags_underscore_secret() {
        assert!(is_sensitive(Path::new("app_secret.yaml")));
    }

    #[test]
    fn sensitive_does_not_flag_tokenizer_py() {
        assert!(!is_sensitive(Path::new("tokenizer.py")));
    }

    #[test]
    fn sensitive_does_not_flag_tokenize_py() {
        assert!(!is_sensitive(Path::new("tokenize.py")));
    }

    #[test]
    fn sensitive_flags_passwords_py() {
        assert!(is_sensitive(Path::new("passwords.py")));
    }

    #[test]
    fn sensitive_flags_ssh_dir() {
        assert!(is_sensitive(Path::new("/home/user/.ssh/id_rsa")));
    }

    #[test]
    fn sensitive_flags_secrets_dir() {
        assert!(is_sensitive(Path::new("config/secrets/db.json")));
    }

    #[test]
    fn sensitive_flags_token_txt() {
        assert!(is_sensitive(Path::new("token.txt")));
    }

    #[test]
    fn sensitive_flags_credentials_json() {
        assert!(is_sensitive(Path::new("credentials.json")));
    }

    #[test]
    fn sensitive_root_credentials_via_name_pattern() {
        // Stage 1 sees parts[:-1] = [] for a bare filename → only Stage 2 fires.
        // "credentials" matches the credential stem pattern.
        assert!(is_sensitive(Path::new("credentials")));
    }

    #[test]
    fn sensitive_secret_handler_txt() {
        assert!(is_sensitive(Path::new("secret_handler.txt")));
    }

    #[test]
    fn sensitive_token_config_yaml() {
        assert!(is_sensitive(Path::new("token_config.yaml")));
    }
}
