//! `.graphifyignore` / `.gitignore` loading and matching.
//!
//! Ports `_parse_gitignore_line`, `_find_vcs_root`, `_load_graphifyignore`,
//! `_is_ignored`, `_load_graphifyinclude`, `_is_included`, and
//! `_could_contain_included_path` from `graphify-py/graphify/detect.py`.

use std::path::{Path, PathBuf};

use regex::Regex;

const VCS_MARKERS: &[&str] = &[".git", ".hg", ".svn", "_darcs", ".fossil"];

/// Ordered list of `(anchor_dir, pattern)` pairs loaded from `.graphifyignore` or `.gitignore` files.
///
/// `anchor_dir` is the directory that contains the ignore file; `pattern` is
/// the raw gitignore-style pattern string (never empty — blank lines and
/// comments are stripped during loading).
pub type IgnorePatterns = Vec<(PathBuf, String)>;

// ── Pattern parsing ──────────────────────────────────────────────────────────

/// Inline comment stripper: whitespace + one-or-more # + rest.
#[allow(clippy::expect_used)]
static INLINE_COMMENT_RE: std::sync::LazyLock<Regex> =
    std::sync::LazyLock::new(|| Regex::new(r"\s+#+[^\\].*$").expect("literal pattern is valid"));

/// Parse one raw line from a `.graphifyignore` or `.gitignore` file.
///
/// Returns an empty string for blank lines and full-line comments.
#[must_use]
pub fn parse_gitignore_line(raw: &str) -> String {
    let line = raw.trim_end_matches(['\n', '\r']);
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return String::new();
    }
    // Strip inline comments (whitespace + # suffix)
    let line = INLINE_COMMENT_RE.replace(line, "");
    // Unescape \# → literal #
    let line = line.replace("\\#", "#");
    // Remove unescaped trailing spaces (replace lookbehind with manual trim).
    // Per gitignore spec: strip trailing spaces that aren't preceded by '\'.
    trim_unescaped_trailing_spaces(&line)
}

/// Remove trailing space characters that are not escaped with `\`.
/// Strip trailing space characters that are not preceded by a backslash escape.
fn trim_unescaped_trailing_spaces(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut end = bytes.len();
    // Walk backwards eating spaces, but stop at first escaped space.
    while end > 0 && bytes[end - 1] == b' ' {
        // Check if this space is escaped
        if end >= 2 && bytes[end - 2] == b'\\' {
            break;
        }
        end -= 1;
    }
    s[..end].to_string()
}

// ── VCS root discovery ───────────────────────────────────────────────────────

/// Walk upward from `start`; return the first directory containing a VCS marker.
#[must_use]
pub fn find_vcs_root(start: &Path) -> Option<PathBuf> {
    let home = dirs_home();
    let mut current = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    loop {
        if VCS_MARKERS.iter().any(|m| current.join(m).exists()) {
            return Some(current);
        }
        let parent = current.parent().map(Path::to_path_buf);
        match parent {
            None => return None,
            Some(p) if p == current => return None,
            Some(p) => {
                if let Some(h) = &home
                    && current == *h
                {
                    return None;
                }
                current = p;
            }
        }
    }
}

/// Returns the user's home directory from `$HOME`, or `None` if unset.
fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

// ── .graphifyignore / .gitignore loading ─────────────────────────────────────

/// Resolve `$GIT_DIR/info/exclude` for the repo rooted at `vcs_root`.
///
/// `info/exclude` records local-only, uncommitted excludes — and is where
/// `git worktree add` writes nested worktree paths — so a repo can ignore a
/// directory without any `.gitignore` entry (#1810). Handles the
/// linked-worktree / submodule case where `.git` is a FILE (`gitdir: <path>`)
/// and the real excludes live in the shared common git dir (via `commondir`).
/// Returns `None` when there is no readable exclude file.
#[must_use]
pub fn git_info_exclude(vcs_root: &Path) -> Option<PathBuf> {
    let dot_git = vcs_root.join(".git");
    let git_dir: PathBuf = if dot_git.is_dir() {
        dot_git
    } else if dot_git.is_file() {
        let content = std::fs::read_to_string(&dot_git).unwrap_or_default();
        let gd_str = content.trim().strip_prefix("gitdir:")?.trim().to_owned();
        let gd_raw = PathBuf::from(gd_str);
        let gd = if gd_raw.is_absolute() {
            gd_raw
        } else {
            let joined = vcs_root.join(&gd_raw);
            joined.canonicalize().unwrap_or(joined)
        };
        // A linked worktree's gitdir holds a `commondir` file pointing at the
        // shared git dir, where info/exclude actually lives.
        let commondir = gd.join("commondir");
        match std::fs::read_to_string(&commondir) {
            Ok(cd_raw) if !cd_raw.trim().is_empty() => {
                let cd = PathBuf::from(cd_raw.trim());
                if cd.is_absolute() {
                    cd
                } else {
                    let joined = gd.join(&cd);
                    joined.canonicalize().unwrap_or(joined)
                }
            }
            _ => gd,
        }
    } else {
        return None;
    };
    let exclude = git_dir.join("info").join("exclude");
    exclude.is_file().then_some(exclude)
}

/// Read `.gitignore` then `.graphifyignore` directly inside `dir` (not its
/// ancestors), returning `(anchor_dir, pattern)` pairs anchored at `dir`.
///
/// `.gitignore` is read first and `.graphifyignore` last, so `.graphifyignore`
/// patterns — including `!` negations — win on conflict via last-match-wins
/// (#1363; #945 keeps a dir with only a `.gitignore` getting sensible
/// defaults). Shared by [`load_graphifyignore`] (the ancestor chain, loaded
/// once before the scan) and the live descendant walk in `walk.rs`, so nested
/// ignore files below the scan root are honored too — previously only the scan
/// root and its ancestors were read (#1206).
#[must_use]
pub fn load_dir_own_ignore(dir: &Path) -> IgnorePatterns {
    let mut patterns: IgnorePatterns = Vec::new();
    for fname in [".gitignore", ".graphifyignore"] {
        let ignore_file = dir.join(fname);
        if ignore_file.exists()
            && let Ok(text) = std::fs::read_to_string(&ignore_file)
        {
            for raw in text.lines() {
                let line = parse_gitignore_line(raw);
                if !line.is_empty() {
                    patterns.push((dir.to_path_buf(), line));
                }
            }
        }
    }
    patterns
}

/// Read `.gitignore` then `.graphifyignore` files from `root` upward to the VCS
/// ceiling and return `(anchor_dir, pattern)` pairs.
///
/// Outer-first (ceiling first, scan root last) so inner rules win via
/// last-match-wins semantics, matching gitignore behaviour exactly. Within a
/// single directory both files are merged via [`load_dir_own_ignore`] (#1363).
/// `$GIT_DIR/info/exclude` is prepended at lowest precedence, anchored at the
/// VCS ceiling, so a nearer `!` re-include still overrides it (#1810). Covers
/// the scan root and its ancestors only — directories below the scan root are
/// picked up live during the walk in `walk.rs` (#1206).
#[must_use]
pub fn load_graphifyignore(root: &Path) -> IgnorePatterns {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let ceiling = find_vcs_root(&root).unwrap_or_else(|| root.clone());
    let dirs = build_dir_list(&root, &ceiling);

    let mut patterns: IgnorePatterns = Vec::new();

    // $GIT_DIR/info/exclude is repo-root-scoped and, per git, ranks below every
    // per-directory .gitignore/.graphifyignore — so load it first (lowest
    // priority under last-match-wins) anchored at the VCS ceiling (#1810).
    if let Some(info_exclude) = git_info_exclude(&ceiling)
        && let Ok(text) = std::fs::read_to_string(&info_exclude)
    {
        for raw in text.lines() {
            let line = parse_gitignore_line(raw);
            if !line.is_empty() {
                patterns.push((ceiling.clone(), line));
            }
        }
    }

    for dir in &dirs {
        patterns.extend(load_dir_own_ignore(dir));
    }
    patterns
}

/// Read `.graphifyinclude` allowlist patterns from `root` and ancestors.
#[must_use]
pub fn load_graphifyinclude(root: &Path) -> IgnorePatterns {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let ceiling = find_vcs_root(&root).unwrap_or_else(|| root.clone());
    let dirs = build_dir_list(&root, &ceiling);

    let mut patterns: IgnorePatterns = Vec::new();
    for dir in &dirs {
        let include_file = dir.join(".graphifyinclude");
        if include_file.exists()
            && let Ok(text) = std::fs::read_to_string(&include_file)
        {
            for raw in text.lines() {
                let line = parse_gitignore_line(raw);
                if !line.is_empty() {
                    patterns.push((dir.clone(), line));
                }
            }
        }
    }
    patterns
}

/// Builds the ancestor directory list from `ceiling` down to `root` (outer-first) for ordered pattern loading.
fn build_dir_list(root: &Path, ceiling: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut current = root.to_path_buf();
    loop {
        dirs.push(current.clone());
        if current == ceiling {
            break;
        }
        match current.parent() {
            Some(p) => current = p.to_path_buf(),
            None => break,
        }
    }
    dirs.reverse(); // ceiling first, root last
    dirs
}

// ── Matching helpers ─────────────────────────────────────────────────────────

/// Returns `true` if `name` matches shell-style glob `pattern`.
fn fnmatch(name: &str, pattern: &str) -> bool {
    glob_match(name, pattern)
}

/// Minimal gitignore-compatible glob matcher.
///
/// Supports `*` (match any non-separator chars), `**` (match any path segment),
/// and `?` (match single char). Case-sensitive.
fn glob_match(text: &str, pat: &str) -> bool {
    glob_match_inner(text.as_bytes(), pat.as_bytes())
}

/// Recursive byte-level glob matcher backing [`glob_match`].
fn glob_match_inner(text: &[u8], pat: &[u8]) -> bool {
    match (text, pat) {
        (_, []) => text.is_empty(),
        ([], [b'*', rest @ ..]) => glob_match_inner(text, rest),
        ([], _) => false,
        (_, [b'*', b'*', rest @ ..]) => {
            // ** matches zero or more path components
            if glob_match_inner(text, rest) {
                return true;
            }
            for i in 0..=text.len() {
                if glob_match_inner(&text[i..], rest) {
                    return true;
                }
                if i < text.len() && text[i] == b'/' && glob_match_inner(&text[i + 1..], rest) {
                    return true;
                }
            }
            false
        }
        (_, [b'*', rest @ ..]) => {
            // * matches zero or more non-separator chars
            let mut i = 0;
            loop {
                if glob_match_inner(&text[i..], rest) {
                    return true;
                }
                if i >= text.len() || text[i] == b'/' {
                    break;
                }
                i += 1;
            }
            false
        }
        ([tc, trest @ ..], [b'?', prest @ ..]) => *tc != b'/' && glob_match_inner(trest, prest),
        ([tc, trest @ ..], [pc, prest @ ..]) => tc == pc && glob_match_inner(trest, prest),
    }
}

/// Converts a path to a forward-slash string for portable pattern matching.
fn path_to_forward_slash(path: &Path) -> String {
    path.to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/")
}

/// Returns `true` if `rel` (forward-slash relative path) or any of its prefix segments matches pattern `p`.
///
/// Optimised to avoid per-segment allocations: instead of joining
/// `parts[..=i]` into a new `String` to form a prefix, we slice into the
/// existing `rel` string using `/` offsets. This is the hottest function
/// inside the per-file `is_ignored` check on large corpora.
fn rel_matches(rel: &str, target_name: &str, p: &str) -> bool {
    if fnmatch(rel, p) {
        return true;
    }
    if fnmatch(target_name, p) {
        return true;
    }
    // Walk segments by byte offset; each iteration considers both the
    // standalone segment and the `rel[..end]` prefix containing it.
    let bytes = rel.as_bytes();
    let mut seg_start = 0usize;
    for i in 0..=bytes.len() {
        let at_end = i == bytes.len();
        if at_end || bytes[i] == b'/' {
            // Segment is rel[seg_start..i]; prefix is rel[..i].
            let segment = &rel[seg_start..i];
            if fnmatch(segment, p) {
                return true;
            }
            let prefix = &rel[..i];
            if fnmatch(prefix, p) {
                return true;
            }
            seg_start = i + 1;
        }
    }
    false
}

/// Evaluates all patterns against `target` using last-match-wins, returning the final ignored state.
///
/// Each pattern is matched ONLY against the path relative to its own anchor
/// directory (the directory that contained the ignore file). Per gitignore
/// semantics, patterns from `A/.gitignore` apply only to paths under `A` — so a
/// nested ignore file's bare `*` cannot leak out and ignore the whole corpus
/// (#1873). A pattern whose anchor does not contain `target` is skipped, and the
/// anchor directory itself is exempt (an ignore file governs its directory's
/// contents, not the directory).
///
/// Optimised hot path: the anchor-relative string is computed once per unique
/// anchor instead of once per pattern. Patterns are loaded outer-first (all
/// patterns from one ignore file are contiguous), so a single-element cache
/// keyed on the last-seen anchor pointer is sufficient.
fn eval_path(target: &Path, patterns: &IgnorePatterns) -> bool {
    let target_name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // Tracks the last-seen anchor pointer and its cached anchor-relative path
    // (`None` when `target` lies outside that anchor). The outer `Option` marks
    // whether the cache slot is populated for the current anchor.
    let mut last_anchor: *const PathBuf = std::ptr::null();
    let mut last_anchor_rel: Option<Option<String>> = None;

    let mut result = false;
    for (anchor, pattern) in patterns {
        let negated = pattern.starts_with('!');
        let raw = if negated {
            &pattern[1..]
        } else {
            pattern.as_str()
        };
        let anchored = raw.starts_with('/');
        let p = raw.trim_matches('/');
        if p.is_empty() {
            continue;
        }

        let anchor_ptr = std::ptr::from_ref::<PathBuf>(anchor);
        if !std::ptr::eq(last_anchor, anchor_ptr) {
            last_anchor = anchor_ptr;
            // Anchors from `load_graphifyignore` are canonicalized. A caller that
            // passes a non-canonical target (e.g. macOS `/var` vs `/private/var`,
            // or the XAML/watch scan roots) would fail the direct strip, so fall
            // back to canonicalizing the target before relativising — keeping the
            // canonical hot path (detect) allocation- and syscall-free.
            last_anchor_rel = Some(
                target
                    .strip_prefix(anchor)
                    .ok()
                    .map(path_to_forward_slash)
                    .or_else(|| {
                        target
                            .canonicalize()
                            .ok()
                            .and_then(|c| c.strip_prefix(anchor).map(path_to_forward_slash).ok())
                    }),
            );
        }

        // `target` outside this pattern's anchor: the pattern cannot match it.
        let Some(Some(rel)) = last_anchor_rel.as_ref() else {
            continue;
        };
        // The anchor dir itself (empty relative path) is exempt.
        if rel.is_empty() {
            continue;
        }

        let matched = if anchored {
            fnmatch(rel, p)
        } else {
            rel_matches(rel, target_name, p)
        };

        if matched {
            result = !negated;
        }
    }
    result
}

/// Per-scan memoization cache for [`is_ignored_with_cache`]: maps an evaluated
/// target path to its last-match-wins ignore verdict. Sharing one across a scan
/// means files under the same subtree do not re-evaluate the same ancestor
/// patterns. Mirrors Python's `_is_ignored(..., _cache=)`.
pub type IgnoreEvalCache = std::collections::HashMap<std::path::PathBuf, bool>;

/// Return `true` if the path should be ignored per `.graphifyignore` patterns.
///
/// Uses gitignore last-match-wins semantics with parent-exclusion rule:
/// a `!` re-include cannot rescue a file whose ancestor directory is excluded.
#[must_use]
pub fn is_ignored(path: &Path, root: &Path, patterns: &IgnorePatterns) -> bool {
    is_ignored_impl(path, root, patterns, None)
}

/// Like [`is_ignored`], but memoizes per-target `eval_path` results in `cache`.
///
/// The verdict is identical to [`is_ignored`]; the cache only avoids
/// re-evaluating shared ancestor directories. Pass a single cache across all
/// calls in one scan.
#[must_use]
pub fn is_ignored_with_cache(
    path: &Path,
    root: &Path,
    patterns: &IgnorePatterns,
    cache: &mut IgnoreEvalCache,
) -> bool {
    is_ignored_impl(path, root, patterns, Some(cache))
}

/// Shared body for [`is_ignored`] / [`is_ignored_with_cache`].
fn is_ignored_impl(
    path: &Path,
    root: &Path,
    patterns: &IgnorePatterns,
    mut cache: Option<&mut IgnoreEvalCache>,
) -> bool {
    if patterns.is_empty() {
        return false;
    }

    // Gitignore parent-exclusion rule: walk ancestors top-down; if any is
    // excluded, the file is excluded regardless of later ! patterns.
    let rel_parts: Vec<_> = path
        .strip_prefix(root)
        .unwrap_or(path)
        .components()
        .collect();

    let mut ancestor = root.to_path_buf();
    for part in rel_parts.iter().take(rel_parts.len().saturating_sub(1)) {
        ancestor = ancestor.join(part);
        if eval_cached(&ancestor, patterns, cache.as_deref_mut()) {
            return true;
        }
    }
    eval_cached(path, patterns, cache)
}

/// Evaluate `target`, consulting/populating `cache` when present.
fn eval_cached(
    target: &Path,
    patterns: &IgnorePatterns,
    cache: Option<&mut IgnoreEvalCache>,
) -> bool {
    match cache {
        Some(c) => {
            if let Some(&v) = c.get(target) {
                return v;
            }
            let v = eval_path(target, patterns);
            c.insert(target.to_path_buf(), v);
            v
        }
        None => eval_path(target, patterns),
    }
}

/// Whether an anchored `.graphifyinclude` stem `p` (anchor-relative, no
/// surrounding slashes) matches the anchor-relative path `rel_str` — the path
/// itself or anything in its subtree.
///
/// For an allowlist an anchored directory (`/src`) covers its descendants
/// (`src/main.py`), mirroring [`could_contain_included_path`]. This is the
/// inverse of the anchored *ignore* fix (#1087): an ignore pattern must not leak
/// into a subtree, but an include directory pulls its whole subtree in. A literal
/// stem uses a zero-alloc `strip_prefix` check (`{p}/...`); a globbed stem
/// (`src*`) needs `{p}/**` since this matcher's `*` does not cross `/` (the
/// `format!` runs only for the rare globbed-include case).
fn anchored_include_matches(rel_str: &str, p: &str) -> bool {
    fnmatch(rel_str, p)
        || rel_str
            .strip_prefix(p)
            .is_some_and(|rest| rest.starts_with('/'))
        || ((p.contains('*') || p.contains('?')) && fnmatch(rel_str, &format!("{p}/**")))
}

/// Return `true` if `path` matches any `.graphifyinclude` allowlist pattern.
#[must_use]
pub fn is_included(path: &Path, root: &Path, patterns: &IgnorePatterns) -> bool {
    if patterns.is_empty() {
        return false;
    }

    let target_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    for (anchor, pattern) in patterns {
        let anchored = pattern.starts_with('/');
        let p = pattern.trim_matches('/');
        if p.is_empty() {
            continue;
        }
        if anchored {
            if path
                .strip_prefix(anchor)
                .is_ok_and(|rel| anchored_include_matches(&path_to_forward_slash(rel), p))
            {
                return true;
            }
        } else {
            let root_matched = path.strip_prefix(root).is_ok_and(|rel| {
                let rel_str = path_to_forward_slash(rel);
                rel_matches(&rel_str, target_name, p)
            });
            if root_matched {
                return true;
            }
            if anchor != root
                && path.strip_prefix(anchor).is_ok_and(|rel| {
                    let rel_str = path_to_forward_slash(rel);
                    rel_matches(&rel_str, target_name, p)
                })
            {
                return true;
            }
        }
    }
    false
}

/// Return `true` if a directory may contain files matched by `.graphifyinclude`.
///
/// Used to keep the walker from pruning a subtree that the allowlist would
/// later accept. It mirrors [`anchored_include_matches`] so a directory is kept
/// when it is the matched stem, lives inside the stem's subtree (a literal
/// `/src` or globbed `/src*` covering `src/deep/main.py`), or is an ancestor of
/// a more-specific pattern target.
#[must_use]
pub fn could_contain_included_path(path: &Path, root: &Path, patterns: &IgnorePatterns) -> bool {
    if patterns.is_empty() {
        return false;
    }

    let mut rels: Vec<String> = Vec::new();
    if let Ok(rel) = path.strip_prefix(root) {
        rels.push(path_to_forward_slash(rel));
    }
    for (anchor, _) in patterns {
        if anchor != root
            && let Ok(rel) = path.strip_prefix(anchor)
        {
            rels.push(path_to_forward_slash(rel));
        }
    }

    for rel in &rels {
        let rel = rel.trim_matches('/');
        if rel.is_empty() {
            return true;
        }
        for (_, pattern) in patterns {
            let p = pattern.trim_matches('/');
            if p.is_empty() {
                continue;
            }
            // `dir` is an ancestor of a more-specific pattern target (e.g. dir
            // `src`, pattern `src/a/b.py`): descend to reach the target.
            if p == rel || p.starts_with(&format!("{rel}/")) {
                return true;
            }
            // `dir` is the matched stem itself or lives inside its subtree. Use
            // the same anchored-subtree test as `is_included` so the
            // descendant-covering semantics (a literal `/src` or globbed `/src*`
            // pulling in `src/deep/main.py`) are honoured during traversal —
            // otherwise the walker prunes subtrees the allowlist would accept.
            if anchored_include_matches(rel, p) {
                return true;
            }
        }
    }
    false
}
