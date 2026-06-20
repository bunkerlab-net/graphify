//! Tree-view HTML generator.
//!
//! Ports `graphify-py/graphify/tree_html.py`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use graphify_build::Graph;
use serde_json::Value;

use crate::HtmlError;

/// Default cap on children rendered under a single node.
pub const DEFAULT_MAX_CHILDREN: usize = 200;

/// Per-file grouping entry: `(node id, optional label, optional file_type)`.
type NodeGroupEntry = (String, Option<String>, Option<String>);

// ── embedded template ────────────────────────────────────────────────────────

static HTML_TEMPLATE: &str = include_str!("tree_template.html");

// ── arena-based tree builder ─────────────────────────────────────────────────

struct ArenaTree {
    /// Node names.
    names: Vec<String>,
    /// `total_count` for each node.
    counts: Vec<usize>,
    /// Child indices for each node.
    children: Vec<Vec<usize>>,
}

impl ArenaTree {
    /// Create a new empty arena tree with no nodes.
    fn new() -> Self {
        Self {
            names: Vec::new(),
            counts: Vec::new(),
            children: Vec::new(),
        }
    }

    /// Allocate a new node with the given name and return its index.
    fn add_node(&mut self, name: impl Into<String>) -> usize {
        let idx = self.names.len();
        self.names.push(name.into());
        self.counts.push(0);
        self.children.push(Vec::new());
        idx
    }

    /// Register `child` as a child of `parent` in the arena.
    fn add_child(&mut self, parent: usize, child: usize) {
        self.children[parent].push(child);
    }

    /// Produce the final owned tree (consuming self).
    fn into_tree(self, root: usize) -> TreeNode {
        build_from_arena(&self, root)
    }
}

/// Recursively convert the arena node at `idx` into an owned `TreeNode`.
fn build_from_arena(tree: &ArenaTree, idx: usize) -> TreeNode {
    let kids: Vec<TreeNode> = tree.children[idx]
        .iter()
        .copied()
        .map(|ci| build_from_arena(tree, ci))
        .collect();
    TreeNode {
        name: tree.names[idx].clone(),
        total_count: tree.counts[idx],
        children: kids,
    }
}

// ── tree node ─────────────────────────────────────────────────────────────────

struct TreeNode {
    name: String,
    total_count: usize,
    children: Vec<TreeNode>,
}

impl TreeNode {
    /// Serialize this node and all its descendants to the `{name, total_count, children}` shape.
    fn to_json(&self) -> Value {
        serde_json::json!({
            "name": self.name,
            "total_count": self.total_count,
            "children": self.children.iter().map(TreeNode::to_json).collect::<Vec<_>>(),
        })
    }
}

// ── common-root helper ───────────────────────────────────────────────────────

/// Compute the longest common path prefix shared by all `paths`.
///
/// Used to strip the project root from node `source_file` values so relative
/// paths appear in the tree. Returns an empty string if paths is empty or has
/// no common prefix.
fn common_root(paths: &[String]) -> String {
    let non_empty: Vec<Vec<String>> = paths
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| {
            Path::new(p.as_str())
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        })
        .collect();

    if non_empty.is_empty() {
        return String::new();
    }

    let mut common: Vec<String> = non_empty[0].clone();
    for parts in non_empty.iter().skip(1) {
        let mut i = 0;
        while i < common.len() && i < parts.len() && common[i] == parts[i] {
            i += 1;
        }
        common.truncate(i);
    }

    if common.is_empty() {
        return String::new();
    }

    // Reconstruct path from components.
    let mut result = common[0].clone();
    for part in &common[1..] {
        result.push('/');
        result.push_str(part);
    }
    result
}

// ── dir-node ensurer ─────────────────────────────────────────────────────────

/// Ensure a directory node exists in `tree`, creating parent nodes recursively.
/// Returns the index of the node for `abs_path`.
fn ensure_dir(
    abs_path: &Path,
    root_idx: usize,
    tree: &mut ArenaTree,
    dir_map: &mut HashMap<String, usize>,
) -> usize {
    let key = abs_path.to_string_lossy().into_owned();
    if let Some(&idx) = dir_map.get(&key) {
        return idx;
    }

    // If at filesystem root or at/above our root, return root.
    let parent_path = match abs_path.parent() {
        Some(p) if p != abs_path => p,
        _ => return root_idx,
    };

    let parent_idx = ensure_dir(parent_path, root_idx, tree, dir_map);
    let name = abs_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let idx = tree.add_node(name);
    tree.add_child(parent_idx, idx);
    dir_map.insert(key, idx);
    idx
}

// ── sort and propagate ────────────────────────────────────────────────────────

/// Sort children (dirs before files, then alphabetically) and propagate
/// `total_count` from leaves to root. Returns the total count at this node.
///
/// Directories sort before files so the tree renders with directory nodes
/// at the top, matching Python's `sorted(…, key=lambda n: (…))` in `tree_html.py`.
fn finalise(node: &mut TreeNode) -> usize {
    if node.children.is_empty() {
        let tc = if node.total_count == 0 {
            1
        } else {
            node.total_count
        };
        node.total_count = tc;
        return tc;
    }

    // Dirs (has children) sort before files; then alphabetically.
    node.children.sort_by(|a, b| {
        let a_dir = !a.children.is_empty();
        let b_dir = !b.children.is_empty();
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    let mut total = 0usize;
    for child in &mut node.children {
        total += finalise(child);
    }
    node.total_count = if total == 0 { 1 } else { total };
    node.total_count
}

// ── public tree builder ───────────────────────────────────────────────────────

/// Build a `{name, total_count, children}` JSON hierarchy from a [`Graph`].
///
/// Mirrors Python `build_tree(graph, *, root, max_children, project_label)`.
#[must_use]
pub fn build_tree(
    graph: &Graph,
    root: Option<&Path>,
    max_children: usize,
    project_label: Option<&str>,
) -> Value {
    let file_nodes = collect_file_nodes(graph);
    if file_nodes.is_empty() {
        return serde_json::json!({
            "name": "(empty graph)",
            "total_count": 0,
            "children": [],
        });
    }

    let source_files: Vec<String> = file_nodes.iter().map(|(_, sf, _, _)| sf.clone()).collect();
    let root_str = root.map_or_else(
        || common_root(&source_files),
        |p| p.to_string_lossy().into_owned(),
    );
    let root_path = PathBuf::from(&root_str);
    let label_root = resolve_root_label(project_label, &root_path, &root_str);

    let by_file = group_file_nodes_by_source(&file_nodes);
    let mut tree = ArenaTree::new();
    let root_idx = tree.add_node(label_root);
    let mut dir_map: HashMap<String, usize> = HashMap::new();
    dir_map.insert(root_path.to_string_lossy().into_owned(), root_idx);

    let mut sorted_files: Vec<String> = by_file.keys().cloned().collect();
    sorted_files.sort();
    let mut build_ctx = TreeBuildCtx {
        tree: &mut tree,
        dir_map: &mut dir_map,
    };
    for src_file in &sorted_files {
        add_file_subtree(
            &mut build_ctx,
            src_file,
            &by_file[src_file],
            &root_path,
            root_idx,
            max_children,
        );
    }

    let mut root_node = tree.into_tree(root_idx);
    finalise(&mut root_node);
    root_node.to_json()
}

/// Collect `(id, source_file, label, file_type)` for nodes that have a non-empty
/// `source_file` attribute.
fn collect_file_nodes(graph: &Graph) -> Vec<(String, String, Option<String>, Option<String>)> {
    graph
        .nodes()
        .filter_map(|(id, attrs)| {
            let sf = attrs.get("source_file")?.as_str()?;
            if sf.is_empty() {
                return None;
            }
            let label = attrs
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let file_type = attrs
                .get("file_type")
                .and_then(Value::as_str)
                .map(str::to_owned);
            Some((id.clone(), sf.to_owned(), label, file_type))
        })
        .collect()
}

/// Resolve the label used for the root node — explicit `project_label` wins,
/// otherwise the final path component of `root_path` or `root_str`.
fn resolve_root_label(project_label: Option<&str>, root_path: &Path, root_str: &str) -> String {
    project_label
        .map(str::to_owned)
        .or_else(|| {
            root_path.file_name().and_then(|n| {
                let s = n.to_string_lossy().into_owned();
                if s.is_empty() { None } else { Some(s) }
            })
        })
        .or_else(|| {
            if root_str.is_empty() {
                None
            } else {
                Some(root_str.to_owned())
            }
        })
        .unwrap_or_else(|| "/".to_owned())
}

/// Bucket file-nodes by their source file path.
fn group_file_nodes_by_source(
    file_nodes: &[(String, String, Option<String>, Option<String>)],
) -> HashMap<String, Vec<NodeGroupEntry>> {
    let mut by_file: HashMap<String, Vec<NodeGroupEntry>> = HashMap::new();
    for (id, sf, label, file_type) in file_nodes {
        by_file
            .entry(sf.clone())
            .or_default()
            .push((id.clone(), label.clone(), file_type.clone()));
    }
    by_file
}

/// Mutable arena state passed through the tree builder.
struct TreeBuildCtx<'a> {
    tree: &'a mut ArenaTree,
    dir_map: &'a mut HashMap<String, usize>,
}

/// Add a single file (and its symbol children) to the arena tree.
fn add_file_subtree(
    ctx: &mut TreeBuildCtx<'_>,
    src_file: &str,
    syms: &[NodeGroupEntry],
    root_path: &Path,
    root_idx: usize,
    max_children: usize,
) {
    let TreeBuildCtx { tree, dir_map } = ctx;
    let src_path = Path::new(src_file);
    let src_name = src_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let parent_path: PathBuf = if let Ok(rel) = src_path.strip_prefix(root_path) {
        root_path
            .join(rel)
            .parent()
            .map_or_else(|| root_path.to_path_buf(), Path::to_path_buf)
    } else {
        root_path.to_path_buf()
    };
    let parent_idx = ensure_dir(&parent_path, root_idx, tree, dir_map);

    let sym_list = sorted_symbol_list(syms, &src_name);
    let sym_count = sym_list.len();
    let (used, extra) = if sym_count > max_children {
        (&sym_list[..max_children], sym_count - max_children)
    } else {
        (&sym_list[..], 0)
    };

    let file_idx = tree.add_node(src_name);
    for (name, _) in used {
        let sym_idx = tree.add_node(name.clone());
        tree.counts[sym_idx] = 1;
        tree.add_child(file_idx, sym_idx);
    }
    if extra > 0 {
        let trunc_idx = tree.add_node(format!("(+{extra} more)"));
        tree.counts[trunc_idx] = extra;
        tree.add_child(file_idx, trunc_idx);
    }
    tree.counts[file_idx] = if sym_count == 0 { 1 } else { sym_count };
    tree.add_child(parent_idx, file_idx);
}

/// Build a sorted `(name, is_underscore)` list for a single file's symbols.
/// Non-underscore names sort first, then alphabetically (case-insensitive).
fn sorted_symbol_list(syms: &[NodeGroupEntry], src_name: &str) -> Vec<(String, bool)> {
    let mut sym_list: Vec<(String, bool)> = syms
        .iter()
        .filter_map(|(id, label, file_type)| {
            let lbl = label.as_deref().unwrap_or(id.as_str());
            // Skip redundant file-name node emitted by graphify.
            if lbl == src_name && file_type.as_deref() == Some("code") {
                return None;
            }
            Some((lbl.to_owned(), lbl.starts_with('_')))
        })
        .collect();
    sym_list.sort_by(|(a_name, a_under), (b_name, b_under)| {
        a_under
            .cmp(b_under)
            .then_with(|| a_name.to_lowercase().cmp(&b_name.to_lowercase()))
    });
    sym_list
}

// ── HTML emitter ─────────────────────────────────────────────────────────────

/// Emit a self-contained tree-viewer HTML string.
///
/// Mirrors Python `emit_html(tree, *, title, header, svg_width, svg_height)`.
#[must_use]
pub fn emit_tree_html(
    tree: &Value,
    title: &str,
    header: &str,
    svg_width: u32,
    svg_height: u32,
) -> String {
    // Compact JSON, neutralise `</script>` so embedded JSON cannot break out,
    // and escape U+2028/U+2029 — valid inside JSON strings but JavaScript line
    // terminators that would otherwise break out of the `<script>` block and
    // enable XSS (matches Python `tree_html.emit_html`'s `ensure_ascii=True`).
    let data_json = serde_json::to_string(tree)
        // infallible for a well-formed serde_json::Value
        .unwrap_or_else(|_| "{}".to_owned())
        .replace("</", "<\\/")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");

    HTML_TEMPLATE
        .replace("{title}", &htmlescape::encode_minimal(title))
        .replace("{header}", &htmlescape::encode_minimal(header))
        .replace("{svg_width}", &svg_width.to_string())
        .replace("{svg_height}", &svg_height.to_string())
        .replace("{data_json}", &data_json)
}

// ── public API ────────────────────────────────────────────────────────────────

/// Render a D3 collapsible-tree HTML page for `graph`.
///
/// `root` sets the project root used to strip common path prefixes from node
/// source files.
#[must_use]
pub fn render_tree_html(graph: &Graph, root: &Path) -> String {
    let tree = build_tree(graph, Some(root), DEFAULT_MAX_CHILDREN, None);
    let title_name = tree.get("name").and_then(Value::as_str).unwrap_or("graph"); // safe: Value::String always returns Some from as_str
    let title = format!("{title_name} \u{2014} graphify tree viewer");
    let header = format!("{title_name} \u{2014} Knowledge Graph");
    emit_tree_html(&tree, &title, &header, 6000, 8000)
}

/// Write a D3 collapsible-tree HTML page to `path`.
///
/// # Errors
///
/// Returns [`HtmlError::Io`] if the output directory cannot be created or the
/// file cannot be written.
pub fn write_tree_html(graph: &Graph, root: &Path, path: &Path) -> Result<(), HtmlError> {
    let html = render_tree_html(graph, root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, html.as_bytes())?;
    Ok(())
}

// ── unit tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tree_tests;
