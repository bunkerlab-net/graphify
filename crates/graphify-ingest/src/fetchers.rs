//! Per-URL-type fetchers (tweet, webpage, arxiv) and binary download
//! helper.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;

use graphify_security::{MAX_FETCH_BYTES, MAX_TEXT_BYTES, safe_fetch, safe_fetch_text};

use crate::error::IngestError;
use crate::regexes::{
    RE_ARXIV_ABSTRACT, RE_ARXIV_AUTHORS, RE_ARXIV_ID, RE_ARXIV_TITLE, RE_TAGS, RE_TITLE,
    RE_WHITESPACE,
};
use crate::text::{html_to_markdown, safe_filename, yaml_str};

/// Default HTTP timeout for all fetch operations.
pub(crate) const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Fetch the raw HTML body of a URL as a UTF-8 string.
///
/// Returns [`IngestError::FetchFailed`] on transport, validation, or
/// size-cap failures.
pub(crate) fn fetch_html(url: &str) -> Result<String, IngestError> {
    safe_fetch_text(url, MAX_TEXT_BYTES, FETCH_TIMEOUT).map_err(|e| IngestError::FetchFailed {
        url: url.to_string(),
        source: e,
    })
}

/// Download a binary resource from `url` and write it to `target_dir`
/// using a slug derived from the URL plus `suffix` as the filename.
pub(crate) fn download_binary(
    url: &str,
    suffix: &str,
    target_dir: &Path,
) -> Result<PathBuf, IngestError> {
    let filename = safe_filename(url, suffix);
    // Apply the same `_N` collision suffix loop used by the text fetchers so
    // binary downloads can't silently overwrite an existing file with the
    // same slug.
    let mut out_path = target_dir.join(&filename);
    let mut counter: u32 = 1;
    while out_path.exists() && counter < 1000 {
        let stem = Path::new(&filename)
            .file_stem()
            .map_or_else(|| filename.clone(), |s| s.to_string_lossy().into_owned());
        out_path = target_dir.join(format!("{stem}_{counter}{suffix}"));
        counter += 1;
    }
    let bytes =
        safe_fetch(url, MAX_FETCH_BYTES, FETCH_TIMEOUT).map_err(|e| IngestError::FetchFailed {
            url: url.to_string(),
            source: e,
        })?;
    std::fs::write(&out_path, bytes)?;
    Ok(out_path)
}

/// Fetch a tweet URL via Twitter's oEmbed endpoint. Returns `(content, filename)`.
///
/// Never propagates errors — oEmbed failures fall back to a stub entry.
pub(crate) fn fetch_tweet(
    url: &str,
    author: Option<&str>,
    contributor: Option<&str>,
) -> (String, String) {
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

/// Fetch a generic webpage and return its rendered markdown content
/// alongside a safe filename.
pub(crate) fn fetch_webpage(
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

/// Fetch an arXiv paper page and return its structured markdown content
/// alongside a safe filename.
///
/// Falls back to [`fetch_webpage`] if no arXiv ID can be extracted from
/// the URL.
pub(crate) fn fetch_arxiv(
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
