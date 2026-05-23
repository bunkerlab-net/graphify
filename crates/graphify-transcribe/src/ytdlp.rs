//! `yt-dlp` subprocess wrapper, behind a [`YtDlpRunner`] trait so tests
//! can inject a mock without spawning processes.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::TranscribeError;

/// Abstraction over the `yt-dlp` invocation.
pub trait YtDlpRunner {
    /// Download audio from `url` into `output_dir` using `out_template`
    /// as the yt-dlp output template.
    ///
    /// Returns the path to the downloaded file.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::BinaryMissing`] when yt-dlp is absent,
    /// [`TranscribeError::BinaryFailed`] on non-zero exit, or
    /// [`TranscribeError::OutputMissing`] if no matching file is found
    /// among the expected extensions after a successful download.
    fn download(
        &self,
        url: &str,
        out_template: &str,
        output_dir: &Path,
        hash_prefix: &str,
    ) -> Result<PathBuf, TranscribeError>;
}

/// Shells out to `yt-dlp` on `PATH`.
pub struct YtDlpCliRunner;

impl YtDlpRunner for YtDlpCliRunner {
    /// Invoke `yt-dlp` to download audio from `url`, then locate and
    /// return the downloaded file.
    fn download(
        &self,
        url: &str,
        out_template: &str,
        output_dir: &Path,
        hash_prefix: &str,
    ) -> Result<PathBuf, TranscribeError> {
        let output = Command::new("yt-dlp")
            .arg("--format")
            .arg("bestaudio[ext=m4a]/bestaudio/best")
            .arg("--output")
            .arg(out_template)
            .arg("--quiet")
            .arg("--no-warnings")
            .arg("--no-playlist")
            .arg(url)
            .output()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    TranscribeError::BinaryMissing {
                        binary: "yt-dlp".to_string(),
                    }
                } else {
                    TranscribeError::Io(e)
                }
            })?;

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(TranscribeError::BinaryFailed {
                binary: "yt-dlp".to_string(),
                code,
                stderr,
            });
        }

        for ext in &["m4a", "opus", "mp3", "ogg", "wav", "webm"] {
            let candidate = output_dir.join(format!("yt_{hash_prefix}.{ext}"));
            if candidate.exists() {
                return Ok(candidate);
            }
        }

        Err(TranscribeError::OutputMissing {
            path: output_dir.join(format!("yt_{hash_prefix}.*")),
        })
    }
}
