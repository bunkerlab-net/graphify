//! Token estimation and file-packing into token-budget chunks.
//!
//! Extracted from `lib.rs` to isolate `estimate_file_tokens`,
//! `pack_chunks_by_tokens`, and `estimate_cost` — the budget-accounting layer
//! used before any backend call.

use std::path::{Path, PathBuf};

use crate::vision::{IMAGE_TOKEN_ESTIMATE, MAX_IMAGES_PER_CHUNK, is_vision_image};
use crate::{
    FILE_CHAR_CAP, LlmError, PER_FILE_OVERHEAD_CHARS, backends::backend_config, tokenizer,
};

/// Estimate the token cost of one file under `read_files` rules.
///
/// Raster images are not read as text; a vision model bills them at a roughly
/// fixed cost, so they estimate to a flat [`IMAGE_TOKEN_ESTIMATE`] rather than by
/// (binary) byte size (#1110).
#[must_use]
pub fn estimate_file_tokens(path: &Path) -> usize {
    if is_vision_image(path) {
        return IMAGE_TOKEN_ESTIMATE;
    }
    if let Ok(content) = std::fs::read_to_string(path) {
        let capped: String = content.chars().take(FILE_CHAR_CAP).collect();
        tokenizer::estimate_file_tokens(&capped, PER_FILE_OVERHEAD_CHARS)
    } else {
        // Fallback: use file size.
        let size = path
            .metadata()
            .map_or(0, |m| usize::try_from(m.len()).unwrap_or(usize::MAX));
        let chars = size.min(FILE_CHAR_CAP) + PER_FILE_OVERHEAD_CHARS;
        chars / tokenizer::CHARS_PER_TOKEN
    }
}

/// Pack files into token-budget chunks, grouped by parent directory.
///
/// # Errors
/// Returns an error if `token_budget` is zero.
pub fn pack_chunks_by_tokens(
    files: &[PathBuf],
    token_budget: usize,
) -> Result<Vec<Vec<PathBuf>>, LlmError> {
    if token_budget == 0 {
        return Err(LlmError::InvalidInput(
            "token_budget must be positive".to_string(),
        ));
    }

    // Group by parent directory (preserving order).
    let mut by_dir: indexmap::IndexMap<PathBuf, Vec<PathBuf>> = indexmap::IndexMap::new();
    for f in files {
        let parent = f.parent().unwrap_or(Path::new(".")).to_path_buf();
        by_dir.entry(parent).or_default().push(f.clone());
    }

    // Sort directories for deterministic output.
    by_dir.sort_keys();

    let mut chunks: Vec<Vec<PathBuf>> = Vec::new();
    let mut current: Vec<PathBuf> = Vec::new();
    let mut current_tokens: usize = 0;
    let mut current_images: usize = 0;

    for (_dir, dir_files) in &by_dir {
        for path in dir_files {
            let cost = estimate_file_tokens(path);
            let is_image = is_vision_image(path);
            // Close the chunk when adding the next file would overflow the token
            // budget OR push past the per-chunk image cap (#1110), so image-dense
            // corpora don't exceed provider per-request image limits.
            let over_budget = current_tokens + cost > token_budget;
            let over_images = is_image && current_images >= MAX_IMAGES_PER_CHUNK;
            if !current.is_empty() && (over_budget || over_images) {
                chunks.push(std::mem::take(&mut current));
                current_tokens = 0;
                current_images = 0;
            }
            current.push(path.clone());
            current_tokens += cost;
            current_images += usize::from(is_image);
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    Ok(chunks)
}

/// Estimate USD cost for a given token count.
///
/// Returns 0.0 for unknown backends.
#[must_use]
pub fn estimate_cost(backend: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let Some(cfg) = backend_config(backend) else {
        return 0.0;
    };
    // Allow precision loss: token counts are at most ~10^9, well within f64 range.
    #[allow(clippy::cast_precision_loss)]
    let cost = (input_tokens as f64 * cfg.pricing.input
        + output_tokens as f64 * cfg.pricing.output)
        / 1_000_000.0;
    cost
}
