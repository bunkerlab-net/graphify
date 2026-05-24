//! Parity tests for sensitive-file detection.
//!
//! Mirrors `graphify-py/tests/test_detect.py` — `_is_sensitive` tests.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use graphify_detect::is_sensitive;
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
