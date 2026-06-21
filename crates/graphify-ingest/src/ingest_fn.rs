//! Top-level [`ingest`] driver: validate, fetch by URL type, write to disk
//! with collision avoidance.

use std::path::{Path, PathBuf};

use graphify_security::validate_url;
use graphify_transcribe::{YtDlpCliRunner, YtDlpRunner, download_audio_with};

use crate::error::IngestError;
use crate::fetchers::{download_binary, fetch_arxiv, fetch_tweet, fetch_webpage};
use crate::text::detect_url_type;

/// Fetch a URL and save it into `target_dir` as a graphify-ready file.
///
/// Returns the path of the saved file.
///
/// Dispatch by URL type:
/// - `pdf` / `image` → download the binary directly.
/// - `youtube` → download audio via `yt-dlp` (delegates to
///   `graphify-transcribe`).
/// - `tweet` → fetch via Twitter's oEmbed API and render as Markdown.
/// - `arxiv` → fetch the abstract page and render as a paper Markdown.
/// - `webpage` (default) → render HTML to Markdown.
///
/// Filenames are derived from the URL; if a collision occurs, a counter
/// suffix is appended up to `_999`.
///
/// # Errors
///
/// Returns [`IngestError`] on URL validation failure, fetch failure, or
/// filesystem I/O failure.
pub fn ingest(
    url: &str,
    target_dir: &Path,
    author: Option<&str>,
    contributor: Option<&str>,
) -> Result<PathBuf, IngestError> {
    ingest_with(url, target_dir, author, contributor, &YtDlpCliRunner)
}

/// Like [`ingest`] but accepts an injected [`YtDlpRunner`] so tests can stub
/// `YouTube` audio downloads without spawning `yt-dlp`.
///
/// # Errors
///
/// Returns [`IngestError`] on URL validation failure, fetch failure,
/// transcription failure, or filesystem I/O failure.
pub fn ingest_with(
    url: &str,
    target_dir: &Path,
    author: Option<&str>,
    contributor: Option<&str>,
    yt_runner: &dyn YtDlpRunner,
) -> Result<PathBuf, IngestError> {
    std::fs::create_dir_all(target_dir)?;
    let url_type = detect_url_type(url);

    validate_url(url).map_err(|e| IngestError::InvalidUrl(e.to_string()))?;

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

    if url_type == "youtube" {
        return download_audio_with(url, target_dir, yt_runner).map_err(|source| {
            IngestError::Transcribe {
                url: url.to_string(),
                source,
            }
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
