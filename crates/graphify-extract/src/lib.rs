//! Tree-sitter structural extraction for 26+ programming languages.
//!
//! Ports `graphify-py/graphify/extract.py`.
//!
//! # Module layout
//!
//! - **`extractors/`** — one file per language (e.g. `python.rs`, `rust_lang.rs`), each
//!   exposing an `extract_<lang>` function, plus `multi.rs` which drives parallel
//!   multi-file extraction and cross-file import resolution.
//! - **`generic/`** — language-agnostic tree-sitter walking logic (`walk.rs`, `calls.rs`,
//!   `inherit.rs`, `js_extra.rs`) and the shared configuration types (`config.rs`).
//! - **`lang_configs.rs`** — pre-built [`generic::LangConfig`] constants (one per language).
//! - **`types.rs`** — core graph types: [`Node`], [`Edge`], [`RawCall`], [`FileResult`], [`ExtractOutput`].
//! - **`import_handlers.rs`** — language-specific import-edge builders.
//! - **`ids.rs`** — deterministic node-ID helpers.
//! - **`tsconfig.rs`** — TypeScript `tsconfig.json` alias resolution.

mod builtins;
pub mod error;
mod forward_refs;
pub mod generic;
pub mod ids;
pub mod import_handlers;
pub mod lang_configs;
pub mod postprocess;
pub mod symbol_resolution;
pub mod tsconfig;
pub mod types;
pub mod workspace;

// Language-specific extractors
mod extractors;

pub use error::ExtractError;
pub use extractors::extract;
pub use extractors::mcp::MCP_CONFIG_FILENAMES;
pub use extractors::{
    extract_astro, extract_bash, extract_blade, extract_c, extract_cpp, extract_csharp,
    extract_csproj, extract_dart, extract_delphi_form, extract_dm, extract_dmf, extract_dmi,
    extract_dmm, extract_elixir, extract_fortran, extract_go, extract_groovy, extract_java,
    extract_js, extract_json, extract_julia, extract_kotlin, extract_lazarus_form,
    extract_lazarus_package, extract_lua, extract_markdown, extract_mcp_config, extract_objc,
    extract_pascal, extract_php, extract_powershell, extract_python, extract_razor, extract_ruby,
    extract_rust, extract_scala, extract_sln, extract_slnx, extract_sql, extract_svelte,
    extract_swift, extract_verilog, extract_zig, is_mcp_config_path, resolve_cpp_path,
};
pub use ids::{file_node_id, file_stem, make_id, make_id1};
pub use types::{Edge, ExtractOutput, FileResult, Node, RawCall};
