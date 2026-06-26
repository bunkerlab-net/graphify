//! Persistence of Q&A results into `graphify-out/memory/` so they get
//! extracted into the graph on the next `--update`.

use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::error::IngestError;
use crate::regexes::RE_NON_WORD;
use crate::text::yaml_str;

/// Work-memory outcome signals accepted by [`save_query_result`] (#1441).
pub const OUTCOMES: [&str; 3] = ["useful", "dead_end", "corrected"];

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
/// `outcome` (one of [`OUTCOMES`]) and `correction` are optional work-memory
/// signals: when set, they are written to both the frontmatter and an
/// `## Outcome` body section so `graphify reflect` can aggregate them.
///
/// # Errors
///
/// Returns [`IngestError::InvalidOutcome`] if `outcome` is set to a value
/// outside [`OUTCOMES`], or [`IngestError::Io`] if the memory directory cannot
/// be created or the file cannot be written.
pub fn save_query_result(
    question: &str,
    answer: &str,
    memory_dir: &Path,
    query_type: &str,
    source_nodes: Option<&[String]>,
    outcome: Option<&str>,
    correction: Option<&str>,
) -> Result<PathBuf, IngestError> {
    if let Some(o) = outcome
        && !OUTCOMES.contains(&o)
    {
        return Err(IngestError::InvalidOutcome { got: o.to_string() });
    }
    std::fs::create_dir_all(memory_dir)?;

    let now = Utc::now();
    let slug: String = {
        let lower = question.to_lowercase();
        let replaced = RE_NON_WORD.replace_all(&lower, "_");
        let trimmed: String = replaced.trim_matches('_').chars().take(50).collect();
        let final_slug = trimmed.trim_matches('_');
        if final_slug.is_empty() {
            "query".to_string()
        } else {
            final_slug.to_string()
        }
    };
    let filename = format!("query_{}_{slug}.md", now.format("%Y%m%d_%H%M%S"));

    let mut frontmatter_lines: Vec<String> = vec![
        "---".to_string(),
        format!("type: \"{}\"", yaml_str(query_type)),
        format!("date: \"{}\"", now.to_rfc3339()),
        format!("question: \"{}\"", yaml_str(question)),
        "contributor: \"graphify\"".to_string(),
    ];
    if let Some(o) = outcome {
        frontmatter_lines.push(format!("outcome: \"{}\"", yaml_str(o)));
    }
    if let Some(c) = correction.filter(|c| !c.is_empty()) {
        frontmatter_lines.push(format!("correction: \"{}\"", yaml_str(c)));
    }

    if let Some(nodes) = source_nodes {
        let nodes_str = nodes
            .iter()
            .take(10)
            .map(|n| format!("\"{}\"", yaml_str(n)))
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

    if outcome.is_some() || correction.is_some_and(|c| !c.is_empty()) {
        body_lines.push(String::new());
        body_lines.push("## Outcome".to_string());
        body_lines.push(String::new());
        if let Some(o) = outcome {
            body_lines.push(format!("- Signal: {o}"));
        }
        if let Some(c) = correction.filter(|c| !c.is_empty()) {
            body_lines.push(format!("- Correction: {c}"));
        }
    }
    if let Some(nodes) = source_nodes {
        body_lines.push(String::new());
        body_lines.push("## Source Nodes".to_string());
        body_lines.push(String::new());
        // Mirror the cap applied to the frontmatter list so the rendered
        // body never drifts above the documented 10-node limit.
        for n in nodes.iter().take(10) {
            body_lines.push(format!("- {n}"));
        }
    }

    let all_lines: Vec<String> = frontmatter_lines.into_iter().chain(body_lines).collect();
    let content = all_lines.join("\n");

    let out_path = memory_dir.join(&filename);
    std::fs::write(&out_path, content.as_bytes())?;
    Ok(out_path)
}
