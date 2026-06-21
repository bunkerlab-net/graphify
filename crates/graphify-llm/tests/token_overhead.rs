//! Guards the per-file token-estimation overhead against the Python
//! `_PER_FILE_OVERHEAD_CHARS` contract (graphify-py issue #1210: each file is
//! wrapped in an `<untrusted_source path=... sha256=...>` block).

use graphify_llm::{PER_FILE_OVERHEAD_CHARS, estimate_file_tokens};

#[test]
fn per_file_overhead_matches_python_untrusted_source_wrapper() {
    // Python `_PER_FILE_OVERHEAD_CHARS = 160`. An undercount here makes the
    // chunk packer pack more files per chunk than graphify-py for the same
    // token budget, diverging the semantic-extraction request boundaries.
    assert_eq!(PER_FILE_OVERHEAD_CHARS, 160);
}

#[test]
fn empty_file_estimate_is_overhead_only() -> Result<(), Box<dyn std::error::Error>> {
    let tmp = tempfile::tempdir()?;
    let empty = tmp.path().join("empty.md");
    std::fs::write(&empty, "")?;
    // An empty file contributes zero content tokens, so its estimate is exactly
    // the per-file overhead expressed in tokens (overhead chars / 4).
    assert_eq!(estimate_file_tokens(&empty), PER_FILE_OVERHEAD_CHARS / 4);
    Ok(())
}
