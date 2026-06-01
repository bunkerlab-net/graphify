//! Shared test helpers for the `graphify-llm` integration tests.
#![allow(dead_code, unsafe_code)]

/// RAII guard that sets/restores process environment variables. Restored in
/// reverse order on drop. Tests that use it must be `#[serial]`.
pub struct EnvGuard {
    saved: Vec<(String, Option<String>)>,
}

impl Default for EnvGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl EnvGuard {
    #[must_use]
    pub fn new() -> Self {
        Self { saved: vec![] }
    }

    pub fn set(&mut self, k: &str, v: &str) -> &mut Self {
        self.saved.push((k.to_string(), std::env::var(k).ok()));
        // SAFETY: every test using EnvGuard is `#[serial]`, so no other thread
        // reads or writes the process environment concurrently with this mutation.
        unsafe { std::env::set_var(k, v) };
        self
    }

    pub fn unset(&mut self, k: &str) -> &mut Self {
        self.saved.push((k.to_string(), std::env::var(k).ok()));
        // SAFETY: see `set` — `#[serial]` execution rules out concurrent env access.
        unsafe { std::env::remove_var(k) };
        self
    }

    /// Unset every environment variable that `detect_backend` consults, so a
    /// test can drive the no-backend / custom-provider path without the host's
    /// real credentials leaking in. The list mirrors the keys checked by
    /// `backends::detect_backend_with` and `bedrock::credentials_appear_configured`
    /// (notably `AWS_CONTAINER_CREDENTIALS_FULL_URI`) — keep it in lockstep with
    /// them, and with `cli_no_backend()` in the binary's `tests/cli_commands.rs`.
    pub fn scrub_backends(&mut self) -> &mut Self {
        for key in [
            // Built-in API keys (gemini -> kimi -> claude -> openai -> deepseek).
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "MOONSHOT_API_KEY",
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "DEEPSEEK_API_KEY",
            // Bedrock credential-provider entry points.
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_PROFILE",
            "AWS_WEB_IDENTITY_TOKEN_FILE",
            "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
            "AWS_CONTAINER_CREDENTIALS_FULL_URI",
            // Ollama (checked last before custom providers).
            "OLLAMA_BASE_URL",
        ] {
            self.unset(key);
        }
        self
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, prev) in self.saved.drain(..).rev() {
            // SAFETY: see `set` — `#[serial]` execution rules out concurrent env access.
            match prev {
                Some(v) => unsafe { std::env::set_var(&k, &v) },
                None => unsafe { std::env::remove_var(&k) },
            }
        }
    }
}
