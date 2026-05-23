//! Pure helpers: URL detection, prompt construction, URL hashing.

use sha1::{Digest, Sha1};

use serde_json::Value;

use crate::constants::{FALLBACK_PROMPT, URL_PREFIXES};

/// Return `true` if `path` looks like a URL rather than a file path.
///
/// Matches the `http://`, `https://`, and `www.` prefixes used by the
/// Python reference.
#[must_use]
pub fn is_url(path: &str) -> bool {
    URL_PREFIXES.iter().any(|p| path.starts_with(p))
}

/// Build a domain hint for Whisper from god-node labels extracted from
/// the corpus.
///
/// Order of precedence:
/// 1. If `god_nodes` is empty → return the fallback prompt.
/// 2. If `GRAPHIFY_WHISPER_PROMPT` env var is set → return its value.
/// 3. Build a topic string from up to 5 node labels (chosen from the
///    first 10 god nodes).
/// 4. If no valid labels exist → return the fallback prompt.
#[must_use]
pub fn build_whisper_prompt(god_nodes: &[Value]) -> String {
    if god_nodes.is_empty() {
        return FALLBACK_PROMPT.to_string();
    }

    if let Ok(override_prompt) = std::env::var("GRAPHIFY_WHISPER_PROMPT")
        && !override_prompt.is_empty()
    {
        return override_prompt;
    }

    let labels: Vec<&str> = god_nodes
        .iter()
        .take(10)
        .filter_map(|n| n.get("label").and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .take(5)
        .collect();

    if labels.is_empty() {
        return FALLBACK_PROMPT.to_string();
    }

    let topics = labels.join(", ");
    format!("Technical discussion about {topics}. Use proper punctuation and paragraph breaks.")
}

/// SHA-1 hash of `url`, first 12 hex characters — used as a stable,
/// filesystem-safe filename prefix for downloaded audio.
pub(crate) fn url_hash_prefix(url: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(url.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)[..12].to_string()
}
