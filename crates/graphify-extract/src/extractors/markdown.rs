//! Markdown extractor — pure line-by-line parsing (no tree-sitter).

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node};
use regex::Regex;

#[allow(clippy::expect_used)] // literal regex pattern; cannot fail
static HEADING_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(#{1,6})\s+(.+)").expect("static heading regex"));

// Inline markdown link `[text](target "title")`. RE2 has no lookbehind, so the
// image form `![alt](src)` is excluded by checking the byte before the match in
// `scan_markdown_links`. The target stops at whitespace/`)`/`>` so an optional
// title and `<...>` wrapper are dropped.
#[allow(clippy::expect_used)] // literal regex pattern; cannot fail
static MD_INLINE_LINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+[^)]*)?\)")
        .expect("static md inline-link regex")
});
// Reference-style link definition line: `[label]: target "title"`.
#[allow(clippy::expect_used)] // literal regex pattern; cannot fail
static MD_REF_DEF_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s{0,3}\[[^\]]+\]:\s*<?([^\s>]+)>?").expect("static md ref-def regex")
});
// Obsidian-style wikilink `[[target]]` / `[[target|alias]]` / `[[target#anchor]]`.
#[allow(clippy::expect_used)] // literal regex pattern; cannot fail
static MD_WIKILINK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\[\[([^\]|#]+)(?:[#|][^\]]*)?\]\]").expect("static md wikilink regex")
});
// Extensions graphify creates document file nodes for; a link to one of these
// resolves to that file's node, links to code/assets are left to their own
// extractors.
const MD_LINKABLE_EXTS: &[&str] = &[".md", ".mdx", ".qmd", ".markdown", ".rst", ".txt"];

/// Extract structural nodes and edges from a Markdown file.
///
/// Emits a node per file and per heading, with `contains` edges nesting
/// headings by level. Fenced code blocks (both backtick and tilde fences) are
/// skipped during parsing so their contents are not misread as headings, but no
/// node is emitted for them — they were always orphans and inflated the
/// disconnected-component count (#1077).
#[must_use]
pub fn extract_markdown(path: &Path) -> FileResult {
    let source = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            return FileResult {
                nodes: vec![],
                edges: vec![],
                raw_calls: vec![],
                error: Some(e.to_string()),
            };
        }
    };

    let stem = file_stem(path);
    let str_path = path.to_string_lossy().into_owned();
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    let file_nid = make_id1(&str_path);
    seen_ids.insert(file_nid.clone());
    nodes.push(Node {
        id: file_nid.clone(),
        label: path
            .file_name()
            .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
        file_type: "document".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: None,
    });

    let mut heading_stack: Vec<(usize, String)> = Vec::new();
    // The currently-open fence as `(marker_char, run_length)`, or `None`
    // outside a fenced block. Tracking the marker char (rather than a bool)
    // lets a `~~~` inside a ``` block — or vice versa — not prematurely close
    // the block; tracking the run length enforces the CommonMark rule that a
    // closing fence must repeat the opening marker at least as many times, so a
    // nested ``` inside a ```` block does not close the outer block early.
    let mut fence: Option<(char, usize)> = None;

    let source_dir: &Path = path.parent().unwrap_or_else(|| Path::new("."));
    // Dedup link edges by resolved target so a hub doc linking the same sibling
    // many times yields one edge (keeps weights meaningful).
    let mut linked_targets: HashSet<String> = HashSet::new();
    let mut ctx = LineCtx {
        stem: &stem,
        file_nid: &file_nid,
        str_path: &str_path,
        nodes: &mut nodes,
        edges: &mut edges,
        seen_ids: &mut seen_ids,
        source_dir,
        linked_targets: &mut linked_targets,
    };
    for (line_num_0, line_text) in source.lines().enumerate() {
        let line_num = line_num_0 + 1;
        // Skip over fenced code blocks so their contents are not parsed as
        // headings, but emit no nodes/edges for them (#1077): they were always
        // orphans (a single contains edge to the parent doc) and inflated the
        // disconnected-component count.
        //
        // Divergence from graphify-py: the Python parser only recognises ```
        // fences, so a `~~~` code block leaks its contents as phantom heading
        // nodes. Both ``` and ~~~ are valid CommonMark fences, so the Rust port
        // honours both.
        // CommonMark allows a fence to be indented by at most three spaces; four
        // or more leading spaces make the line an indented code block, not a
        // fence. Count leading spaces explicitly rather than trimming so an
        // over-indented ``` is not mistaken for a fence.
        let leading_spaces = line_text.chars().take_while(|&c| c == ' ').count();
        let trimmed = &line_text[leading_spaces..];
        let marker = if leading_spaces > 3 {
            None
        } else if trimmed.starts_with("```") {
            Some('`')
        } else if trimmed.starts_with("~~~") {
            Some('~')
        } else {
            None
        };
        if let Some(marker) = marker {
            let marker_len = trimmed.chars().take_while(|&c| c == marker).count();
            match fence {
                None => fence = Some((marker, marker_len)),
                // Close only on the same marker repeated at least as many times
                // as the opening fence, with nothing but optional whitespace
                // after the run (CommonMark). A shorter or mismatched run, or a
                // closing line carrying an info string (e.g. ```text), does not
                // close the block.
                Some((open_ch, open_len))
                    if open_ch == marker
                        && marker_len >= open_len
                        && trimmed[marker_len..].trim().is_empty() =>
                {
                    fence = None;
                }
                Some(_) => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        // Markdown links -> document references (#1376). Scanned on every
        // non-fenced line (including heading lines, which the heading branch
        // below skips past) so links anywhere in the doc are captured.
        scan_markdown_links(&mut ctx, line_text, line_num);
        if let Some(cap) = HEADING_RE.captures(line_text) {
            handle_heading(&mut ctx, &mut heading_stack, &cap, line_num);
        }
    }

    FileResult {
        nodes,
        edges,
        raw_calls: vec![],
        error: None,
    }
}

/// Per-file context passed to the line handlers.
struct LineCtx<'a> {
    stem: &'a str,
    file_nid: &'a str,
    str_path: &'a str,
    nodes: &'a mut Vec<Node>,
    edges: &'a mut Vec<Edge>,
    seen_ids: &'a mut HashSet<String>,
    source_dir: &'a Path,
    linked_targets: &'a mut HashSet<String>,
}

/// Emit a heading node, attach it to its parent, and update the heading stack.
fn handle_heading(
    ctx: &mut LineCtx<'_>,
    heading_stack: &mut Vec<(usize, String)>,
    cap: &regex::Captures<'_>,
    line_num: usize,
) {
    let level = cap[1].len();
    let title = cap[2].trim().to_string();
    let mut h_nid = make_id(&[ctx.stem, &title]);
    if ctx.seen_ids.contains(&h_nid) {
        h_nid = make_id(&[ctx.stem, &title, &line_num.to_string()]);
    }
    if ctx.seen_ids.insert(h_nid.clone()) {
        ctx.nodes.push(Node {
            id: h_nid.clone(),
            label: title,
            file_type: "document".to_string(),
            source_file: ctx.str_path.to_string(),
            source_location: Some(format!("L{line_num}")),
            metadata: None,
        });
    }
    while heading_stack.last().is_some_and(|(lvl, _)| *lvl >= level) {
        heading_stack.pop();
    }
    let parent = heading_stack
        .last()
        .map_or(ctx.file_nid, |(_, nid)| nid.as_str());
    ctx.edges.push(Edge {
        external: false,
        source: parent.to_string(),
        target: h_nid.clone(),
        relation: "contains".to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: ctx.str_path.to_string(),
        source_location: Some(format!("L{line_num}")),
        weight: 1.0,
        context: None,
        confidence_score: None,
    });
    heading_stack.push((level, h_nid));
}

/// Scan one non-fenced line for inline, wikilink, and reference-style links,
/// emitting a `references` edge to each resolved sibling document.
fn scan_markdown_links(ctx: &mut LineCtx<'_>, line_text: &str, line_num: usize) {
    let bytes = line_text.as_bytes();
    // RE2 has no lookbehind: an image `![alt](src)` / `![[embed]]` has a `!`
    // immediately before the `[`, so skip a match whose preceding byte is `!`.
    let not_image = |start: usize| start == 0 || bytes[start - 1] != b'!';
    for m in MD_INLINE_LINK_RE.captures_iter(line_text) {
        let (Some(whole), Some(target)) = (m.get(0), m.get(1)) else {
            continue;
        };
        if not_image(whole.start()) {
            add_markdown_link(ctx, target.as_str(), line_num);
        }
    }
    for m in MD_WIKILINK_RE.captures_iter(line_text) {
        let (Some(whole), Some(target)) = (m.get(0), m.get(1)) else {
            continue;
        };
        if not_image(whole.start()) {
            add_markdown_link(ctx, target.as_str(), line_num);
        }
    }
    if let Some(m) = MD_REF_DEF_RE.captures(line_text)
        && let Some(target) = m.get(1)
    {
        add_markdown_link(ctx, target.as_str(), line_num);
    }
}

/// Resolve and record a single markdown link as a `references` edge from the
/// file node to the target document's node, deduped and skipping self-links.
fn add_markdown_link(ctx: &mut LineCtx<'_>, raw: &str, line_num: usize) {
    let Some(resolved) = resolve_markdown_link(raw, ctx.source_dir) else {
        return;
    };
    // Build the target ID with the SAME recipe as the target file's own node
    // (`make_id1` of the path). Using the resolved absolute path means both this
    // edge endpoint and the target file node get remapped identically by
    // `extract()`'s id remap, so the edge merges onto the real node (no ghost).
    let tgt_nid = make_id1(&resolved.to_string_lossy());
    if tgt_nid == ctx.file_nid || !ctx.linked_targets.insert(tgt_nid.clone()) {
        return;
    }
    ctx.edges.push(Edge {
        external: false,
        source: ctx.file_nid.to_string(),
        target: tgt_nid,
        relation: "references".to_string(),
        confidence: "EXTRACTED".to_string(),
        source_file: ctx.str_path.to_string(),
        source_location: Some(format!("L{line_num}")),
        weight: 1.0,
        context: None,
        confidence_score: None,
    });
}

/// Resolve a markdown link target to the normalized absolute path of a sibling
/// document, or `None` to skip it (external URLs, in-page anchors, non-doc
/// targets). Anchor/query suffixes are stripped; extension-less targets (typical
/// of wikilinks) are treated as sibling `.md`. Mirrors `_resolve_markdown_link`.
#[must_use]
fn resolve_markdown_link(raw: &str, source_dir: &Path) -> Option<PathBuf> {
    let target = raw.trim();
    if target.is_empty() {
        return None;
    }
    // Drop anchor/query so `./repo.md#setup` resolves to the same node as `./repo.md`.
    let target = target
        .split('#')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .trim();
    if target.is_empty() {
        return None;
    }
    let low = target.to_lowercase();
    if target.contains("://")
        || low.starts_with("mailto:")
        || low.starts_with("tel:")
        || low.starts_with("//")
        || low.starts_with("data:")
    {
        return None;
    }
    let (target_with_ext, suffix_lc) = match Path::new(target).extension() {
        None => (format!("{target}.md"), ".md".to_string()),
        Some(ext) => (
            target.to_string(),
            format!(".{}", ext.to_string_lossy().to_lowercase()),
        ),
    };
    if !MD_LINKABLE_EXTS.contains(&suffix_lc.as_str()) {
        return None;
    }
    let candidate = Path::new(&target_with_ext);
    let abs = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        source_dir.join(candidate)
    };
    Some(lexical_normalize(&abs))
}

/// Lexically normalize a path (collapse `.`/`..`/redundant separators) without
/// touching the filesystem, matching Python's `os.path.normpath`.
#[must_use]
fn lexical_normalize(p: &Path) -> PathBuf {
    let mut comps: Vec<Component<'_>> = Vec::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match comps.last() {
                Some(Component::Normal(_)) => {
                    comps.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => comps.push(c),
            },
            other => comps.push(other),
        }
    }
    if comps.is_empty() {
        return PathBuf::from(".");
    }
    comps.iter().copied().map(Component::as_os_str).collect()
}
