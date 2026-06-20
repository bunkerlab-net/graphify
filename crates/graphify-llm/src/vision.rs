//! Image / vision support for semantic extraction (#1110).
//!
//! Ports the vision helpers from `graphify-py/graphify/llm.py`: raster images are
//! routed through per-backend vision payloads (base64 for Anthropic, data-URI for
//! OpenAI-compatible, raw bytes for Bedrock, file paths for claude-cli) so a
//! diagram/screenshot/chart becomes a graph node instead of being read as garbage
//! text. Non-vision backends fall back to a text reference via [`strip_pixels`],
//! so the node is still created — just unseen.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::{Value, json};

use crate::file_slice::Unit;

/// Raster image extensions routed through the vision path (not read as text).
pub const VISION_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

/// Per-image byte ceiling for inline (base64/bytes) backends. Anthropic caps a
/// request at 32 MB and Bedrock images at ~5 MB; 5 MB/image keeps every backend
/// within limits. Oversized images fall back to a text reference.
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// Flat token estimate per image for chunk packing. Vision models bill an image
/// at a roughly fixed cost regardless of file size, so estimating by byte size
/// would force every large PNG into its own chunk.
pub const IMAGE_TOKEN_ESTIMATE: usize = 1_600;

/// Hard cap on images per chunk, independent of the token budget (Anthropic
/// allows 100/request; the claude-cli Read-tool loop wants far fewer).
pub const MAX_IMAGES_PER_CHUNK: usize = 20;

/// Backends that read an image by file path (claude-cli's Read tool) instead of
/// inlining base64; for them no bytes are loaded and no size cap applies.
pub const PATH_IMAGE_BACKENDS: &[&str] = &["claude-cli"];

/// Map a raster image extension (without the dot, lowercased) to its media type.
#[must_use]
pub fn image_media_type(ext_lower: &str) -> &'static str {
    match ext_lower {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

/// `true` when `path`'s extension is a raster image handled by the vision path.
#[must_use]
pub fn is_vision_image(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| VISION_IMAGE_EXTENSIONS.contains(&ext.as_str()))
}

/// Split a chunk into `(text-like files, raster-image files)`.
#[must_use]
pub fn partition_semantic_files(files: &[PathBuf]) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut text = Vec::new();
    let mut images = Vec::new();
    for f in files {
        if is_vision_image(f) {
            images.push(f.clone());
        } else {
            text.push(f.clone());
        }
    }
    (text, images)
}

/// Split a chunk of units into `(text-like units, raster-image files)`.
///
/// A [`Unit::Slice`] is always text (only splittable text is sliced), so it
/// never lands in the image partition. Mirrors `_partition_semantic_files`.
#[must_use]
pub fn partition_semantic_units(units: &[Unit]) -> (Vec<Unit>, Vec<PathBuf>) {
    let mut text = Vec::new();
    let mut images = Vec::new();
    for u in units {
        match u {
            Unit::Whole(p) if is_vision_image(p) => images.push(p.clone()),
            Unit::Slice(_) | Unit::Whole(_) => text.push(u.clone()),
        }
    }
    (text, images)
}

/// A single image destined for a vision request.
///
/// `raw` is `None` when the image is unreadable, exceeds [`MAX_IMAGE_BYTES`], or
/// the target backend has no vision support — in every such case the content
/// builders emit a text reference instead of pixels, so the image still becomes a
/// graph node.
#[derive(Debug, Clone)]
pub struct ImageRef {
    /// Absolute path (claude-cli reads it via the Read tool).
    pub path: PathBuf,
    /// Path relative to the corpus root (the node's `source_file`).
    pub rel: String,
    /// Media type, e.g. `"image/png"`.
    pub media_type: String,
    /// Raw pixel bytes, or `None` (see struct docs).
    pub raw: Option<Vec<u8>>,
}

impl ImageRef {
    /// Standard base64 of the raw bytes (empty string when `raw` is `None`).
    #[must_use]
    pub fn b64(&self) -> String {
        self.raw
            .as_ref()
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
            .unwrap_or_default()
    }

    /// Bedrock Converse wants a bare format token (`png`), not a media type.
    #[must_use]
    pub fn bedrock_format(&self) -> &str {
        self.media_type
            .split_once('/')
            .map_or(self.media_type.as_str(), |(_, fmt)| fmt)
    }
}

/// Build [`ImageRef`]s for raster images.
///
/// `read_bytes = true` (base64/bytes backends) loads the pixels and drops any
/// image over [`MAX_IMAGE_BYTES`] to a reference, because an inline request body
/// has a hard size ceiling. `read_bytes = false` (path-based backends, e.g.
/// claude-cli) skips the read entirely — those backends open the file themselves
/// and downsample as needed, so there is no per-image size limit.
#[must_use]
pub fn build_image_refs(image_files: &[PathBuf], root: &Path, read_bytes: bool) -> Vec<ImageRef> {
    let mut refs = Vec::with_capacity(image_files.len());
    for p in image_files {
        let rel = p.strip_prefix(root).map_or_else(
            |_| p.to_string_lossy().into_owned(),
            |r| r.to_string_lossy().into_owned(),
        );
        let ext = p
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let media = image_media_type(&ext).to_string();
        let mut raw: Option<Vec<u8>> = None;
        if read_bytes {
            // Check the on-disk size first so an oversized image is never read
            // into memory just to be dropped — a multi-GB file would otherwise
            // be fully allocated before the length check rejected it.
            match std::fs::metadata(p) {
                Ok(meta) if meta.len() > MAX_IMAGE_BYTES as u64 => {
                    eprintln!(
                        "[graphify] image {rel} is {} KB, over the {} MB inline-image \
                         limit for this backend; sending it as a reference node without \
                         inline pixels.",
                        meta.len() / 1024,
                        MAX_IMAGE_BYTES / (1024 * 1024)
                    );
                }
                Ok(_) => match std::fs::read(p) {
                    Ok(bytes) => raw = Some(bytes),
                    Err(exc) => eprintln!("[graphify] could not read image {rel}: {exc}"),
                },
                Err(exc) => eprintln!("[graphify] could not stat image {rel}: {exc}"),
            }
        }
        let abs_path = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        refs.push(ImageRef {
            path: abs_path,
            rel,
            media_type: media,
            raw,
        });
    }
    refs
}

/// Return refs with pixel data dropped (for non-vision backends).
#[must_use]
pub fn strip_pixels(refs: &[ImageRef]) -> Vec<ImageRef> {
    refs.iter()
        .map(|r| ImageRef {
            raw: None,
            ..r.clone()
        })
        .collect()
}

/// Whether `backend`'s configured model can see images.
///
/// Ollama is special-cased: its default model is text-only, so vision is opt-in
/// via `GRAPHIFY_OLLAMA_VISION=1` once the user selects a vision model.
#[must_use]
pub fn backend_supports_vision(backend: &str) -> bool {
    if backend == "ollama" {
        return std::env::var("GRAPHIFY_OLLAMA_VISION").is_ok_and(|v| v.trim() == "1");
    }
    // Mirrors the `"vision": True` keys in Python's `BACKENDS`. Unknown backends
    // (including custom providers, which carry no vision flag) return false.
    matches!(
        backend,
        "claude" | "kimi" | "gemini" | "openai" | "bedrock" | "claude-cli"
    )
}

/// Text block listing the images so the model emits one node per image.
///
/// Always included alongside the visual payload (and used on its own when the
/// backend can't see pixels), so an image becomes a graph node either way.
/// `with_paths = true` also lists the absolute path and asks the model to open it
/// with the Read tool — used by the claude-cli backend.
#[must_use]
pub fn image_notes(refs: &[ImageRef], with_paths: bool) -> String {
    if refs.is_empty() {
        return String::new();
    }
    let header = if with_paths {
        "Use the Read tool to open and view each image file at the path below, \
         then emit one node per image"
    } else {
        "The following image file(s) are attached as visual input. Emit one \
         node per image"
    };
    let mut lines = vec![
        "=== IMAGES ===".to_string(),
        format!(
            "{header} with \"file_type\":\"image\" and the listed source_file, a label \
             describing what it depicts (diagram, screenshot, chart, photo, UI, logo), \
             and edges to any code/doc nodes the image clearly references."
        ),
    ];
    for (i, r) in refs.iter().enumerate() {
        let mut note = format!("[image {}] source_file: {}", i + 1, r.rel);
        if with_paths {
            let _ = write!(note, "  path: {}", r.path.display());
        }
        if r.raw.is_none() && !with_paths {
            note.push_str(" (not shown: unreadable or exceeds size limit)");
        }
        lines.push(note);
    }
    lines.join("\n")
}

/// Append the image notes to `user_message` (mirrors `_with_image_notes`).
#[must_use]
pub fn with_image_notes(user_message: &str, refs: &[ImageRef], with_paths: bool) -> String {
    let notes = image_notes(refs, with_paths);
    if notes.is_empty() {
        return user_message.to_string();
    }
    if user_message.trim().is_empty() {
        return notes;
    }
    format!("{user_message}\n\n{notes}")
}

/// Build the Anthropic `messages[].content` value: a plain string, or a block
/// list with base64 image blocks followed by the text block.
#[must_use]
pub fn anthropic_content(user_message: &str, refs: &[ImageRef]) -> Value {
    let blocks: Vec<Value> = refs
        .iter()
        .filter(|r| r.raw.is_some())
        .map(|r| {
            json!({
                "type": "image",
                "source": {"type": "base64", "media_type": r.media_type, "data": r.b64()},
            })
        })
        .collect();
    let text = with_image_notes(user_message, refs, false);
    if blocks.is_empty() {
        return Value::String(text);
    }
    let mut content = blocks;
    content.push(json!({"type": "text", "text": text}));
    Value::Array(content)
}

/// Build the OpenAI-compatible user `content` value: a plain string, or a part
/// list with the text part followed by `image_url` data-URI parts.
#[must_use]
pub fn openai_content(user_message: &str, refs: &[ImageRef]) -> Value {
    let parts: Vec<Value> = refs
        .iter()
        .filter(|r| r.raw.is_some())
        .map(|r| {
            json!({
                "type": "image_url",
                "image_url": {
                    "url": format!("data:{};base64,{}", r.media_type, r.b64()),
                    "detail": "auto",
                },
            })
        })
        .collect();
    let text = with_image_notes(user_message, refs, false);
    if parts.is_empty() {
        return Value::String(text);
    }
    let mut content = vec![json!({"type": "text", "text": text})];
    content.extend(parts);
    Value::Array(content)
}

/// Build the Bedrock Converse user `content` value: image blocks (carrying the
/// bare format token + base64 pixels) followed by the text block.
///
/// Python's `_bedrock_content` emits raw `bytes` that boto3 serialises directly.
/// The Rust bedrock backend keeps the uniform `messages: &[Value]` call
/// interface, so this emits base64 under an `image` block that
/// `bedrock::build_messages` decodes into a typed `ContentBlock::Image`
/// (`ImageSource::Bytes`). Unlike boto3 the base64 round-trips cleanly in Rust.
/// Only refs whose pixels loaded (`raw.is_some()`) become image blocks; the text
/// block always uses [`with_image_notes`] so every image is still a node.
#[must_use]
pub fn bedrock_content(user_message: &str, refs: &[ImageRef]) -> Value {
    let mut content: Vec<Value> = refs
        .iter()
        .filter(|r| r.raw.is_some())
        .map(|r| {
            json!({
                "image": {"format": r.bedrock_format(), "data_b64": r.b64()},
            })
        })
        .collect();
    content.push(json!({"text": with_image_notes(user_message, refs, false)}));
    Value::Array(content)
}
