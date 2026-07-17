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
    IgnorePatterns, could_contain_included_path, is_ignored, is_included, load_dir_own_ignore,
    load_graphifyignore, load_graphifyinclude,
};
use crate::office::{convert_office_file, xlsx_to_markdown};
use crate::sensitive::{SKIP_FILES, is_noise_dir, is_sensitive};

/// File-count threshold above which word counting is dispatched to Rayon.
const PARALLEL_COUNT_THRESHOLD: usize = 64;

/// Classification verdict for one file, produced by the parallel Phase 1a
/// scan in [`detect`]. The downstream serial merge dispatches each variant
/// without touching shared mutable state until that point.
enum FileDecision {
    /// File is filtered out with no diagnostic (converted sidecar, `converted/`
    /// subtree). Ignore-rule drops use [`FileDecision::Ignored`] instead.
    Skip,
    /// File dropped by a `.gitignore`/`.graphifyignore`/`--exclude` rule —
    /// recorded in `ignored` so an over-broad ignore is visible instead of
    /// silently vanishing (#1922).
    Ignored(String),
    /// File was considered but has no supported extension / shebang — surfaced in
    /// `unclassified` so it is visible rather than silently dropped (#1692).
    Unclassified(String),
    /// File is sensitive — record its display string in `skipped_sensitive`.
    Sensitive(String),
    /// File classifies directly as `ftype`; word count is deferred to Phase 2.
    Direct(FileType),
    /// Google Workspace shortcut (`.gdoc`/`.gsheet`/...) — needs conversion.
    GoogleWorkspace(FileType),
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
    include_patterns: &IgnorePatterns,
) -> FileDecision {
    let in_memory = memory_dir.exists() && p.starts_with(memory_dir);
    if !in_memory && p.starts_with(converted_dir) {
        return FileDecision::Skip;
    }
    // Memory-dir sidecars bypass ignore filtering: a user's `.gitignore`
    // pattern (e.g. `*.md`) must not erase the `graphify-out/memory` notes we
    // generate ourselves. Mirrors graphify-py `detect()` (#1047).
    //
    // A `.graphifyinclude` allowlist re-includes an otherwise-ignored file
    // (gitignore-negation style, but in a dedicated file): the walker already
    // descends into ignored directories that `could_contain_included_path`
    // flags, and this is the matching file-level rescue. The sensitive-file
    // guard below still runs, so an allowlist cannot pull secrets into the
    // corpus. (graphify-py defines these helpers but never wired them in — the
    // feature was left inert there; the Rust port completes it.)
    if !in_memory && is_ignored(p, root, ignore_patterns) && !is_included(p, root, include_patterns)
    {
        return FileDecision::Ignored(p.to_string_lossy().into_owned());
    }
    if is_sensitive(p) {
        return FileDecision::Sensitive(p.to_string_lossy().into_owned());
    }
    let Some(ftype) = classify_file(p) else {
        return FileDecision::Unclassified(p.to_string_lossy().into_owned());
    };
    let ext_lower = p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    if GOOGLE_WORKSPACE_EXTENSIONS.contains(&ext_lower.as_str()) {
        return FileDecision::GoogleWorkspace(ftype);
    }
    if ext_lower == "docx" || ext_lower == "xlsx" {
        return FileDecision::Office(ftype);
    }
    FileDecision::Direct(ftype)
}

/// Word count above which a knowledge graph is recommended over a flat context window.
pub const CORPUS_WARN_THRESHOLD: u64 = 50_000;
/// Word count above which semantic extraction is considered expensive and the user is advised to narrow the scan scope.
pub const CORPUS_UPPER_THRESHOLD: u64 = 500_000;
/// File count above which the corpus is considered large regardless of word count.
pub const FILE_COUNT_UPPER: usize = 500;

/// Canonical file-type bucket keys, in the fixed order every [`DetectResult`]
/// must present them (see [`DetectResult::files`]). Used both by the fresh
/// `detect` walk and by callers that reconstruct a `DetectResult` (e.g. the
/// incremental path) so the two produce structurally identical results.
pub const FILE_TYPE_KINDS: [&str; 5] = ["code", "document", "paper", "image", "video"];

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
    /// Files that were considered but not classified — an extension in no
    /// supported set, or an extensionless non-shebang project file (Dockerfile,
    /// Makefile, LICENSE, ...). Surfaced (sorted) so they are visible rather than
    /// silently dropped (#1692); the CLI prints a one-line notice.
    pub unclassified: Vec<String>,
    /// Directories skipped during enumeration because their scan failed (e.g. a
    /// transient `PermissionError`, or a directory created/deleted mid-walk by
    /// concurrent writes). Surfaced so an incomplete file list is visible rather
    /// than silently producing a partial `graph.json`. Each entry is
    /// `"<path>: <error>"`; a matching warning is printed to stderr as it happens.
    pub walk_errors: Vec<String>,
    /// Files and directories dropped by a `.gitignore`/`.graphifyignore`/
    /// `--exclude` rule (#1922). Directory entries carry a trailing separator and
    /// keep the list bounded — a pruned `data/` is one entry, not one per
    /// contained file. Recorded so an over-broad ignore is visible instead of
    /// silently vanishing from the corpus. Sorted.
    pub ignored: Vec<String>,
    /// Number of active ignore patterns loaded from `.graphifyignore` / `.gitignore` files.
    pub graphifyignore_patterns: usize,
    /// Canonicalized path of the scan root as a UTF-8 string.
    pub scan_root: String,
}

/// `true` when `root` has any direct symlinked child.
///
/// Kept for callers that use it, but detection no longer enables symlink
/// following automatically (009a98b): following is now an explicit opt-in and
/// out-of-root symlink targets are never indexed.
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

/// `true` when `path` resolves to a target inside `root` (009a98b): a symlink
/// whose target escapes the scan root must never be indexed. Mirrors Python
/// `_resolves_under_root`.
#[must_use]
pub fn resolves_under_root(path: &Path, root: &Path) -> bool {
    let (Ok(rp), Ok(rr)) = (path.canonicalize(), root.canonicalize()) else {
        return false;
    };
    rp.starts_with(&rr)
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
// Cohesive parallel-walk driver: builder config, `filter_entry` pruning, the
// per-worker collect callback, and result/error/symlink extraction are one flow;
// splitting fragments the shared-state threading.
#[allow(clippy::too_many_lines)]
fn walk_dir_parallel(
    ctx: &WalkCtx<'_>,
    dir: &Path,
) -> (Vec<PathBuf>, Vec<String>, Vec<String>, Vec<String>) {
    let shared: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(Vec::new()));
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sym_skipped: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let ignored: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let mut builder = WalkBuilder::new(dir);
    builder
        .standard_filters(false) // graphify applies its own ignore logic
        .follow_links(ctx.follow_symlinks)
        .threads(0); // 0 → ignore::walk picks rayon's default thread count

    let root = ctx.root.to_path_buf();
    let ignore_patterns = ctx.ignore_patterns.clone();
    let include_patterns = ctx.include_patterns.clone();
    let sym_c = Arc::clone(&sym_skipped);
    let ignored_c = Arc::clone(&ignored);
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
        // (parent name is derived inside `is_noise_dir` from the dir path)
        // Containment: never descend a symlinked directory whose target escapes
        // the scan root (009a98b). Harmless when not following (symlinks aren't
        // traversed anyway); load-bearing when `follow_symlinks` is opted in.
        if path.is_symlink() && !resolves_under_root(path, &root) {
            sym_c
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(format!(
                    "{} [symlink target outside scan root]",
                    path.display()
                ));
            return false;
        }
        if is_noise_dir(dir_name, Some(path)) {
            return false;
        }
        // Negations need no special-casing: `is_ignored` applies last-match-wins
        // (so `!dir/` un-ignores a directory and it won't be pruned) and the
        // gitignore parent-exclusion rule (a `!` cannot rescue a file beneath an
        // excluded dir), so descending an ignored directory to look for a
        // re-included file is never necessary (#1276). The previous blanket
        // `has_negation` check disabled pruning for EVERY ignored dir whenever any
        // `!` rule existed — a pathological slowdown on large repos for no gain.
        if is_ignored(path, &root, &ignore_patterns)
            && !could_contain_included_path(path, &root, &include_patterns)
        {
            // Record the pruned subtree (dir + trailing separator) so an
            // over-broad ignore is visible; one entry covers the whole subtree.
            // Filesystem roots (`/`, `C:\`) already end in a separator — don't
            // double it.
            let shown = path.display().to_string();
            let sep = std::path::MAIN_SEPARATOR;
            let subtree = if shown.ends_with(sep) {
                shown
            } else {
                format!("{shown}{sep}")
            };
            ignored_c
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(subtree);
            return false;
        }
        true
    });

    let walker = builder.build_parallel();
    walker.run(|| {
        let errors_local = Arc::clone(&errors);
        let mut buf = LocalBuffer {
            local: Vec::new(),
            shared: Arc::clone(&shared),
        };
        Box::new(move |result| {
            let entry = match result {
                Ok(entry) => entry,
                Err(e) => {
                    // A scan failure (permission denied, or a dir removed mid-walk)
                    // is surfaced instead of being silently swallowed (#partial-graph).
                    let target = ignore_error_path(&e)
                        .map_or_else(|| "<unknown>".to_string(), |p| p.display().to_string());
                    let err_str = e
                        .io_error()
                        .map_or_else(|| e.to_string(), std::string::ToString::to_string);
                    let mut g = errors_local
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    record_walk_error(&target, &err_str, &mut g);
                    return WalkState::Continue;
                }
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
    let files = {
        let mut guard = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *guard)
    };
    let errs = {
        let mut guard = errors
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *guard)
    };
    let sym = {
        let mut guard = sym_skipped
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *guard)
    };
    let ignored = {
        let mut guard = ignored
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::take(&mut *guard)
    };
    (files, errs, sym, ignored)
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
    walk_errors: &mut Vec<String>,
    sym_skipped: &mut Vec<String>,
) {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            record_walk_error(&dir.display().to_string(), &e.to_string(), walk_errors);
            return;
        }
    };

    let mut subdirs: Vec<PathBuf> = Vec::new();
    let mut file_paths: Vec<PathBuf> = Vec::new();
    let mut seen_in_dir: HashSet<PathBuf> = HashSet::new();

    for entry in rd {
        // Surface — not swallow — a per-entry read failure or a stat failure, so a
        // file we cannot enumerate/classify is reported rather than silently
        // vanishing (the walk-errors contract). Python's `os.walk` classifies via
        // scandir's d_type and never stats per entry, so this is Rust-specific.
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                record_walk_error(&dir.display().to_string(), &e.to_string(), walk_errors);
                continue;
            }
        };
        let path = entry.path();
        let meta = if ctx.follow_symlinks {
            std::fs::metadata(&path)
        } else {
            std::fs::symlink_metadata(&path)
        };
        let m = match meta {
            Ok(m) => m,
            Err(e) => {
                record_walk_error(&path.display().to_string(), &e.to_string(), walk_errors);
                continue;
            }
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
            if ctx.follow_symlinks && path.is_symlink() && !resolves_under_root(&path, ctx.root) {
                sym_skipped.push(format!(
                    "{} [symlink target outside scan root]",
                    path.display()
                ));
                continue; // out-of-root symlink target — don't index (009a98b)
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
        // (parent name is derived inside `is_noise_dir` from the dir path)

        if !in_memory_tree {
            if is_noise_dir(dir_name, Some(subdir.as_path())) {
                continue;
            }
            // See `walk_dir_parallel`: negations need no special-casing here, so
            // ignored directories are always pruned (#1276).
            if is_ignored(&subdir, ctx.root, ctx.ignore_patterns)
                && !could_contain_included_path(&subdir, ctx.root, ctx.include_patterns)
            {
                continue;
            }
        }

        walk_dir(
            ctx,
            &subdir,
            in_memory_tree,
            seen,
            all_files,
            walk_errors,
            sym_skipped,
        );
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
    detect_with_cache_root(root, follow_symlinks, extra_excludes, None)
}

/// [`detect`] with an explicit cache root for the word-count/stat-index cache.
///
/// `cache_root` (e.g. from `extract --out <dir>`) relocates the stat-index
/// cache FILE out of the scanned corpus — entry keys are absolute paths, so the
/// relocation is safe. `None` roots the cache at `root` (#1747).
#[must_use]
pub fn detect_with_cache_root(
    root: &Path,
    follow_symlinks: Option<bool>,
    extra_excludes: Option<&[String]>,
    cache_root: Option<&Path>,
) -> DetectResult {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    // Following symlinks is now an explicit opt-in (009a98b): detection no longer
    // auto-enables it from a symlinked child, and out-of-root targets are skipped.
    let follow_symlinks = follow_symlinks.unwrap_or(false);

    let mut ignore_patterns = load_graphifyignore(&root);
    if let Some(excludes) = extra_excludes {
        for pat in excludes {
            let line = crate::ignore::parse_gitignore_line(pat);
            if !line.is_empty() {
                ignore_patterns.push((root.clone(), line));
            }
        }
    }
    // Nested .gitignore/.graphifyignore files BELOW the scan root are honored
    // too (#1206). The parallel walker is handed a frozen pattern list, so we
    // pre-collect descendant ignore files here (Python loads them live during
    // its os.walk); anchor-scoping makes the two equivalent.
    collect_nested_ignore(&root, follow_symlinks, &mut ignore_patterns);
    let include_patterns = load_graphifyinclude(&root);
    let graphifyignore_patterns = ignore_patterns.len();

    let out_dir = root.join(graphify_security::graphify_out());
    let memory_dir = out_dir.join("memory");
    let converted_dir = out_dir.join("converted");
    let google_workspace = graphify_google::google_workspace_enabled(None);

    let ctx = WalkCtx {
        root: &root,
        follow_symlinks,
        ignore_patterns: &ignore_patterns,
        include_patterns: &include_patterns,
    };
    let (all_files, walk_errors, sym_skipped, mut ignored) =
        run_walk_phase(&ctx, &root, &memory_dir);

    let ClassifyOutput {
        files,
        to_count,
        mut skipped_sensitive,
        unclassified,
        ignored: ignored_files,
    } = run_classify_phase(
        &all_files,
        &root,
        &memory_dir,
        &converted_dir,
        &ignore_patterns,
        &include_patterns,
        google_workspace,
    );
    // Symlink targets that escaped the scan root (dirs pruned mid-walk + files
    // filtered post-walk) are surfaced alongside sensitive-file skips (009a98b).
    skipped_sensitive.extend(sym_skipped);
    // Merge dir-level (walk) and file-level (classify) ignores; sort once (#1922).
    ignored.extend(ignored_files);
    ignored.sort();

    let total_words = run_word_count_phase(&to_count, &root, cache_root);
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
        unclassified,
        walk_errors,
        ignored,
        graphifyignore_patterns,
        scan_root: root.to_string_lossy().into_owned(),
    }
}

/// Pre-collect nested `.gitignore`/`.graphifyignore` patterns from directories
/// BELOW the scan root, appending them to `patterns` (#1206).
///
/// The parallel walker is handed a frozen pattern list, so — unlike Python's
/// os.walk, which loads each descendant's ignore file live before pruning that
/// directory's children — we traverse descendants top-down here first. Outer
/// dirs are processed before inner ones, so a parent's patterns precede a
/// child's; the anchor-scoping in `eval_path` (#1873) then makes each pattern
/// govern only its own subtree, exactly as live loading would.
///
/// Only noise directories and (unless `follow_symlinks`) symlinked directories
/// are skipped — the same coarse pruning the walk applies before any ignore
/// evaluation. We deliberately do NOT prune by the ignore set being built here:
/// the real walker retains some ignored directories via
/// `could_contain_included_path` (and `.graphifyinclude` is not even loaded
/// yet), so pruning here could drop a nested ignore file the walk still needs.
/// Anchor-scoping makes collecting patterns from otherwise-excluded subtrees
/// harmless — they can only ever govern their own (already-excluded) subtree.
fn collect_nested_ignore(root: &Path, follow_symlinks: bool, patterns: &mut IgnorePatterns) {
    let mut visited: HashSet<PathBuf> = HashSet::new();
    // Seed with the canonical root so a descendant symlink pointing back at the
    // root is treated as already-visited and never re-queues the whole corpus.
    visited.insert(root.canonicalize().unwrap_or_else(|_| root.to_path_buf()));
    let mut queue: std::collections::VecDeque<PathBuf> = std::collections::VecDeque::new();
    queue.push_back(root.to_path_buf());
    while let Some(dir) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        // Deterministic order so the appended-pattern sequence is stable.
        let mut child_dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        child_dirs.sort();
        for child in child_dirs {
            let name = child.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if is_noise_dir(name, Some(&child)) {
                continue;
            }
            // os.walk (followlinks=False) never descends symlinked dirs; when
            // following, an out-of-root target is skipped for containment.
            if child.is_symlink() && (!follow_symlinks || !resolves_under_root(&child, root)) {
                continue;
            }
            // Guard against symlink loops when following is enabled.
            let key = child.canonicalize().unwrap_or_else(|_| child.clone());
            if !visited.insert(key) {
                continue;
            }
            // Every surviving descendant (child != root) contributes its own
            // ignore file, anchored at itself.
            patterns.extend(load_dir_own_ignore(&child));
            queue.push_back(child);
        }
    }
}

/// Record a directory-scan failure: append `"<target>: <err>"` to `walk_errors`
/// and warn to stderr, so an incomplete enumeration is visible rather than a
/// silently partial `graph.json`. Mirrors Python `_on_walk_error`.
fn record_walk_error(target: &str, err: &str, walk_errors: &mut Vec<String>) {
    walk_errors.push(format!("{target}: {err}"));
    eprintln!(
        "[graphify] WARNING: could not scan {target} ({err}); \
         its files are missing from this run's enumeration."
    );
}

/// The path an `ignore_walk::Error` is associated with, unwrapping the
/// `WithDepth` / `WithLineNumber` wrappers around a `WithPath`. `None` for
/// path-less errors.
fn ignore_error_path(err: &ignore_walk::Error) -> Option<&Path> {
    match err {
        ignore_walk::Error::WithPath { path, .. } => Some(path),
        ignore_walk::Error::WithDepth { err, .. }
        | ignore_walk::Error::WithLineNumber { err, .. } => ignore_error_path(err),
        _ => None,
    }
}

/// Drop files that are symlinks resolving outside `root`, recording each in
/// `sym_skipped` (009a98b). Mirrors Python's per-file `_resolves_under_root` check.
fn retain_contained(
    files: Vec<PathBuf>,
    root: &Path,
    sym_skipped: &mut Vec<String>,
) -> Vec<PathBuf> {
    let mut kept: Vec<PathBuf> = Vec::with_capacity(files.len());
    for p in files {
        if p.is_symlink() && !resolves_under_root(&p, root) {
            sym_skipped.push(format!(
                "{} [symlink target outside scan root]",
                p.display()
            ));
        } else {
            kept.push(p);
        }
    }
    kept
}

/// Walk the project + sidecar memory tree, deduplicating against `seen`. Returns
/// the discovered files, directories whose scan failed (`walk_errors`), and paths
/// skipped because a symlink target escaped the scan root (`sym_skipped`, 009a98b).
fn run_walk_phase(
    ctx: &WalkCtx<'_>,
    root: &Path,
    memory_dir: &Path,
) -> (Vec<PathBuf>, Vec<String>, Vec<String>, Vec<String>) {
    let scan_paths: Vec<(PathBuf, bool)> = {
        let mut v = vec![(root.to_path_buf(), false)];
        if memory_dir.exists() {
            v.push((memory_dir.to_path_buf(), true));
        }
        v
    };
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut all_files: Vec<PathBuf> = Vec::new();
    let mut walk_errors: Vec<String> = Vec::new();
    let mut sym_skipped: Vec<String> = Vec::new();
    let mut ignored: Vec<String> = Vec::new();
    let t_walk = std::time::Instant::now();
    for (scan_root, in_memory) in &scan_paths {
        if *in_memory {
            // The in-memory sidecar tree disables noise-dir and ignore filtering;
            // the existing sequential walker carries those rules.
            walk_dir(
                ctx,
                scan_root,
                *in_memory,
                &mut seen,
                &mut all_files,
                &mut walk_errors,
                &mut sym_skipped,
            );
        } else {
            // Main project tree: dispatch to the parallel walker.
            let (found, errs, sym, ign) = walk_dir_parallel(ctx, scan_root);
            for p in found {
                if seen.insert(p.clone()) {
                    all_files.push(p);
                }
            }
            walk_errors.extend(errs);
            sym_skipped.extend(sym);
            ignored.extend(ign);
        }
    }
    // Per-file containment (009a98b): drop symlinked files whose target escapes
    // the scan root, recording each like an escaped directory.
    let mut all_files = retain_contained(all_files, root, &mut sym_skipped);
    // Sort lexicographically by the path's string form so classification (and
    // therefore graph.json) is deterministic regardless of the parallel walker's
    // completion order (8db19d6). Sort by the full string, not by PathBuf
    // components, to match Python's `sorted(key=str)`.
    all_files.sort_by_cached_key(|p| p.to_string_lossy().into_owned());
    walk_errors.sort();
    sym_skipped.sort();
    sym_skipped.dedup();
    if std::env::var("GRAPHIFY_PERF_LOG").is_ok() {
        eprintln!(
            "[perf]   walk_dir: {:.2}s ({} files)",
            t_walk.elapsed().as_secs_f64(),
            all_files.len()
        );
    }
    // `ignored` (dir-level prunes) is merged with file-level ignores and sorted
    // once in `detect_with_cache_root`.
    (all_files, walk_errors, sym_skipped, ignored)
}

/// Output of [`run_classify_phase`].
struct ClassifyOutput {
    files: IndexMap<String, Vec<String>>,
    to_count: Vec<(PathBuf, FileType)>,
    skipped_sensitive: Vec<String>,
    unclassified: Vec<String>,
    ignored: Vec<String>,
}

/// Classify each file, dispatch sidecar conversions, and return the per-kind file
/// map, word-count work list, and skipped-sensitive list.
fn run_classify_phase(
    all_files: &[PathBuf],
    root: &Path,
    memory_dir: &Path,
    converted_dir: &Path,
    ignore_patterns: &crate::ignore::IgnorePatterns,
    include_patterns: &crate::ignore::IgnorePatterns,
    google_workspace: bool,
) -> ClassifyOutput {
    let mut files: IndexMap<String, Vec<String>> = FILE_TYPE_KINDS
        .iter()
        .map(|k| ((*k).to_string(), Vec::new()))
        .collect();
    let mut to_count: Vec<(PathBuf, FileType)> = Vec::new();
    let mut skipped_sensitive: Vec<String> = Vec::new();
    let mut unclassified: Vec<String> = Vec::new();
    let mut ignored: Vec<String> = Vec::new();

    let t_phase1 = std::time::Instant::now();
    // Phase 1a (parallel): classify each file independently.
    let decisions: Vec<FileDecision> = if all_files.len() >= PARALLEL_COUNT_THRESHOLD {
        all_files
            .par_iter()
            .map(|p| {
                classify_one(
                    p,
                    root,
                    memory_dir,
                    converted_dir,
                    ignore_patterns,
                    include_patterns,
                )
            })
            .collect()
    } else {
        all_files
            .iter()
            .map(|p| {
                classify_one(
                    p,
                    root,
                    memory_dir,
                    converted_dir,
                    ignore_patterns,
                    include_patterns,
                )
            })
            .collect()
    };

    // Phase 1b (sequential): apply decisions and run sidecar conversions.
    let mut convert_ctx = ConvertCtx {
        root,
        converted_dir,
        ignore_patterns,
        include_patterns,
        files: &mut files,
        to_count: &mut to_count,
        skipped_sensitive: &mut skipped_sensitive,
        unclassified: &mut unclassified,
        ignored: &mut ignored,
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

    unclassified.sort();
    ClassifyOutput {
        files,
        to_count,
        skipped_sensitive,
        unclassified,
        ignored,
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
        FileDecision::Unclassified(rendered) => ctx.unclassified.push(rendered),
        FileDecision::Sensitive(rendered) => ctx.skipped_sensitive.push(rendered),
        FileDecision::Ignored(rendered) => ctx.ignored.push(rendered),
        FileDecision::Direct(ftype) => {
            ctx.files
                .entry(ftype.as_str().to_string())
                .or_default()
                .push(p.to_string_lossy().into_owned());
            ctx.to_count.push((p.to_path_buf(), ftype));
        }
        FileDecision::GoogleWorkspace(ftype) => {
            convert_google_workspace(ctx, p, ftype, google_workspace);
        }
        FileDecision::Office(ftype) => {
            convert_office(ctx, p, ftype);
        }
    }
}

/// Sum word counts across `to_count`, dispatching to Rayon for large lists.
fn run_word_count_phase(
    to_count: &[(PathBuf, FileType)],
    root: &Path,
    cache_root: Option<&Path>,
) -> u64 {
    let t_phase2 = std::time::Instant::now();
    // Cache each count against the file's stat signature so unchanged PDFs/docx
    // aren't re-parsed on every run just to size the corpus (#1656). cache_root
    // (when given, e.g. from `extract --out`) keeps the cache out of the corpus (#1747).
    let count = |p: &Path, ftype: FileType| {
        graphify_cache::cached_word_count(p, root, |pp| count_words(pp, ftype), cache_root)
    };
    let total = if to_count.len() >= PARALLEL_COUNT_THRESHOLD {
        to_count
            .par_iter()
            .map(|(p, ftype)| count(p, *ftype))
            .sum::<u64>()
    } else {
        to_count
            .iter()
            .map(|(p, ftype)| count(p, *ftype))
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
    include_patterns: &'a IgnorePatterns,
    files: &'a mut IndexMap<String, Vec<String>>,
    to_count: &'a mut Vec<(PathBuf, FileType)>,
    skipped_sensitive: &'a mut Vec<String>,
    unclassified: &'a mut Vec<String>,
    ignored: &'a mut Vec<String>,
}

impl ConvertCtx<'_> {
    /// Register a converted sidecar file produced from `src`.
    ///
    /// The sidecar inherits its source's allowlist verdict: an ordinary source
    /// still has its sidecar filtered through `.graphifyignore` (matching
    /// graphify-py `detect()`), but a source rescued by `.graphifyinclude`
    /// keeps its sidecar even when the converted path would otherwise be
    /// ignored. The allowlist is keyed on source paths, so the verdict is taken
    /// from `src` rather than the derived `md_path` — otherwise a global ignore
    /// such as `*` would silently drop the very content the rescue was meant to
    /// keep (the `Direct` path already honours this in `classify_one`). Word
    /// counting is deferred to a parallel pass after all conversions complete.
    fn record(&mut self, src: &Path, md_path: &Path, ftype: FileType) {
        let source_rescued = is_included(src, self.root, self.include_patterns);
        if !source_rescued && is_ignored(md_path, self.root, self.ignore_patterns) {
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
fn convert_google_workspace(
    ctx: &mut ConvertCtx<'_>,
    p: &Path,
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
        Ok(Some(md_path)) => ctx.record(p, &md_path, ftype),
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
        Ok(Some(md_path)) => ctx.record(p, &md_path, ftype),
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
