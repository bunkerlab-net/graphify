//! Public extractor functions — one per language (or group of related languages).
//!
//! Each function mirrors a Python `extract_<lang>` function from `extract.py`.

pub mod apex;
pub mod bash;
pub mod blade;
pub mod dart;
pub mod dm;
pub mod dotnet;
pub mod elixir;
pub mod fortran;
pub mod go;
pub mod groovy;
pub mod json_lang;
pub mod julia;
pub mod manifest_ingest;
pub mod markdown;
pub mod mcp;
pub mod multi;
pub mod objc;
pub mod pascal;
pub mod powershell;
pub mod rust_lang;
pub mod sql;
pub mod svelte;
pub mod terraform;
pub mod verilog;
pub mod zig;

mod python_rationale;

use std::path::Path;

use crate::generic::extract_generic;
use crate::lang_configs;
use crate::types::FileResult;

pub use groovy::extract_groovy;
pub use multi::extract;

/// Size cap for project XML files (`.csproj` / `.fsproj` / `.vbproj` / `.lpk`).
/// Real files are well under 2 MiB; anything larger is malformed or hostile.
/// Mirrors `_PROJECT_XML_MAX_BYTES` in `graphify-py`.
pub(crate) const PROJECT_XML_MAX_BYTES: u64 = 2 * 1024 * 1024;

/// Reject project XML that declares a DTD or entities.
///
/// Defense in depth against billion-laughs style entity-expansion `DoS`.
/// Legitimate `MSBuild` and Lazarus package files never contain a `<!DOCTYPE`
/// or `<!ENTITY` declaration, so this is a zero-false-positive screen.
/// Mirrors `_project_xml_is_safe` in `graphify-py`.
#[must_use]
pub(crate) fn project_xml_is_safe(src: &[u8]) -> bool {
    // Scan the raw bytes with an ASCII case-insensitive window match rather
    // than allocating a lowercase copy of the whole (up to 2 MiB) file.
    fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|w| w.eq_ignore_ascii_case(needle))
    }
    !contains_ci(src, b"<!doctype") && !contains_ci(src, b"<!entity")
}

// ── Python ────────────────────────────────────────────────────────────────────

/// Extract classes, functions, and imports from a `.py` file.
#[must_use]
pub fn extract_python(path: &Path) -> FileResult {
    let mut result = extract_generic(path, &lang_configs::PYTHON);
    if result.error.is_none() {
        python_rationale::extract_python_rationale(path, &mut result);
    }
    result
}

// ── JavaScript / TypeScript ───────────────────────────────────────────────────

/// Extract classes, functions, arrow functions, and imports from `.js`/`.ts`/`.tsx` files.
#[must_use]
pub fn extract_js(path: &Path) -> FileResult {
    let config = match path.extension().and_then(|e| e.to_str()) {
        Some("tsx") => &*lang_configs::TYPESCRIPT_TSX,
        Some("ts") => &*lang_configs::TYPESCRIPT,
        _ => &*lang_configs::JAVASCRIPT,
    };
    extract_generic(path, config)
}

// ── Java ──────────────────────────────────────────────────────────────────────

/// Extract classes, interfaces, methods, constructors, and imports from a `.java` file.
#[must_use]
pub fn extract_java(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::JAVA)
}

// ── C ─────────────────────────────────────────────────────────────────────────

/// Extract functions and includes from a `.c`/`.h` file.
#[must_use]
pub fn extract_c(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::C)
}

// ── C++ ───────────────────────────────────────────────────────────────────────

/// Extract functions, classes, and includes from a `.cpp`/`.cc`/`.cxx`/`.hpp` file.
#[must_use]
pub fn extract_cpp(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::CPP)
}

// ── Ruby ──────────────────────────────────────────────────────────────────────

/// Extract classes, methods, singleton methods, and calls from a `.rb` file.
#[must_use]
pub fn extract_ruby(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::RUBY)
}

// ── C# ────────────────────────────────────────────────────────────────────────

/// Extract classes, interfaces, methods, namespaces, and usings from a `.cs` file.
#[must_use]
pub fn extract_csharp(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::CSHARP)
}

// ── Kotlin ────────────────────────────────────────────────────────────────────

/// Extract classes, objects, functions, and imports from a `.kt`/`.kts` file.
#[must_use]
pub fn extract_kotlin(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::KOTLIN)
}

// ── Scala ─────────────────────────────────────────────────────────────────────

/// Extract classes, objects, functions, and imports from a `.scala` file.
#[must_use]
pub fn extract_scala(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::SCALA)
}

// ── PHP ───────────────────────────────────────────────────────────────────────

/// Extract classes, functions, methods, namespace uses, and calls from a `.php` file.
#[must_use]
pub fn extract_php(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::PHP)
}

// ── Lua ───────────────────────────────────────────────────────────────────────

/// Extract functions, methods, and `require()` imports from a `.lua` file.
#[must_use]
pub fn extract_lua(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::LUA)
}

// ── Swift ─────────────────────────────────────────────────────────────────────

/// Extract classes, structs, protocols, functions, imports, and calls from a `.swift` file.
#[must_use]
pub fn extract_swift(path: &Path) -> FileResult {
    extract_generic(path, &lang_configs::SWIFT)
}

// ── Go ────────────────────────────────────────────────────────────────────────
pub use go::extract_go;

// ── Rust ──────────────────────────────────────────────────────────────────────
pub use rust_lang::extract_rust;

// ── Zig ───────────────────────────────────────────────────────────────────────
pub use zig::extract_zig;

// ── PowerShell ────────────────────────────────────────────────────────────────
pub use powershell::{extract_powershell, extract_powershell_manifest};

// ── Elixir ────────────────────────────────────────────────────────────────────
pub use elixir::extract_elixir;

// ── Julia ─────────────────────────────────────────────────────────────────────
pub use julia::extract_julia;

// ── Fortran ───────────────────────────────────────────────────────────────────
pub use fortran::{extract_fortran, resolve_cpp_path};

// ── ObjC ──────────────────────────────────────────────────────────────────────
pub use objc::extract_objc;

// ── Bash ──────────────────────────────────────────────────────────────────────
pub use bash::extract_bash;

// ── Apex ──────────────────────────────────────────────────────────────────────
pub use apex::extract_apex;

// ── Terraform / HCL ─────────────────────────────────────────────────────────────
pub use terraform::extract_terraform;

// ── JSON ──────────────────────────────────────────────────────────────────────
pub use json_lang::extract_json;

// ── Verilog ───────────────────────────────────────────────────────────────────
pub use verilog::extract_verilog;

// ── SQL ───────────────────────────────────────────────────────────────────────
pub use sql::{extract_sql, extract_sql_with_content};

// ── Markdown ──────────────────────────────────────────────────────────────────
pub use markdown::extract_markdown;

// ── Pascal ────────────────────────────────────────────────────────────────────
pub use pascal::{
    extract_delphi_form, extract_lazarus_form, extract_lazarus_package, extract_pascal,
};

// ── Svelte / Astro ────────────────────────────────────────────────────────────
pub use svelte::{extract_astro, extract_svelte};

// ── Dart ──────────────────────────────────────────────────────────────────────
pub use dart::extract_dart;

// ── BYOND DreamMaker (.dm / .dme / .dmi / .dmm / .dmf) ──────────────────────────
pub use dm::{extract_dm, extract_dmf, extract_dmi, extract_dmm};

// ── MCP config (.mcp.json / claude_desktop_config.json / ...) ─────────────────
pub use mcp::{extract_mcp_config, is_mcp_config_path};

// ── Package manifests (apm.yml / pyproject.toml / go.mod / pom.xml) ────────────
pub use manifest_ingest::extract_package_manifest;

// ── Blade ─────────────────────────────────────────────────────────────────────
pub use blade::extract_blade;

// ── .NET (.sln / .slnx / .csproj / .razor) ─────────────────────────────────────
pub use dotnet::{extract_csproj, extract_razor, extract_sln, extract_slnx};
