//! File-extension constants used by the watcher and the output-dir
//! environment-variable override.

/// Default output sub-directory name, overridden by the `GRAPHIFY_OUT`
/// environment variable.
pub(crate) const DEFAULT_GRAPHIFY_OUT: &str = "graphify-out";

/// All extensions the watcher pays attention to (code + doc + paper +
/// image).
///
/// Corresponds to `_WATCHED_EXTENSIONS` in the Python reference.
/// Elements are bare extensions **without** a leading dot, matching
/// `graphify_detect::extensions::CODE_EXTENSIONS` et al.
///
/// Built as the concatenation of the four upstream slices. In const
/// context we cannot call heap-allocating helpers, so we list the
/// combined set explicitly, keeping it in sync with the detect crate
/// slices.
pub const WATCHED_EXTENSIONS: &[&str] = &[
    // CODE_EXTENSIONS
    "py", "ts", "js", "jsx", "tsx", "mjs", "ejs", "go", "rs", "java", "groovy", "gradle", "cpp",
    "cc", "cxx", "c", "h", "hpp", "rb", "swift", "kt", "kts", "cs", "scala", "php", "lua", "luau",
    "toc", "zig", "ps1", "ex", "exs", "m", "mm", "jl", "vue", "svelte", "astro", "dart", "v", "sv",
    "sql", "r", "f", "F", "f90", "F90", "f95", "F95", "f03", "F03", "f08", "F08", "pas", "pp",
    "dpr", "dpk", "lpr", "inc", "dfm", "lfm", "lpk", "sh", "bash", "json",
    // DOC_EXTENSIONS
    "md", "mdx", "qmd", "txt", "rst", "html", "yaml", "yml", // PAPER_EXTENSIONS
    "pdf", // IMAGE_EXTENSIONS
    "png", "jpg", "jpeg", "gif", "webp", "svg",
];
