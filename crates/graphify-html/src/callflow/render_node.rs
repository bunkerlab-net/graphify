//! Per-node HTML helpers: tag badges, node descriptions, and node-reference
//! formatting used in the call-detail table.
//!
//! Extracted from `render` so the label-to-HTML dispatch logic is readable in
//! isolation without scrolling past the larger page-section generators.

use std::collections::HashMap;

use super::loader::{pick_text, safe_file_path};
use super::options::Node;

// ── Node display helpers ────────────────────────────────────────────────────

/// Return a human-readable display name for a node, or `fallback` when absent.
pub(super) fn node_display_name(node: Option<&Node>, fallback: &str) -> String {
    match node {
        None => fallback.to_owned(),
        Some(n) => {
            let label = if n.label.is_empty() {
                fallback.to_owned()
            } else {
                n.label.clone()
            };
            super::builder::humanize_label(&label, &n.source_file)
        }
    }
}

/// Render a sorted list of node references as HTML `<code>` tags with optional file paths.
///
/// Truncates to `limit` items and appends a "+N more" label when the list is longer.
pub(super) fn format_node_refs(
    node_ids: &[&str],
    nodes: &[Node],
    lang: &str,
    empty_text: &str,
    limit: usize,
) -> String {
    if node_ids.is_empty() {
        return htmlescape::encode_minimal(empty_text);
    }
    let node_by_id: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut sorted: Vec<&str> = node_ids.to_vec();
    sorted.sort_by_key(|&nid| node_display_name(node_by_id.get(nid).copied(), nid).to_lowercase());
    let mut parts: Vec<String> = sorted
        .iter()
        .take(limit)
        .map(|&nid| {
            let node = node_by_id.get(nid).copied();
            let label = node_display_name(node, nid);
            let source = node
                .map(|n| safe_file_path(&n.source_file))
                .unwrap_or_default();
            if source.is_empty() {
                format!("<code>{}</code>", htmlescape::encode_minimal(&label))
            } else {
                format!(
                    "<code>{}</code><br><small style=\"color:var(--muted)\">{}</small>",
                    htmlescape::encode_minimal(&label),
                    htmlescape::encode_minimal(&source)
                )
            }
        })
        .collect();
    if node_ids.len() > limit {
        let more = node_ids.len() - limit;
        parts.push(htmlescape::encode_minimal(pick_text(
            lang,
            &format!("+{more} 个更多"),
            &format!("+{more} more"),
        )));
    }
    parts.join("<br>")
}

/// Produce an HTML `<span class="tag ...">` badge for a node based on its kind.
///
/// Falls back to label-based heuristics when the primary `kind` dispatch doesn't match.
pub(super) fn suggest_tag(label: &str, file_type: &str, lang: &str, kind: &str) -> String {
    let names: &[(&str, &str, &str, &str)] = &[
        ("concept", "概念", "Concept", "tag-func"),
        ("entry", "入口", "Entry", "tag-cmd"),
        ("api", "API", "API", "tag-endpoint"),
        ("async", "异步", "Async", "tag-async"),
        ("klass", "类", "Class", "tag-class"),
        ("ui", "UI", "UI", "tag-hook"),
        ("module", "模块", "Module", "tag-class"),
        ("test", "测试", "Test", "tag-func"),
        ("function", "函数", "Function", "tag-func"),
    ];
    for &(k, zh, en, cls) in names {
        if kind == k {
            let text = pick_text(lang, zh, en);
            return format!("<span class=\"tag {cls}\">{text}</span>");
        }
    }
    if file_type == "rationale" {
        return format!(
            "<span class=\"tag tag-func\">{}</span>",
            pick_text(lang, "概念", "Concept")
        );
    }
    let lower = label.to_lowercase();
    if lower.contains("router") || lower.contains("endpoint") || lower.contains("/api/") {
        return format!(
            "<span class=\"tag tag-endpoint\">{}</span>",
            pick_text(lang, "API端点", "API")
        );
    }
    if lower.contains("async") || lower.contains("await") || lower.contains("stream") {
        return format!(
            "<span class=\"tag tag-async\">{}</span>",
            pick_text(lang, "异步", "Async")
        );
    }
    if lower.contains("class") || lower.contains("model") || lower.contains("schema") {
        return format!(
            "<span class=\"tag tag-class\">{}</span>",
            pick_text(lang, "类", "Class")
        );
    }
    if lower.contains("hook") || lower.contains("usestate") || lower.contains("useeffect") {
        return "<span class=\"tag tag-hook\">Hook</span>".to_owned();
    }
    if lower.contains("component") || lower.contains("props") {
        return format!(
            "<span class=\"tag tag-class\">{}</span>",
            pick_text(lang, "组件", "Component")
        );
    }
    format!(
        "<span class=\"tag tag-func\">{}</span>",
        pick_text(lang, "函数", "Function")
    )
}

#[allow(clippy::too_many_lines)] // Long dispatch function; splitting into sub-functions would obscure the linear logic.
/// Generate a one-sentence plain-language description of a node.
///
/// Uses a cascade of label-keyword heuristics to produce a human-readable
/// sentence for the node detail panel. Mirrors Python's `_describe_node` in
/// `callflow.py`.
pub(super) fn describe_node(label: &str, source_file: &str, file_type: &str, lang: &str) -> String {
    let lower = label.to_lowercase();
    let source = if source_file.is_empty() {
        pick_text(lang, "项目", "project")
    } else {
        source_file
    };
    if file_type == "rationale" {
        return pick_text(
            lang,
            &format!("设计说明：{label}"),
            &format!("Design note for {label}."),
        )
        .to_owned();
    }
    if file_type == "document" {
        return pick_text(
            lang,
            &format!("文档入口，描述 {label} 相关能力。"),
            &format!("Documentation node describing {label}."),
        )
        .to_owned();
    }
    if matches!(
        std::path::Path::new(label)
            .extension()
            .and_then(|e| e.to_str()),
        Some("py" | "tsx" | "ts")
    ) {
        return pick_text(
            lang,
            &format!("{source} 中的模块文件，承载该层主要实现。"),
            &format!("Module file in {source}."),
        )
        .to_owned();
    }
    if lower.contains("config") {
        return pick_text(
            lang,
            "读取、解析或持久化项目配置。",
            "Reads, resolves, or persists project configuration.",
        )
        .to_owned();
    }
    if lower.contains("scan") {
        return pick_text(
            lang,
            "触发项目扫描或处理扫描状态。",
            "Starts scanning or handles scan status.",
        )
        .to_owned();
    }
    if lower.contains("ingest") || lower.contains("clone") || lower.contains("git") {
        return pick_text(
            lang,
            "把本地目录或远程仓库转换为分析上下文。",
            "Turns a local path or remote repository into analysis context.",
        )
        .to_owned();
    }
    if lower.contains("prompt") {
        return pick_text(
            lang,
            "构造发送给 LLM 的结构化提示。",
            "Builds structured prompts for model calls.",
        )
        .to_owned();
    }
    if lower.contains("analy") {
        return pick_text(
            lang,
            "编排分析流程并产出结构化文档数据。",
            "Orchestrates analysis and returns structured documentation data.",
        )
        .to_owned();
    }
    if lower.contains("graph") || lower.contains("dependency") {
        return pick_text(
            lang,
            "构建依赖关系并提供排序或图形化数据。",
            "Builds dependency relationships and graph data.",
        )
        .to_owned();
    }
    if lower.contains("export") || lower.contains("markdown") || lower.contains("html") {
        return pick_text(
            lang,
            "将文档数据导出为目标格式。",
            "Exports documentation data to a target format.",
        )
        .to_owned();
    }
    if lower.contains("chat") || lower.contains("rag") || lower.contains("retrieve") {
        return pick_text(
            lang,
            "支撑检索增强问答或流式聊天。",
            "Supports retrieval-augmented Q&A or streaming chat.",
        )
        .to_owned();
    }
    if lower.contains("wiki") || lower.contains("page") || lower.contains("sidebar") {
        return pick_text(
            lang,
            "组织文档页面、侧边栏或内容读取。",
            "Organizes documentation pages, navigation, or content lookup.",
        )
        .to_owned();
    }
    if lower.contains("cache") || lower.contains("hash") {
        return pick_text(
            lang,
            "缓存分析结果或生成缓存键。",
            "Caches analysis results or computes cache keys.",
        )
        .to_owned();
    }
    if lower.contains("test") {
        return pick_text(
            lang,
            "验证导入、入口点或版本等基础行为。",
            "Verifies imports, entry points, or version behavior.",
        )
        .to_owned();
    }
    pick_text(
        lang,
        &format!("{source} 中的 {label} 节点。"),
        &format!("{label} node in {source}."),
    )
    .to_owned()
}
