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

/// Read `.graphifyignore` (falling back to `.gitignore`) files from `root` upward
/// to the VCS ceiling and return `(anchor_dir, pattern)` pairs.
///
/// Outer-first (ceiling first, scan root last) so inner rules win via
/// last-match-wins semantics, matching gitignore behaviour exactly.
#[must_use]
pub fn load_graphifyignore(root: &Path) -> IgnorePatterns {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let ceiling = find_vcs_root(&root).unwrap_or_else(|| root.clone());
    let dirs = build_dir_list(&root, &ceiling);

    let mut patterns: IgnorePatterns = Vec::new();
    for dir in &dirs {
        // Prefer .graphifyignore; fall back to .gitignore.
        let ignore_file = {
            let gfi = dir.join(".graphifyignore");
            if gfi.exists() {
                gfi
            } else {
                dir.join(".gitignore")
            }
        };
        if ignore_file.exists()
            && let Ok(text) = std::fs::read_to_string(&ignore_file)
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
/// Optimised hot path: relative-path strings are computed once per unique
/// anchor instead of once per pattern. The original implementation re-ran
/// `target.strip_prefix(anchor)` + `path_to_forward_slash` for every pattern
/// even when many patterns share the same anchor. Patterns are loaded
/// outer-first (all patterns from one `.gitignore` are contiguous), so a
/// single-element cache keyed on the last-seen anchor pointer is sufficient.
fn eval_path(target: &Path, root: &Path, patterns: &IgnorePatterns) -> bool {
    let target_name = target.file_name().and_then(|n| n.to_str()).unwrap_or("");

    // Precompute `target` relative to `root` once — every non-anchored
    // pattern reuses it.
    let root_rel: Option<String> = target.strip_prefix(root).ok().map(path_to_forward_slash);

    // Tracks the last-seen anchor pointer and its cached relativised path.
    // Patterns are loaded outer-first, so consecutive runs of patterns from
    // the same `.gitignore` file all share an anchor.
    let mut last_anchor: *const PathBuf = std::ptr::null();
    let mut last_anchor_rel: Option<String> = None;

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

        let need_anchor_rel = anchored || anchor != root;
        let anchor_ptr = std::ptr::from_ref::<PathBuf>(anchor);
        if need_anchor_rel && !std::ptr::eq(last_anchor, anchor_ptr) {
            last_anchor = anchor_ptr;
            last_anchor_rel = target.strip_prefix(anchor).ok().map(path_to_forward_slash);
        }

        let matched = if anchored {
            // Anchored patterns match the anchor-relative path directly — no
            // subtree/basename fallback. Without this, `/inbox/` would leak into
            // `src/inbox/` deep in the tree via segment matching (#1087).
            last_anchor_rel
                .as_deref()
                .is_some_and(|rel| fnmatch(rel, p))
        } else {
            let root_matched = root_rel
                .as_deref()
                .is_some_and(|rel| rel_matches(rel, target_name, p));

            let anchor_matched = !root_matched
                && anchor != root
                && last_anchor_rel
                    .as_deref()
                    .is_some_and(|rel| rel_matches(rel, target_name, p));

            root_matched || anchor_matched
        };

        if matched {
            result = !negated;
        }
    }
    result
}

/// Return `true` if the path should be ignored per `.graphifyignore` patterns.
///
/// Uses gitignore last-match-wins semantics with parent-exclusion rule:
/// a `!` re-include cannot rescue a file whose ancestor directory is excluded.
#[must_use]
pub fn is_ignored(path: &Path, root: &Path, patterns: &IgnorePatterns) -> bool {
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
        if eval_path(&ancestor, root, patterns) {
            return true;
        }
    }
    eval_path(path, root, patterns)
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
            // Anchored include patterns match the anchor-relative path directly.
            // For an *allowlist*, an anchored directory (`/src`) also covers its
            // descendants (`src/main.py`), so a `"{p}/"`-prefix match is included
            // too — mirroring `could_contain_included_path`. This is the inverse
            // of the anchored *ignore* fix (#1087): an ignore pattern must not
            // leak into a subtree at the wrong depth, but an include directory is
            // meant to pull its whole subtree in.
            if path.strip_prefix(anchor).ok().is_some_and(|rel| {
                let rel_str = path_to_forward_slash(rel);
                fnmatch(&rel_str, p) || rel_str.starts_with(&format!("{p}/"))
            }) {
                return true;
            }
        } else {
            let root_matched = path.strip_prefix(root).ok().is_some_and(|rel| {
                let rel_str = path_to_forward_slash(rel);
                rel_matches(&rel_str, target_name, p)
            });
            if root_matched {
                return true;
            }
            if anchor != root
                && path.strip_prefix(anchor).ok().is_some_and(|rel| {
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
            if p == rel || p.starts_with(&format!("{rel}/")) {
                return true;
            }
            if fnmatch(rel, p) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
#[path = "ignore_tests.rs"]
mod ignore_tests;
