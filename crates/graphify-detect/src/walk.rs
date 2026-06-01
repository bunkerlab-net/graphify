//! Directory walking and file discovery.
//!
//! Ports `detect`, `collect_files`, and `_auto_follow_symlinks` from
//! `graphify-py/graphify/detect.py`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ignore_walk::{WalkBuilder, WalkState};
use indexmap::IndexMap;
use rayon::prelude::*;

use crate::extensions::{FileType, GOOGLE_WORKSPACE_EXTENSIONS, classify_file};
use crate::ignore::{
    IgnorePatterns, could_contain_included_path, is_ignored, load_graphifyignore,
    load_graphifyinclude,
};
use crate::office::{convert_office_file, xlsx_to_markdown};
use crate::sensitive::{SKIP_FILES, is_noise_dir, is_sensitive};

/// File-count threshold above which word counting is dispatched to Rayon.
const PARALLEL_COUNT_THRESHOLD: usize = 64;

/// Classification verdict for one file, produced by the parallel Phase 1a
/// scan in [`detect`]. The downstream serial merge dispatches each variant
/// without touching shared mutable state until that point.
enum FileDecision {
    /// File is filtered out (ignored, converted sidecar, unclassifiable).
    Skip,
    /// File is sensitive — record its display string in `skipped_sensitive`.
    Sensitive(String),
    /// File classifies directly as `ftype`; word count is deferred to Phase 2.
    Direct(FileType),
    /// Google Workspace shortcut (`.gdoc`/`.gsheet`/...) — needs conversion.
    GoogleWorkspace(String, FileType),
    /// Office document (`.docx`/`.xlsx`) — needs conversion.
    Office(FileType),
}

/// Pure per-file classification used by [`detect`]'s Phase 1a Rayon scan.
fn classify_one(
    p: &Path,
    root: &Path,
    memory_dir: &Path,
    converted_dir: &Path,
    ignore_patterns: &IgnorePatterns,
) -> FileDecision {
    let in_memory = memory_dir.exists() && p.starts_with(memory_dir);
    if !in_memory && p.starts_with(converted_dir) {
        return FileDecision::Skip;
    }
    // Memory-dir sidecars bypass ignore filtering: a user's `.gitignore`
    // pattern (e.g. `*.md`) must not erase the `graphify-out/memory` notes we
    // generate ourselves. Mirrors graphify-py `detect()` (#1047).
    if !in_memory && is_ignored(p, root, ignore_patterns) {
        return FileDecision::Skip;
    }
    if is_sensitive(p) {
        return FileDecision::Sensitive(p.to_string_lossy().into_owned());
    }
    let Some(ftype) = classify_file(p) else {
        return FileDecision::Skip;
    };
    let ext_lower = p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    if GOOGLE_WORKSPACE_EXTENSIONS.contains(&ext_lower.as_str()) {
        return FileDecision::GoogleWorkspace(ext_lower, ftype);
    }
    if ext_lower == "docx" || ext_lower == "xlsx" {
        return FileDecision::Office(ftype);
    }
    FileDecision::Direct(ftype)
}

/// Convertible Google Workspace extensions (matches
/// `graphify_google::GOOGLE_WORKSPACE_EXTENSIONS`, with leading dots stripped).
const GOOGLE_CONVERTIBLE_EXTS: &[&str] = &["gdoc", "gsheet", "gslides"];

/// Word count above which a knowledge graph is recommended over a flat context window.
pub const CORPUS_WARN_THRESHOLD: u64 = 50_000;
/// Word count above which semantic extraction is considered expensive and the user is advised to narrow the scan scope.
pub const CORPUS_UPPER_THRESHOLD: u64 = 500_000;
/// File count above which the corpus is considered large regardless of word count.
pub const FILE_COUNT_UPPER: usize = 500;

/// Full output of a [`detect`] run, analogous to the Python dict return.
#[derive(Debug, Clone)]
pub struct DetectResult {
    /// Files grouped by type string, in the fixed insertion order `"code"`,
    /// `"document"`, `"paper"`, `"image"`, `"video"`. `IndexMap` (not `HashMap`)
    /// keeps that order observable so the flattened extraction file list — and
    /// hence `graph.json` node order and the manifest — is deterministic and
    /// matches Python's insertion-ordered dict.
    pub files: IndexMap<String, Vec<String>>,
    /// Total number of discovered files across all types.
    pub total_files: usize,
    /// Estimated total word count across all non-video files.
    pub total_words: u64,
    /// `true` when `total_words` exceeds [`CORPUS_WARN_THRESHOLD`], indicating a graph is recommended.
    pub needs_graph: bool,
    /// Human-readable corpus-size advisory, or `None` when no caveat applies.
    pub warning: Option<String>,
    /// Display strings for files skipped due to sensitive-file or conversion-failure rules.
    pub skipped_sensitive: Vec<String>,
    /// Number of active ignore patterns loaded from `.graphifyignore` / `.gitignore` files.
    pub graphifyignore_patterns: usize,
    /// Canonicalized path of the scan root as a UTF-8 string.
    pub scan_root: String,
}

/// Auto-detect symlink following: `true` when `root` has any direct symlinked child.
#[must_use]
pub fn auto_follow_symlinks(root: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in rd.flatten() {
        if entry.path().is_symlink() {
            return true;
        }
    }
    false
}

/// Count words in a file (for non-video types).
fn count_words(path: &Path, ftype: FileType) -> u64 {
    if ftype == FileType::Video {
        return 0;
    }
    let Ok(text) = std::fs::read(path) else {
        return 0;
    };
    String::from_utf8_lossy(&text).split_whitespace().count() as u64
}

// ── Walk context (bundles parameters to stay under clippy's 7-arg limit) ─────

struct WalkCtx<'a> {
    root: &'a Path,
    follow_symlinks: bool,
    ignore_patterns: &'a IgnorePatterns,
    include_patterns: &'a IgnorePatterns,
}

/// Per-thread collector for the parallel walker. Each Rayon worker owns one
/// of these; on drop the local buffer is appended to the shared `Vec` under
/// a single mutex acquisition per thread instead of per file.
struct LocalBuffer {
    local: Vec<PathBuf>,
    shared: Arc<Mutex<Vec<PathBuf>>>,
}

impl Drop for LocalBuffer {
    fn drop(&mut self) {
        if self.local.is_empty() {
            return;
        }
        let mut shared = self
            .shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        shared.append(&mut self.local);
    }
}

/// Parallel directory walker using `ignore::WalkBuilder::build_parallel`.
///
/// Standard `ignore` filters (hidden, gitignore, etc.) are disabled — graphify
/// applies its own `ignore_patterns` via `filter_entry`. Returns files in
/// non-deterministic order; callers that depend on stable ordering must sort
/// the result. Empty `Vec` on filesystem errors during the walk.
fn walk_dir_parallel(ctx: &WalkCtx<'_>, dir: &Path) -> Vec<PathBuf> {
    let shared: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));

    let mut builder = WalkBuilder::new(dir);
    builder
        .standard_filters(false) // graphify applies its own ignore logic
        .follow_links(ctx.follow_symlinks)
        .threads(0); // 0 → ignore::walk picks rayon's default thread count

    let root = ctx.root.to_path_buf();
    let ignore_patterns = ctx.ignore_patterns.clone();
    let include_patterns = ctx.include_patterns.clone();
    builder.filter_entry(move |entry| {
        let file_type = entry.file_type();
        let is_dir = file_type.is_some_and(|ft| ft.is_dir());
        if !is_dir {
            // Per-file pruning is handled in `classify_one` later; let the
            // walker emit the file so we can decide there. Returning true
            // would still emit but we'd lose the noise-dir pruning of
            // sibling subdirs underneath this entry — which doesn't apply
            // to non-dirs anyway.
            return true;
        }
        let path = entry.path();
        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let parent_name = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str());
        if is_noise_dir(dir_name, parent_name) {
            return false;
        }
        let has_negation = ignore_patterns.iter().any(|(_, p)| p.starts_with('!'));
        if !has_negation
            && is_ignored(path, &root, &ignore_patterns)
            && !could_contain_included_path(path, &root, &include_patterns)
        {
            return false;
        }
        true
    });

    let walker = builder.build_parallel();
    walker.run(|| {
        let mut buf = LocalBuffer {
            local: Vec::new(),
            shared: Arc::clone(&shared),
        };
        Box::new(move |result| {
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            let Some(ft) = entry.file_type() else {
                return WalkState::Continue;
            };
            // The walker emits both files and symlinks; treat symlinks-to-files
            // the same way the sequential walker did.
            let path = entry.path();
            let is_regular_file = ft.is_file();
            let is_symlink_to_file = ft.is_symlink()
                && !ctx.follow_symlinks
                && std::fs::metadata(path).is_ok_and(|m| m.is_file());
            if !is_regular_file && !is_symlink_to_file {
                return WalkState::Continue;
            }
            let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if SKIP_FILES.contains(fname) {
                return WalkState::Continue;
            }
            buf.local.push(path.to_path_buf());
            WalkState::Continue
        })
    });

    // walker.run is synchronous; by the time it returns, every per-worker
    // Box (and its captured LocalBuffer) has been dropped, so no other Arc
    // clones remain. We take the inner Vec under the lock to avoid a clone.
    let mut guard = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    std::mem::take(&mut *guard)
}

/// Recursively collect all files under `dir`, respecting ignore/include patterns and noise-dir pruning.
///
/// `in_memory_tree` disables noise-dir and ignore filtering, used when scanning the `graphify-out/memory` sidecar directory.
fn walk_dir(
    ctx: &WalkCtx<'_>,
    dir: &Path,
    in_memory_tree: bool,
    seen: &mut HashSet<PathBuf>,
    all_files: &mut Vec<PathBuf>,
) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };

    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut file_paths: Vec<PathBuf> = Vec::new();
    let mut seen_in_dir: HashSet<PathBuf> = HashSet::new();

    for entry in rd.flatten() {
        let path = entry.path();
        let meta = if ctx.follow_symlinks {
            std::fs::metadata(&path)
        } else {
            std::fs::symlink_metadata(&path)
        };
        let Ok(m) = meta else {
            continue;
        };

        if m.is_dir() {
            if ctx.follow_symlinks && path.is_symlink() {
                // Circular symlink detection
                if let (Ok(real), Ok(real_dir)) =
                    (std::fs::canonicalize(&path), std::fs::canonicalize(dir))
                    && (real_dir == real || real_dir.starts_with(&real))
                {
                    continue;
                }
            }
            subdirs.push(path);
        } else if m.is_file() {
            seen_in_dir.insert(path.clone());
            file_paths.push(path);
        }
    }

    // When follow_symlinks=false, also pick up symlinks-to-files
    // (Python's os.walk followlinks=False still lists symlink files).
    if !ctx.follow_symlinks
        && let Ok(rd2) = std::fs::read_dir(dir)
    {
        for entry in rd2.flatten() {
            let path = entry.path();
            if path.is_symlink()
                && !seen_in_dir.contains(&path)
                && std::fs::metadata(&path).is_ok_and(|m| m.is_file())
            {
                file_paths.push(path);
            }
        }
    }

    // Collect files
    for p in file_paths {
        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if SKIP_FILES.contains(fname) {
            continue;
        }
        if !seen.contains(&p) {
            seen.insert(p.clone());
            all_files.push(p);
        }
    }

    // Recurse into subdirs with noise-dir pruning
    for subdir in subdirs {
        let dir_name = subdir.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let parent_name = subdir
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str());

        if !in_memory_tree {
            if is_noise_dir(dir_name, parent_name) {
                continue;
            }
            let has_negation = ctx.ignore_patterns.iter().any(|(_, p)| p.starts_with('!'));
            if !has_negation
                && is_ignored(&subdir, ctx.root, ctx.ignore_patterns)
                && !could_contain_included_path(&subdir, ctx.root, ctx.include_patterns)
            {
                continue;
            }
        }

        walk_dir(ctx, &subdir, in_memory_tree, seen, all_files);
    }
}

/// Format a number with thousands separators (matches Python `f"{n:,}"`).
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    let offset = s.len() % 3;
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (i % 3 == offset || (offset == 0 && i % 3 == 0)) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Build a corpus-size advisory message, or `None` when the graph is strongly recommended and no caveat is needed.
fn build_warning(total_words: u64, total_files: usize) -> Option<String> {
    let needs_graph = total_words >= CORPUS_WARN_THRESHOLD;
    if !needs_graph {
        Some(format!(
            "Corpus is ~{} words - fits in a single context window. You may not need a graph.",
            format_number(total_words)
        ))
    } else if total_words >= CORPUS_UPPER_THRESHOLD || total_files >= FILE_COUNT_UPPER {
        Some(format!(
            "Large corpus: {} files · ~{} words. Semantic extraction will be expensive (many Claude tokens). Consider running on a subfolder.",
            total_files,
            format_number(total_words)
        ))
    } else {
        None
    }
}

/// Discover all supported files under `root`, returning a rich result struct.
///
/// Mirrors Python's `detect()` function.
#[must_use]
pub fn detect(
    root: &Path,
    follow_symlinks: Option<bool>,
    extra_excludes: Option<&[String]>,
) -> DetectResult {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let follow_symlinks = follow_symlinks.unwrap_or_else(|| auto_follow_symlinks(&root));

    let mut ignore_patterns = load_graphifyignore(&root);
    if let Some(excludes) = extra_excludes {
        for pat in excludes {
            let line = crate::ignore::parse_gitignore_line(pat);
            if !line.is_empty() {
                ignore_patterns.push((root.clone(), line));
            }
        }
    }
    let include_patterns = load_graphifyinclude(&root);
    let graphifyignore_patterns = ignore_patterns.len();

    let memory_dir = root.join("graphify-out").join("memory");
    let converted_dir = root.join("graphify-out").join("converted");
    let google_workspace = graphify_google::google_workspace_enabled(None);

    let ctx = WalkCtx {
        root: &root,
        follow_symlinks,
        ignore_patterns: &ignore_patterns,
        include_patterns: &include_patterns,
    };
    let all_files = run_walk_phase(&ctx, &root, &memory_dir);

    let ClassifyOutput {
        files,
        to_count,
        skipped_sensitive,
    } = run_classify_phase(
        &all_files,
        &root,
        &memory_dir,
        &converted_dir,
        &ignore_patterns,
        google_workspace,
    );

    let total_words = run_word_count_phase(&to_count);
    let total_files: usize = files.values().map(Vec::len).sum();
    let needs_graph = total_words >= CORPUS_WARN_THRESHOLD;
    let warning = build_warning(total_words, total_files);

    DetectResult {
        files,
        total_files,
        total_words,
        needs_graph,
        warning,
        skipped_sensitive,
        graphifyignore_patterns,
        scan_root: root.to_string_lossy().into_owned(),
    }
}

/// Walk the project + sidecar memory tree, deduplicating against `seen`.
fn run_walk_phase(ctx: &WalkCtx<'_>, root: &Path, memory_dir: &Path) -> Vec<PathBuf> {
    let scan_paths: Vec<(PathBuf, bool)> = {
        let mut v = vec![(root.to_path_buf(), false)];
        if memory_dir.exists() {
            v.push((memory_dir.to_path_buf(), true));
        }
        v
    };
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut all_files: Vec<PathBuf> = Vec::new();
    let t_walk = std::time::Instant::now();
    for (scan_root, in_memory) in &scan_paths {
        if *in_memory {
            // The in-memory sidecar tree disables noise-dir and ignore filtering;
            // the existing sequential walker carries those rules.
            walk_dir(ctx, scan_root, *in_memory, &mut seen, &mut all_files);
        } else {
            // Main project tree: dispatch to the parallel walker.
            let found = walk_dir_parallel(ctx, scan_root);
            for p in found {
                if seen.insert(p.clone()) {
                    all_files.push(p);
                }
            }
        }
    }
    // Sort lexicographically by the path's string form so classification (and
    // therefore graph.json) is deterministic regardless of the parallel walker's
    // completion order (8db19d6). Sort by the full string, not by PathBuf
    // components, to match Python's `sorted(key=str)`.
    all_files.sort_by_cached_key(|p| p.to_string_lossy().into_owned());
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf]   walk_dir: {:.2}s ({} files)",
            t_walk.elapsed().as_secs_f64(),
            all_files.len()
        );
    }
    all_files
}

/// Output of [`run_classify_phase`].
struct ClassifyOutput {
    files: IndexMap<String, Vec<String>>,
    to_count: Vec<(PathBuf, FileType)>,
    skipped_sensitive: Vec<String>,
}

/// Classify each file, dispatch sidecar conversions, and return the per-kind file
/// map, word-count work list, and skipped-sensitive list.
fn run_classify_phase(
    all_files: &[PathBuf],
    root: &Path,
    memory_dir: &Path,
    converted_dir: &Path,
    ignore_patterns: &crate::ignore::IgnorePatterns,
    google_workspace: bool,
) -> ClassifyOutput {
    let mut files: IndexMap<String, Vec<String>> = ["code", "document", "paper", "image", "video"]
        .iter()
        .map(|k| ((*k).to_string(), Vec::new()))
        .collect();
    let mut to_count: Vec<(PathBuf, FileType)> = Vec::new();
    let mut skipped_sensitive: Vec<String> = Vec::new();

    let t_phase1 = std::time::Instant::now();
    // Phase 1a (parallel): classify each file independently.
    let decisions: Vec<FileDecision> = if all_files.len() >= PARALLEL_COUNT_THRESHOLD {
        all_files
            .par_iter()
            .map(|p| classify_one(p, root, memory_dir, converted_dir, ignore_patterns))
            .collect()
    } else {
        all_files
            .iter()
            .map(|p| classify_one(p, root, memory_dir, converted_dir, ignore_patterns))
            .collect()
    };

    // Phase 1b (sequential): apply decisions and run sidecar conversions.
    let mut convert_ctx = ConvertCtx {
        root,
        converted_dir,
        ignore_patterns,
        files: &mut files,
        to_count: &mut to_count,
        skipped_sensitive: &mut skipped_sensitive,
    };
    for (p, decision) in all_files.iter().zip(decisions) {
        apply_file_decision(&mut convert_ctx, p, decision, google_workspace);
    }

    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf]   walk Phase 1 (classify+convert): {:.2}s ({} to_count)",
            t_phase1.elapsed().as_secs_f64(),
            to_count.len()
        );
    }
    // Sort each file-type bucket so the emitted file lists are deterministic
    // regardless of walk/classify order (8db19d6).
    for bucket in files.values_mut() {
        bucket.sort();
    }

    ClassifyOutput {
        files,
        to_count,
        skipped_sensitive,
    }
}

/// Apply one classification decision to the shared accumulators.
fn apply_file_decision(
    ctx: &mut ConvertCtx<'_>,
    p: &Path,
    decision: FileDecision,
    google_workspace: bool,
) {
    match decision {
        FileDecision::Skip => {}
        FileDecision::Sensitive(rendered) => ctx.skipped_sensitive.push(rendered),
        FileDecision::Direct(ftype) => {
            ctx.files
                .entry(ftype.as_str().to_string())
                .or_default()
                .push(p.to_string_lossy().into_owned());
            ctx.to_count.push((p.to_path_buf(), ftype));
        }
        FileDecision::GoogleWorkspace(ext_lower, ftype) => {
            convert_google_workspace(ctx, p, &ext_lower, ftype, google_workspace);
        }
        FileDecision::Office(ftype) => {
            convert_office(ctx, p, ftype);
        }
    }
}

/// Sum word counts across `to_count`, dispatching to Rayon for large lists.
fn run_word_count_phase(to_count: &[(PathBuf, FileType)]) -> u64 {
    let t_phase2 = std::time::Instant::now();
    let total = if to_count.len() >= PARALLEL_COUNT_THRESHOLD {
        to_count
            .par_iter()
            .map(|(p, ftype)| count_words(p, *ftype))
            .sum::<u64>()
    } else {
        to_count
            .iter()
            .map(|(p, ftype)| count_words(p, *ftype))
            .sum::<u64>()
    };
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf]   walk Phase 2 (word counts): {:.2}s",
            t_phase2.elapsed().as_secs_f64()
        );
    }
    total
}

/// Collect all supported files under `root`, returning a flat `Vec<PathBuf>`.
///
/// Simplified wrapper over [`detect`] that concatenates all file lists.
#[must_use]
pub fn collect_files(root: &Path) -> Vec<PathBuf> {
    let result = detect(root, None, None);
    result
        .files
        .into_values()
        .flatten()
        .map(PathBuf::from)
        .collect()
}

/// Mutable bundle threaded through the per-file conversion helpers.
struct ConvertCtx<'a> {
    root: &'a Path,
    converted_dir: &'a Path,
    ignore_patterns: &'a IgnorePatterns,
    files: &'a mut IndexMap<String, Vec<String>>,
    to_count: &'a mut Vec<(PathBuf, FileType)>,
    skipped_sensitive: &'a mut Vec<String>,
}

impl ConvertCtx<'_> {
    /// Register a converted sidecar file. Word counting is deferred to a
    /// parallel pass after all conversions have completed.
    fn record(&mut self, md_path: &Path, ftype: FileType) {
        if is_ignored(md_path, self.root, self.ignore_patterns) {
            return;
        }
        self.files
            .entry(ftype.as_str().to_string())
            .or_default()
            .push(md_path.to_string_lossy().into_owned());
        self.to_count.push((md_path.to_path_buf(), ftype));
    }
}

/// Convert a `.gdoc`/`.gsheet`/`.gslides` shortcut to a markdown sidecar.
///
/// Other Google Workspace types (`.gdraw`, `.gform`, etc.) have no Markdown
/// export path and are recorded in `skipped_sensitive`.
fn convert_google_workspace(
    ctx: &mut ConvertCtx<'_>,
    p: &Path,
    ext_lower: &str,
    ftype: FileType,
    google_workspace: bool,
) {
    if !google_workspace {
        ctx.skipped_sensitive.push(format!(
            "{} [Google Workspace shortcut skipped - pass --google-workspace or set GRAPHIFY_GOOGLE_WORKSPACE=1]",
            p.to_string_lossy()
        ));
        return;
    }
    if !GOOGLE_CONVERTIBLE_EXTS.contains(&ext_lower) {
        ctx.skipped_sensitive.push(format!(
            "{} [Google Workspace shortcut type .{ext_lower} not exportable to Markdown]",
            p.to_string_lossy(),
        ));
        return;
    }
    let convert_res = graphify_google::convert_google_workspace_file::<
        fn(&str, &str, &Path, Option<&str>) -> Result<(), graphify_google::GoogleError>,
        _,
        std::io::Error,
    >(
        p,
        ctx.converted_dir,
        Some(|tmp_path: &Path| -> Result<String, std::io::Error> {
            Ok(xlsx_to_markdown(tmp_path))
        }),
        None,
    );
    match convert_res {
        Ok(Some(md_path)) => ctx.record(&md_path, ftype),
        Ok(None) => ctx.skipped_sensitive.push(format!(
            "{} [Google Workspace export produced no readable text]",
            p.to_string_lossy()
        )),
        Err(e) => ctx.skipped_sensitive.push(format!(
            "{} [Google Workspace export failed: {e}]",
            p.to_string_lossy()
        )),
    }
}

/// Convert a `.docx`/`.xlsx` to a markdown sidecar via `office::convert_office_file`.
fn convert_office(ctx: &mut ConvertCtx<'_>, p: &Path, ftype: FileType) {
    match convert_office_file(p, ctx.converted_dir) {
        Ok(Some(md_path)) => ctx.record(&md_path, ftype),
        Ok(None) => ctx.skipped_sensitive.push(format!(
            "{} [office document contained no extractable text]",
            p.to_string_lossy()
        )),
        Err(e) => ctx.skipped_sensitive.push(format!(
            "{} [office conversion failed: {e}]",
            p.to_string_lossy()
        )),
    }
}
