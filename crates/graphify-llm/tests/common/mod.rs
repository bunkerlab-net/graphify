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
