//! URL / PDF / Office document ingestion into corpus.
//!
//! Ports `graphify-py/graphify/ingest.py`.

use std::fmt::Write as FmtWrite;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use chrono::Utc;
use regex::Regex;
use thiserror::Error;

use graphify_security::{
    MAX_FETCH_BYTES, MAX_TEXT_BYTES, SecurityError, safe_fetch, safe_fetch_text, validate_url,
};

/// Default HTTP timeout for all fetch operations.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors produced by the ingest module.
#[derive(Debug, Error)]
pub enum IngestError {
    /// URL failed security validation.
    #[error("ingest: {0}")]
    InvalidUrl(String),

    /// Network / HTTP fetch failure.
    #[error("ingest: failed to fetch {url:?}: {source}")]
    FetchFailed {
        url: String,
        #[source]
        source: SecurityError,
    },

    /// Filesystem I/O failure.
    #[error("ingest: I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Filename collision counter exhausted.
    #[error("ingest: could not find a free filename after 999 attempts for {0:?}")]
    FilenameFull(PathBuf),
}

// ---------------------------------------------------------------------------
// Static regexes (known-good patterns — unwrap inside LazyLock is an invariant)
// ---------------------------------------------------------------------------

#[allow(clippy::unwrap_used)]
static RE_SAFE_FILENAME: LazyLock<Regex> = LazyLock::new(|| {
    // Replace anything that isn't a word char or hyphen
    Regex::new(r"[^\w\-]").unwrap()
});

#[allow(clippy::unwrap_used)]
static RE_MULTI_UNDERSCORE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_+").unwrap());

#[allow(clippy::unwrap_used)]
static RE_SCRIPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<script[^>]*>.*?</script>").unwrap());

#[allow(clippy::unwrap_used)]
static RE_STYLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<style[^>]*>.*?</style>").unwrap());

#[allow(clippy::unwrap_used)]
static RE_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?si)<title[^>]*>(.*?)</title>").unwrap());

#[allow(clippy::unwrap_used)]
static RE_WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

#[allow(clippy::unwrap_used)]
static RE_TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

#[allow(clippy::unwrap_used)]
static RE_ARXIV_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{4}\.\d{4,5})").unwrap());

#[allow(clippy::unwrap_used)]
static RE_ARXIV_ABSTRACT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?si)class="abstract[^"]*"[^>]*>(.*?)</blockquote>"#).unwrap());

#[allow(clippy::unwrap_used)]
static RE_ARXIV_TITLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?si)class="title[^"]*"[^>]*>(.*?)</h1>"#).unwrap());

#[allow(clippy::unwrap_used)]
static RE_ARXIV_AUTHORS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?si)class="authors"[^>]*>(.*?)</div>"#).unwrap());

#[allow(clippy::unwrap_used)]
static RE_NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w]").unwrap());

// ---------------------------------------------------------------------------
// _yaml_str
// ---------------------------------------------------------------------------

/// Escape a string for embedding in a YAML double-quoted scalar.
///
/// Handles every YAML 1.1/1.2 line-break and control character that could
/// let a hostile value break out of the quoted scalar and inject sibling
/// YAML keys (F-009 / F-019). Matches the Python reference byte-for-byte.
#[must_use]
pub fn yaml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let cp = ch as u32;
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            _ if cp == 0x2028 => out.push_str("\\L"),
            _ if cp == 0x2029 => out.push_str("\\P"),
            _ if cp < 0x20 || cp == 0x7F => {
                // known-good format string; write! only errors on OOM
                let _ = write!(out, "\\x{cp:02x}");
            }
            _ => out.push(ch),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// _safe_filename
// ---------------------------------------------------------------------------

/// Turn a URL into a safe filename.
#[must_use]
pub fn safe_filename(url: &str, suffix: &str) -> String {
    let name = match url::Url::parse(url) {
        Ok(u) => {
            let netloc = u.host_str().unwrap_or("");
            let path = u.path();
            format!("{netloc}{path}")
        }
        Err(_) => url.to_string(),
    };
    let name = RE_SAFE_FILENAME.replace_all(&name, "_");
    let name = name.trim_matches('_');
    let name = RE_MULTI_UNDERSCORE.replace_all(name, "_");
    let truncated: String = name.chars().take(80).collect();
    format!("{truncated}{suffix}")
}

// ---------------------------------------------------------------------------
// _detect_url_type
// ---------------------------------------------------------------------------

/// Classify a URL for targeted extraction.
#[must_use]
pub fn detect_url_type(url: &str) -> &'static str {
    let lower = url.to_lowercase();
    if lower.contains("twitter.com") || lower.contains("x.com") {
        return "tweet";
    }
    if lower.contains("arxiv.org") {
        return "arxiv";
    }
    if lower.contains("github.com") {
        return "github";
    }
    if lower.contains("youtube.com") || lower.contains("youtu.be") {
        return "youtube";
    }
    // Normalise to lowercase path for extension checks
    let ext = url::Url::parse(url)
        .ok()
        .and_then(|u| {
            Path::new(u.path())
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
        })
        .unwrap_or_default();
    match ext.as_str() {
        "pdf" => return "pdf",
        "png" | "jpg" | "jpeg" | "webp" | "gif" => return "image",
        _ => {}
    }
    "webpage"
}

// ---------------------------------------------------------------------------
// _html_to_markdown
// ---------------------------------------------------------------------------

/// Convert HTML to clean markdown, pre-stripping script/style tags.
#[must_use]
pub fn html_to_markdown(html: &str) -> String {
    // Always strip script/style so their text never leaks into output
    let html = RE_SCRIPT.replace_all(html, "");
    let html = RE_STYLE.replace_all(html.as_ref(), "");
    html2md::parse_html(html.as_ref())
}

// ---------------------------------------------------------------------------
// Internal fetch helpers
// ---------------------------------------------------------------------------

fn fetch_html(url: &str) -> Result<String, IngestError> {
    safe_fetch_text(url, MAX_TEXT_BYTES, FETCH_TIMEOUT).map_err(|e| IngestError::FetchFailed {
        url: url.to_string(),
        source: e,
    })
}

fn download_binary(url: &str, suffix: &str, target_dir: &Path) -> Result<PathBuf, IngestError> {
    let filename = safe_filename(url, suffix);
    let out_path = target_dir.join(&filename);
    let bytes =
        safe_fetch(url, MAX_FETCH_BYTES, FETCH_TIMEOUT).map_err(|e| IngestError::FetchFailed {
            url: url.to_string(),
            source: e,
        })?;
    std::fs::write(&out_path, bytes)?;
    Ok(out_path)
}

// ---------------------------------------------------------------------------
// _fetch_tweet
// ---------------------------------------------------------------------------

/// Fetch a tweet URL. Returns `(content, filename)`.
///
/// Never propagates errors — oEmbed failures fall back to a stub entry.
fn fetch_tweet(url: &str, author: Option<&str>, contributor: Option<&str>) -> (String, String) {
    let oembed_url = url.replace("x.com", "twitter.com");
    let encoded =
        percent_encoding::utf8_percent_encode(&oembed_url, percent_encoding::NON_ALPHANUMERIC);
    let oembed_api = format!("https://publish.twitter.com/oembed?url={encoded}&omit_script=true");

    let (tweet_text, tweet_author) =
        match safe_fetch_text(&oembed_api, MAX_TEXT_BYTES, FETCH_TIMEOUT) {
            Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                Ok(data) => {
                    let html = data.get("html").and_then(|v| v.as_str()).unwrap_or("");
                    let text = RE_TAGS.replace_all(html, "").trim().to_string();
                    let auth = data
                        .get("author_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    (text, auth)
                }
                Err(_) => (
                    format!("Tweet at {url} (could not fetch content)"),
                    "unknown".to_string(),
                ),
            },
            Err(_) => (
                format!("Tweet at {url} (could not fetch content)"),
                "unknown".to_string(),
            ),
        };

    let now = Utc::now().to_rfc3339();
    let contrib = contributor.or(author).unwrap_or("unknown");
    let content = format!(
        "---\nsource_url: \"{}\"\ntype: tweet\nauthor: \"{}\"\ncaptured_at: {}\ncontributor: \"{}\"\n---\n\n# Tweet by @{}\n\n{}\n\nSource: {}\n",
        yaml_str(url),
        yaml_str(&tweet_author),
        now,
        yaml_str(contrib),
        tweet_author,
        tweet_text,
        url,
    );
    let filename = safe_filename(url, ".md");
    (content, filename)
}

// ---------------------------------------------------------------------------
// _fetch_webpage
// ---------------------------------------------------------------------------

fn fetch_webpage(
    url: &str,
    author: Option<&str>,
    contributor: Option<&str>,
) -> Result<(String, String), IngestError> {
    let html = fetch_html(url)?;

    let title = RE_TITLE.captures(&html).and_then(|c| c.get(1)).map_or_else(
        || url.to_string(),
        |m| {
            RE_WHITESPACE
                .replace_all(m.as_str(), " ")
                .trim()
                .to_string()
        },
    );

    let markdown = html_to_markdown(&html);
    let markdown_truncated: String = markdown.chars().take(12000).collect();

    let now = Utc::now().to_rfc3339();
    let contrib = contributor.or(author).unwrap_or("unknown");
    let content = format!(
        "---\nsource_url: \"{}\"\ntype: webpage\ntitle: \"{}\"\ncaptured_at: {}\ncontributor: \"{}\"\n---\n\n# {}\n\nSource: {}\n\n---\n\n{}\n",
        yaml_str(url),
        yaml_str(&title),
        now,
        yaml_str(contrib),
        title,
        url,
        markdown_truncated,
    );
    let filename = safe_filename(url, ".md");
    Ok((content, filename))
}

// ---------------------------------------------------------------------------
// _fetch_arxiv
// ---------------------------------------------------------------------------

fn fetch_arxiv(
    url: &str,
    author: Option<&str>,
    contributor: Option<&str>,
) -> Result<(String, String), IngestError> {
    let Some(cap) = RE_ARXIV_ID.captures(url) else {
        return fetch_webpage(url, author, contributor);
    };
    let arxiv_id = cap.get(1).map_or("", |m| m.as_str()).to_string();

    let api_url = format!("https://export.arxiv.org/abs/{arxiv_id}");
    let (title, abstract_text, paper_authors) = match fetch_html(&api_url) {
        Ok(html) => {
            let abstract_text = RE_ARXIV_ABSTRACT
                .captures(&html)
                .and_then(|c| c.get(1))
                .map_or_else(String::new, |m| {
                    RE_TAGS.replace_all(m.as_str(), "").trim().to_string()
                });
            let title = RE_ARXIV_TITLE
                .captures(&html)
                .and_then(|c| c.get(1))
                .map_or_else(
                    || arxiv_id.clone(),
                    |m| RE_TAGS.replace_all(m.as_str(), " ").trim().to_string(),
                );
            let paper_authors = RE_ARXIV_AUTHORS
                .captures(&html)
                .and_then(|c| c.get(1))
                .map_or_else(String::new, |m| {
                    RE_TAGS.replace_all(m.as_str(), "").trim().to_string()
                });
            (title, abstract_text, paper_authors)
        }
        Err(_) => (arxiv_id.clone(), String::new(), String::new()),
    };

    let now = Utc::now().to_rfc3339();
    let contrib = contributor.or(author).unwrap_or("unknown");
    let content = format!(
        "---\nsource_url: \"{}\"\narxiv_id: \"{}\"\ntype: paper\ntitle: \"{}\"\npaper_authors: \"{}\"\ncaptured_at: {}\ncontributor: \"{}\"\n---\n\n# {}\n\n**Authors:** {}\n**arXiv:** {}\n\n## Abstract\n\n{}\n\nSource: {}\n",
        yaml_str(url),
        yaml_str(&arxiv_id),
        yaml_str(&title),
        yaml_str(&paper_authors),
        now,
        yaml_str(contrib),
        title,
        paper_authors,
        arxiv_id,
        abstract_text,
        url,
    );
    let dot_replaced = arxiv_id.replace('.', "_");
    let filename = format!("arxiv_{dot_replaced}.md");
    Ok((content, filename))
}

// ---------------------------------------------------------------------------
// ingest
// ---------------------------------------------------------------------------

/// Fetch a URL and save it into `target_dir` as a graphify-ready file.
///
/// Returns the path of the saved file.
///
/// # Errors
///
/// Returns [`IngestError`] if URL validation fails, fetch fails, or I/O fails.
pub fn ingest(
    url: &str,
    target_dir: &Path,
    author: Option<&str>,
    contributor: Option<&str>,
) -> Result<PathBuf, IngestError> {
    std::fs::create_dir_all(target_dir)?;
    let url_type = detect_url_type(url);

    validate_url(url).map_err(|e| IngestError::InvalidUrl(e.to_string()))?;

    // Binary / external handler types
    if url_type == "pdf" {
        return download_binary(url, ".pdf", target_dir);
    }

    if url_type == "image" {
        let suffix = url::Url::parse(url)
            .ok()
            .and_then(|u| {
                Path::new(u.path())
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
            })
            .unwrap_or_else(|| ".jpg".to_string());
        return download_binary(url, &suffix, target_dir);
    }

    // YouTube: stub — requires graphify-transcribe
    if url_type == "youtube" {
        return Err(IngestError::FetchFailed {
            url: url.to_string(),
            source: SecurityError::Transport(
                "youtube ingestion requires graphify-transcribe".to_string(),
            ),
        });
    }

    let (content, filename) = if url_type == "tweet" {
        fetch_tweet(url, author, contributor)
    } else if url_type == "arxiv" {
        fetch_arxiv(url, author, contributor)?
    } else {
        fetch_webpage(url, author, contributor)?
    };

    let mut out_path = target_dir.join(&filename);
    // Avoid overwriting — append counter if needed
    let mut counter: u32 = 1;
    while out_path.exists() && counter < 1000 {
        let stem = Path::new(&filename)
            .file_stem()
            .map_or_else(|| filename.clone(), |s| s.to_string_lossy().into_owned());
        out_path = target_dir.join(format!("{stem}_{counter}.md"));
        counter += 1;
    }
    if counter >= 1000 && out_path.exists() {
        return Err(IngestError::FilenameFull(out_path));
    }

    std::fs::write(&out_path, content.as_bytes())?;
    Ok(out_path)
}

// ---------------------------------------------------------------------------
// save_query_result
// ---------------------------------------------------------------------------

/// Save a Q&A result as markdown so it gets extracted into the graph on next
/// `--update`.
///
/// Files are stored in `memory_dir` (typically `graphify-out/memory/`) with
/// YAML frontmatter that graphify's extractor reads as node metadata.
///
/// # Errors
///
/// Returns [`IngestError`] if directory creation or file write fails.
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
