//! Persistence of Q&A results into `graphify-out/memory/` so they get
//! extracted into the graph on the next `--update`.

use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::error::IngestError;
use crate::regexes::RE_NON_WORD;
use crate::text::yaml_str;

/// Save a Q&A result as Markdown.
///
/// Files are stored in `memory_dir` (typically `graphify-out/memory/`)
/// with YAML frontmatter that graphify's extractor reads as node metadata.
/// The filename includes a timestamp and a slugified prefix of the
/// question for human readability.
///
/// `source_nodes` is optional; if provided, the first 10 are embedded into
/// the frontmatter as a YAML list and into the body as a bullet list.
///
/// # Errors
///
/// Returns [`IngestError::Io`] if the memory directory cannot be created
/// or the file cannot be written.
pub fn save_query_result(
    question: &str,
    answer: &str,
    memory_dir: &Path,
    query_type: &str,
    source_nodes: Option<&[String]>,
) -> Result<PathBuf, IngestError> {
    std::fs::create_dir_all(memory_dir)?;

    let now = Utc::now();
    let slug: String = {
        let lower = question.to_lowercase();
        let replaced = RE_NON_WORD.replace_all(&lower, "_");
        let trimmed: String = replaced.trim_matches('_').chars().take(50).collect();
        trimmed.trim_matches('_').to_string()
    };
    let filename = format!("query_{}_{slug}.md", now.format("%Y%m%d_%H%M%S"));

    let mut frontmatter_lines: Vec<String> = vec![
        "---".to_string(),
        format!("type: \"{query_type}\""),
        format!("date: \"{}\"", now.to_rfc3339()),
        format!("question: \"{}\"", yaml_str(question)),
        "contributor: \"graphify\"".to_string(),
    ];

    if let Some(nodes) = source_nodes {
        let nodes_str = nodes
            .iter()
            .take(10)
            .map(|n| format!("\"{n}\""))
            .collect::<Vec<_>>()
            .join(", ");
        frontmatter_lines.push(format!("source_nodes: [{nodes_str}]"));
    }

    frontmatter_lines.push("---".to_string());

    let mut body_lines: Vec<String> = vec![
        String::new(),
        format!("# Q: {question}"),
        String::new(),
        "## Answer".to_string(),
        String::new(),
        answer.to_string(),
    ];

    if let Some(nodes) = source_nodes {
        body_lines.push(String::new());
        body_lines.push("## Source Nodes".to_string());
        body_lines.push(String::new());
        for n in nodes {
            body_lines.push(format!("- {n}"));
        }
    }

    let all_lines: Vec<String> = frontmatter_lines.into_iter().chain(body_lines).collect();
    let content = all_lines.join("\n");

    let out_path = memory_dir.join(&filename);
    std::fs::write(&out_path, content.as_bytes())?;
    Ok(out_path)
}
