//! `whisper-cli` subprocess wrapper, behind a [`WhisperRunner`] trait so
//! tests can inject a mock without spawning processes.

use std::path::Path;
use std::process::Command;

use crate::error::TranscribeError;

/// Abstraction over the `whisper-cli` invocation.
///
/// The default implementation ([`WhisperCliRunner`]) shells out to the
/// real binary. Tests may inject a stub instead.
pub trait WhisperRunner {
    /// Run transcription on `audio`, returning the transcript text.
    ///
    /// # Errors
    ///
    /// Returns [`TranscribeError::BinaryMissing`] when the binary is
    /// absent, [`TranscribeError::BinaryFailed`] on non-zero exit, or
    /// [`TranscribeError::OutputMissing`] when whisper did not write the
    /// expected `.txt` file.
    fn run(&self, audio: &Path, prompt: &str, model: &str) -> Result<String, TranscribeError>;
}

/// Shells out to `whisper-cli` on `PATH`.
///
/// Invocation:
/// ```text
/// whisper-cli --model <model> --output-txt --file <audio>
///             [--prompt <prompt>]
/// ```
/// The transcript is written to `<audio_stem>.txt` beside the audio file;
/// this runner reads that file and returns its contents.
pub struct WhisperCliRunner;

impl WhisperRunner for WhisperCliRunner {
    /// Invoke `whisper-cli` on `audio` and return the transcript text
    /// from the generated `.txt` file.
    fn run(&self, audio: &Path, prompt: &str, model: &str) -> Result<String, TranscribeError> {
        let mut cmd = Command::new("whisper-cli");
        cmd.arg("--model")
            .arg(model)
            .arg("--output-txt")
            .arg("--file")
            .arg(audio);
        if !prompt.is_empty() {
            cmd.arg("--prompt").arg(prompt);
        }

        let output = cmd.output().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                TranscribeError::BinaryMissing {
                    binary: "whisper-cli".to_string(),
                }
            } else {
                TranscribeError::Io(e)
            }
        })?;

        if !output.status.success() {
            let code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
            return Err(TranscribeError::BinaryFailed {
                binary: "whisper-cli".to_string(),
                code,
                stderr,
            });
        }

        let txt_path = audio.with_extension("txt");
        if !txt_path.exists() {
            return Err(TranscribeError::OutputMissing { path: txt_path });
        }
        let text = std::fs::read_to_string(&txt_path)?;
        Ok(text)
    }
}
