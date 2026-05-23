//! Per-file extraction cache. Ports `graphify-py/graphify/cache.py`.
//!
//! Cache layout under `<root>/graphify-out/cache/`:
//! - `ast/<hash>.json` — AST extraction results
//! - `semantic/<hash>.json` — LLM/semantic extraction results
//! - `stat-index.json` — file stat fastpath
//!
//! `GRAPHIFY_OUT` env var overrides the output dir name (relative or
//! absolute).

mod error;
mod hash;
mod paths;
mod semantic;
mod stat_index;
mod store;

pub use error::CacheError;
pub use hash::{body_content, file_hash};
pub use paths::cache_dir;
pub use semantic::{SemanticCacheSplit, check_semantic_cache, save_semantic_cache};
pub use stat_index::{
    _reset_stat_index_for_tests, ensure_atexit_flush_registered, flush_stat_index,
};
pub use store::{cached_files, clear_cache, load_cached, save_cached};
