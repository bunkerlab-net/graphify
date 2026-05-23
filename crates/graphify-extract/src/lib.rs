//! Tree-sitter structural extraction for 26+ programming languages.
//!
//! Ports `graphify-py/graphify/extract.py`.

pub mod error;
pub mod generic;
pub mod ids;
pub mod import_handlers;
pub mod lang_configs;
pub mod tsconfig;
pub mod types;

// Language-specific extractors
mod extractors;

pub use error::ExtractError;
pub use extractors::extract;
pub use extractors::{
    extract_astro, extract_bash, extract_blade, extract_c, extract_cpp, extract_csharp,
    extract_dart, extract_delphi_form, extract_elixir, extract_fortran, extract_go, extract_groovy,
    extract_java, extract_js, extract_json, extract_julia, extract_kotlin, extract_lazarus_form,
    extract_lazarus_package, extract_lua, extract_markdown, extract_objc, extract_pascal,
    extract_php, extract_powershell, extract_python, extract_ruby, extract_rust, extract_scala,
    extract_sql, extract_svelte, extract_swift, extract_verilog, extract_zig,
};
pub use ids::{file_stem, make_id, make_id1};
pub use types::{Edge, ExtractOutput, FileResult, Node, RawCall};
