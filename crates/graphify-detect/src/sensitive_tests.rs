//! Unit tests for [`crate::sensitive`].
//!
//! Each case names a real-world secret-leak pattern; the assertions document
//! exactly which path shapes the heuristic catches and which it deliberately
//! tolerates (e.g. `tokenizer.py`).

use super::*;
use std::path::Path;

/// `api_token.txt` should be flagged — `api_token` is a canonical secret stem.
#[test]
fn sensitive_flags_api_token_txt() {
    assert!(is_sensitive(Path::new("api_token.txt")));
}

/// `oauth_token.json` matches the OAuth secret heuristic.
#[test]
fn sensitive_flags_oauth_token_json() {
    assert!(is_sensitive(Path::new("oauth_token.json")));
}

/// Underscore-delimited `secret` stems are treated as sensitive.
#[test]
fn sensitive_flags_underscore_secret() {
    assert!(is_sensitive(Path::new("app_secret.yaml")));
}

/// `tokenizer.py` looks like it contains "token" but is a library file —
/// guard the heuristic so we do not flag standard library code.
#[test]
fn sensitive_does_not_flag_tokenizer_py() {
    assert!(!is_sensitive(Path::new("tokenizer.py")));
}

/// Same guard for `tokenize.py`.
#[test]
fn sensitive_does_not_flag_tokenize_py() {
    assert!(!is_sensitive(Path::new("tokenize.py")));
}

/// Files literally named `passwords.py` are secrets, regardless of extension.
#[test]
fn sensitive_flags_passwords_py() {
    assert!(is_sensitive(Path::new("passwords.py")));
}

/// `.ssh/` directory contents are always sensitive — SSH keys.
#[test]
fn sensitive_flags_ssh_dir() {
    assert!(is_sensitive(Path::new("/home/user/.ssh/id_rsa")));
}

/// Any path nested under `secrets/` is sensitive (directory-based rule).
#[test]
fn sensitive_flags_secrets_dir() {
    assert!(is_sensitive(Path::new("config/secrets/db.json")));
}

/// `token.txt` at the leaf is sensitive even with no parent context.
#[test]
fn sensitive_flags_token_txt() {
    assert!(is_sensitive(Path::new("token.txt")));
}

/// Canonical `credentials.json` filename — must be flagged.
#[test]
fn sensitive_flags_credentials_json() {
    assert!(is_sensitive(Path::new("credentials.json")));
}

/// Bare `credentials` (no extension) still flags via Stage-2 stem matching.
#[test]
fn sensitive_root_credentials_via_name_pattern() {
    assert!(is_sensitive(Path::new("credentials")));
}

/// Compound `secret_*` names are flagged regardless of the suffix word.
#[test]
fn sensitive_secret_handler_txt() {
    assert!(is_sensitive(Path::new("secret_handler.txt")));
}

/// `token_config.yaml` is flagged — `token` stem applies even before `config`.
#[test]
fn sensitive_token_config_yaml() {
    assert!(is_sensitive(Path::new("token_config.yaml")));
}
