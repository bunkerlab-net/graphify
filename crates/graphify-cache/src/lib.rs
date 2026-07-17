//! Content-addressed, per-file extraction cache for graphify.
//!
//! Stores and retrieves AST and semantic extraction results keyed by a
//! SHA-256 hash of file content (plus the file's root-relative path).
//! A process-wide stat index (`size + mtime_ns`) is used as a fastpath
//! to avoid re-hashing unmodified files on repeated runs.
//!
//! Ports `graphify-py/graphify/cache.py`.
//!
//! # Cache layout
//!
//! All output lives under `<root>/graphify-out/cache/` (overridable via the
//! `GRAPHIFY_OUT` environment variable):
//! - `ast/v{version}/<hash>.json` — AST extraction results, namespaced by
//!   graphify version (entries from other versions are swept on first use)
//! - `semantic/<hash>.json` — LLM/semantic extraction results (unversioned)
//! - `semantic-deep/<hash>.json` — `--mode deep` semantic results (unversioned, #1894)
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
pub use hash::{body_content, cached_word_count, file_hash};
pub use paths::{EXTRACTOR_VERSION, cache_dir, cache_dir_versioned};
pub use semantic::{
    SemanticCacheOptions, SemanticCacheSplit, check_semantic_cache, prune_semantic_cache,
    save_semantic_cache, semantic_kind,
};
pub use stat_index::{_reset_stat_index_for_tests, StatIndexFlushGuard, flush_stat_index};
pub use store::{
    cached_files, clear_cache, load_cached, load_cached_versioned, save_cached,
    save_cached_versioned,
};
