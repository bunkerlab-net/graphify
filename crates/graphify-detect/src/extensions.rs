//! File-type classification: extension sets and the `classify_file` function.
//!
//! Ports the extension sets and `classify_file` from `graphify-py/graphify/detect.py`.

use std::path::Path;

use regex::Regex;

use crate::shebang::shebang_interpreter;

/// File extension → type mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    /// Source code file (e.g. `.rs`, `.py`, `.ts`).
    Code,
    /// Prose document (e.g. `.md`, `.html`, `.docx`).
    Document,
    /// Academic paper or PDF with paper-like content.
    Paper,
    /// Raster or vector image (e.g. `.png`, `.svg`).
    Image,
    /// Audio or video media file (e.g. `.mp4`, `.mp3`).
    Video,
}

impl FileType {
    /// Return the string key used in JSON output (matches Python `FileType.value`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Document => "document",
            Self::Paper => "paper",
            Self::Image => "image",
            Self::Video => "video",
        }
    }
}

/// Code source file extensions (without the leading dot).
///
/// This is the authoritative list — matches Python `CODE_EXTENSIONS`. The
/// `.tsx` entry precedes `.js` to mirror the Python ordering, and `.ets`
/// (`ArkTS` / `HarmonyOS`) was added immediately after `.ejs` so the new
/// entry sits at the tail of the TypeScript-family run rather than in the
/// middle of an unrelated group.
pub const CODE_EXTENSIONS: &[&str] = &[
    "py", "ts", "tsx", "js", "jsx", "mjs", "ejs", "ets", "go", "rs", "java", "groovy", "gradle",
    "cpp", "cc", "cxx", "c", "h", "hpp", "rb", "swift", "kt", "kts", "cs", "scala", "php", "lua",
    "luau", "toc", "zig", "ps1", "psm1", "psd1", "ex", "exs", "m", "mm", "jl", "vue", "svelte",
    "astro", "dart", "v", "sv", "svh", "sql", "r", "f", "F", "f90", "F90", "f95", "F95", "f03",
    "F03", "f08", "F08", "pas", "pp", "dpr", "dpk", "lpr", "inc", "dfm", "lfm", "lpk", "sh",
    "bash", "json", "tf", "tfvars", "hcl", "dm", "dme", "dmi", "dmm", "dmf", "sln", "slnx",
    "csproj", "fsproj", "vbproj", "razor", "cshtml", "cls", "trigger",
];

/// Package-manifest filename (lowercased) → ecosystem tag.
///
/// Mirrors Python `graphify.manifest_ingest.PACKAGE_MANIFEST_NAMES`. The
/// ecosystem tag selects the manifest parser downstream in `graphify-extract`.
/// Kept as an ordered slice so the few entries stay allocation-free yet iterate
/// in a stable order matching the Python dict's insertion order.
pub const PACKAGE_MANIFEST_NAMES: &[(&str, &str)] = &[
    ("apm.yml", "apm"),
    ("apm.yaml", "apm"),
    ("pyproject.toml", "python"),
    ("go.mod", "go"),
    ("pom.xml", "maven"),
];

const DOC_EXTENSIONS: &[&str] = &["md", "mdx", "qmd", "txt", "rst", "html", "yaml", "yml"];

const PAPER_EXTENSIONS: &[&str] = &["pdf"];

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "svg"];

const OFFICE_EXTENSIONS: &[&str] = &["docx", "xlsx"];

/// Google Workspace shortcut extensions (mirrors Python `GOOGLE_WORKSPACE_EXTENSIONS`).
pub const GOOGLE_WORKSPACE_EXTENSIONS: &[&str] = &[
    "gdoc", "gsheet", "gslides", "gdraw", "gform", "gmap", "gsite",
];

const VIDEO_EXTENSIONS: &[&str] = &[
    "mp4", "mov", "webm", "mkv", "avi", "m4v", "mp3", "wav", "m4a", "ogg",
];

/// Xcode asset-catalog directory markers.
const ASSET_DIR_MARKERS: &[&str] = &[
    ".imageset",
    ".xcassets",
    ".appiconset",
    ".colorset",
    ".launchimage",
];

/// Interpreter names that indicate a shebang file is code.
const SHEBANG_CODE_INTERPRETERS: &[&str] = &[
    "python", "python3", "python2", "ruby", "perl", "node", "nodejs", "bash", "sh", "dash", "zsh",
    "fish", "ksh", "tcsh", "lua", "php", "julia", "Rscript",
];

// ── Paper signal regexes (mirrors Python `_PAPER_SIGNALS`) ──────────────────

static PAPER_SIGNALS: std::sync::LazyLock<Vec<Regex>> = std::sync::LazyLock::new(|| {
    let patterns = [
        r"(?i)\barxiv\b",
        r"(?i)\bdoi\s*:",
        r"(?i)\babstract\b",
        r"(?i)\bproceedings\b",
        r"(?i)\bjournal\b",
        r"(?i)\bpreprint\b",
        r"\\cite\{",
        r"\[\d+\]",
        r"\[\n\d+\n\]",
        r"(?i)eq\.\s*\d+|equation\s+\d+",
        r"\d{4}\.\d{4,5}",
        r"(?i)\bwe propose\b",
        r"(?i)\bliterature\b",
    ];
    #[allow(clippy::expect_used)]
    let compiled = patterns
        .iter()
        .map(|p| Regex::new(p).expect("literal patterns are valid"))
        .collect();
    compiled
});

const PAPER_SIGNAL_THRESHOLD: usize = 3;

/// Heuristic: does this text file read like an academic paper?
///
/// Reads only the first 3000 bytes (matching Python's `[:3000]`) so big
/// papers don't get fully loaded into memory.
fn looks_like_paper(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; 3000];
    let Ok(n) = std::io::Read::read(&mut file, &mut buf) else {
        return false;
    };
    let text = String::from_utf8_lossy(&buf[..n]);
    let hits = PAPER_SIGNALS.iter().filter(|r| r.is_match(&text)).count();
    hits >= PAPER_SIGNAL_THRESHOLD
}

/// Peek at the first line of an extensionless file for a shebang interpreter.
fn shebang_file_type(path: &Path) -> Option<FileType> {
    let interp = shebang_interpreter(path)?;
    if SHEBANG_CODE_INTERPRETERS.contains(&interp.as_str()) {
        Some(FileType::Code)
    } else {
        None
    }
}

/// Returns `true` if `path`'s filename (lowercased) is a recognized package
/// manifest. Mirrors Python `manifest_ingest.is_package_manifest_path`.
#[must_use]
pub fn is_package_manifest_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    // Allocate the lowercase form only when the filename actually has uppercase
    // ASCII; manifest filenames are checked on every file, so the common
    // already-lowercase case stays allocation-free.
    let name_owned;
    let name_lc: &str = if name.bytes().any(|b| b.is_ascii_uppercase()) {
        name_owned = name.to_ascii_lowercase();
        &name_owned
    } else {
        name
    };
    PACKAGE_MANIFEST_NAMES.iter().any(|(n, _)| *n == name_lc)
}

/// Classify a file by its extension (and content for ambiguous cases).
///
/// Returns `None` for unknown/unsupported types.
#[must_use]
pub fn classify_file(path: &Path) -> Option<FileType> {
    // Package manifests (apm.yml, pyproject.toml, go.mod, pom.xml) are parsed
    // deterministically, so route them to the AST path (Code) rather than the
    // LLM document path — otherwise apm.yml (a .yml "document") would be
    // LLM-extracted and a package would split into duplicate file-anchored
    // nodes (#1377).
    if is_package_manifest_path(path) {
        return Some(FileType::Code);
    }
    // Compound extension check first (.blade.php). The case-insensitive
    // `.to_lowercase()` on the full path was the most expensive allocation
    // in this function — replace with an `eq_ignore_ascii_case` suffix
    // check that needs no allocation at all.
    if let Some(s) = path.to_str()
        && s.len() >= 10
        && s.as_bytes()[s.len() - 10..].eq_ignore_ascii_case(b".blade.php")
    {
        return Some(FileType::Code);
    }

    let ext_raw = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if ext_raw.is_empty() {
        return shebang_file_type(path);
    }
    // Only allocate the lowercase string if the raw extension contains any
    // uppercase ASCII letters. The hot path on macOS/Linux for typical
    // file extensions (`.rs`, `.js`, `.py`) is already lowercase, so this
    // skips the allocation entirely for the common case.
    let ext_owned;
    let ext: &str = if ext_raw.bytes().any(|b| b.is_ascii_uppercase()) {
        ext_owned = ext_raw.to_ascii_lowercase();
        &ext_owned
    } else {
        ext_raw
    };

    // Check case-sensitive CODE_EXTENSIONS first (some have upper-case variants like .F90)
    if CODE_EXTENSIONS.contains(&ext_raw) || CODE_EXTENSIONS.contains(&ext) {
        return Some(FileType::Code);
    }

    if PAPER_EXTENSIONS.contains(&ext) {
        // PDFs inside Xcode asset catalogs are vector icons, not papers.
        let in_asset_catalog = path.components().any(|c| {
            c.as_os_str()
                .to_str()
                .is_some_and(|s| ASSET_DIR_MARKERS.iter().any(|m| s.ends_with(m)))
        });
        if in_asset_catalog {
            return None;
        }
        return Some(FileType::Paper);
    }

    if IMAGE_EXTENSIONS.contains(&ext) {
        return Some(FileType::Image);
    }

    if DOC_EXTENSIONS.contains(&ext) {
        if looks_like_paper(path) {
            return Some(FileType::Paper);
        }
        return Some(FileType::Document);
    }

    if OFFICE_EXTENSIONS.contains(&ext) {
        return Some(FileType::Document);
    }

    if GOOGLE_WORKSPACE_EXTENSIONS.contains(&ext) {
        return Some(FileType::Document);
    }

    if VIDEO_EXTENSIONS.contains(&ext) {
        return Some(FileType::Video);
    }

    None
}

#[cfg(test)]
#[path = "extensions_tests.rs"]
mod extensions_tests;
