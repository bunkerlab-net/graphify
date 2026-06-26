//! Parity tests for sensitive-file detection.
//!
//! Mirrors `graphify-py/tests/test_detect.py` — `_is_sensitive` tests.
#![allow(clippy::expect_used)]
// `std::env::set_var` is unsafe in edition 2024 — test-only, serialised below.
#![allow(unsafe_code)]

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

// ── Generic keywords must be load-bearing (#436 / #718): a topic slug is not
// a secret store, but a keyword that ends the stem (or a short name) is. ──

#[test]
fn sensitive_does_not_flag_token_economics_note() {
    assert!(!is_sensitive(Path::new("token-economics-of-recall.md")));
}

#[test]
fn sensitive_does_not_flag_password_policy_discussion() {
    assert!(!is_sensitive(Path::new("password-policy-discussion.md")));
}

#[test]
fn sensitive_flags_keyword_at_end_of_long_name() {
    // Keyword as the final word names the file's contents — still a secret store.
    assert!(is_sensitive(Path::new("github-personal-access-token.txt")));
}

#[test]
fn sensitive_flags_my_private_key_txt() {
    // Multi-word keyword at end of stem: the end-of-stem check runs before word
    // counting, so splitting `private_key` on `_` cannot un-flag it.
    assert!(is_sensitive(Path::new("my_private_key.txt")));
}

#[test]
fn sensitive_flags_dotfile_token() {
    // Leading dot stripped before stem extraction; `.token` keeps its keyword.
    assert!(is_sensitive(Path::new(".token")));
}

#[test]
fn sensitive_flags_plural_tokens_txt() {
    assert!(is_sensitive(Path::new("tokens.txt")));
}

/// RAII guard that sets an env var and restores it on drop.
struct EnvGuard {
    key: &'static str,
    prev: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let prev = std::env::var(key).ok();
        // SAFETY: test-only, serialised via `#[serial_test::serial]`.
        unsafe { std::env::set_var(key, value) };
        Self { key, prev }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.prev {
            // SAFETY: test-only cleanup.
            Some(v) => unsafe { std::env::set_var(self.key, v) },
            None => unsafe { std::env::remove_var(self.key) },
        }
    }
}

#[test]
#[serial_test::serial(graphify_out_env)]
fn noise_dir_flags_default_graphify_out() {
    assert!(graphify_detect::is_noise_dir("graphify-out", None));
}

#[test]
#[serial_test::serial(graphify_out_env)]
fn noise_dir_honours_graphify_out_override() {
    // A custom GRAPHIFY_OUT dir must be skipped so it is never re-ingested as
    // source (#1423); a normal dir name is still walked.
    let _guard = EnvGuard::set("GRAPHIFY_OUT", "custom-out");
    assert!(graphify_detect::is_noise_dir("custom-out", None));
    assert!(!graphify_detect::is_noise_dir("src", None));
}
