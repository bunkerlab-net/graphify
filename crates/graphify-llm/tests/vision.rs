//! Tests for image / vision support (#1110).
//!
//! Mirrors the pure-helper and content-builder cases in
//! `graphify-py/tests/test_image_vision.py`. The per-backend wire integration
//! (sending blocks to the live SDK) is covered where each backend is exercised.

#![allow(clippy::expect_used, unsafe_code)]

use std::path::PathBuf;

use base64::Engine;
use graphify_llm::vision::{
    self, IMAGE_TOKEN_ESTIMATE, ImageRef, MAX_IMAGES_PER_CHUNK, anthropic_content,
    backend_supports_vision, bedrock_content, build_image_refs, is_vision_image, openai_content,
    partition_semantic_files, strip_pixels, with_image_notes,
};
use serial_test::serial;

/// Any non-empty byte string stands in for image content — the builders never
/// decode pixels, they only base64 the bytes.
const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\nFAKEPIXELDATA";

/// A corpus with one raster image (in a subdir), one svg (text), one markdown.
fn make_corpus(root: &std::path::Path) -> (PathBuf, PathBuf, PathBuf) {
    std::fs::create_dir_all(root.join("sub")).expect("mkdir");
    let img = root.join("sub").join("diagram.png");
    std::fs::write(&img, PNG_BYTES).expect("write png");
    let svg = root.join("icon.svg");
    std::fs::write(&svg, "<svg><rect/></svg>").expect("write svg");
    let doc = root.join("README.md");
    std::fs::write(&doc, "# Title\nbody").expect("write md");
    (img, svg, doc)
}

#[test]
fn pdf_is_not_treated_as_vision_image() {
    let pdf = PathBuf::from("paper.pdf");
    assert!(!is_vision_image(&pdf));
    let (text, images) = partition_semantic_files(std::slice::from_ref(&pdf));
    assert_eq!(text, vec![pdf]);
    assert!(images.is_empty());
}

#[test]
fn partition_splits_raster_from_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (img, svg, doc) = make_corpus(tmp.path());
    let (text, images) = partition_semantic_files(&[doc.clone(), img.clone(), svg.clone()]);
    assert_eq!(images, vec![img]);
    // svg is XML markup -> stays on the text side (read as source, not pixels).
    let text_set: std::collections::HashSet<_> = text.into_iter().collect();
    assert_eq!(
        text_set,
        [doc, svg]
            .into_iter()
            .collect::<std::collections::HashSet<_>>()
    );
}

#[test]
fn build_image_refs_sets_rel_media_and_bytes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (img, _, _) = make_corpus(tmp.path());
    let refs = build_image_refs(std::slice::from_ref(&img), tmp.path(), true);
    let r = &refs[0];
    // strip_prefix uses the platform separator; accept either.
    assert!(
        r.rel == "sub/diagram.png" || r.rel == "sub\\diagram.png",
        "{}",
        r.rel
    );
    assert_eq!(r.media_type, "image/png");
    assert_eq!(r.raw.as_deref(), Some(PNG_BYTES));
    assert!(!r.b64().is_empty());
    assert_eq!(r.bedrock_format(), "png");
}

#[test]
fn build_image_refs_drops_oversized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // > 5 MB inline cap -> reference node only, no pixels.
    let big = tmp.path().join("big.jpg");
    std::fs::write(&big, vec![b'x'; vision::MAX_IMAGE_BYTES + 1]).expect("write big");
    let refs = build_image_refs(std::slice::from_ref(&big), tmp.path(), true);
    assert!(refs[0].raw.is_none());
    assert_eq!(refs[0].media_type, "image/jpeg");
}

#[test]
fn path_backend_skips_byte_read_and_size_cap() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // read_bytes=false (claude-cli) loads no bytes and applies no size cap.
    let big = tmp.path().join("huge.png");
    std::fs::write(&big, vec![b'x'; vision::MAX_IMAGE_BYTES + 100]).expect("write huge");
    let refs = build_image_refs(std::slice::from_ref(&big), tmp.path(), false);
    assert!(refs[0].raw.is_none());
    assert_eq!(refs[0].rel, "huge.png");
    assert_eq!(
        refs[0].path.file_name().and_then(|n| n.to_str()),
        Some("huge.png")
    );
}

#[test]
#[serial(env)]
fn capability_flags() {
    for b in [
        "claude",
        "claude-cli",
        "openai",
        "gemini",
        "bedrock",
        "kimi",
    ] {
        assert!(backend_supports_vision(b), "{b} should support vision");
    }
    assert!(!backend_supports_vision("deepseek"));

    // ollama is opt-in via env (default model is text-only).
    let prev = std::env::var("GRAPHIFY_OLLAMA_VISION").ok();
    // SAFETY: test-only, serialized via #[serial].
    unsafe { std::env::remove_var("GRAPHIFY_OLLAMA_VISION") };
    assert!(!backend_supports_vision("ollama"));
    unsafe { std::env::set_var("GRAPHIFY_OLLAMA_VISION", "1") };
    assert!(backend_supports_vision("ollama"));
    // SAFETY: restore.
    match prev {
        Some(v) => unsafe { std::env::set_var("GRAPHIFY_OLLAMA_VISION", v) },
        None => unsafe { std::env::remove_var("GRAPHIFY_OLLAMA_VISION") },
    }
}

#[test]
fn image_token_estimate_is_flat() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (img, _, _) = make_corpus(tmp.path());
    assert_eq!(
        graphify_llm::estimate_file_tokens(&img),
        IMAGE_TOKEN_ESTIMATE
    );
}

#[test]
fn chunk_packing_caps_images_per_chunk() {
    // Many images + a huge token budget must still cap images per chunk.
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut imgs = Vec::new();
    for i in 0..(MAX_IMAGES_PER_CHUNK * 2 + 3) {
        let p = tmp.path().join(format!("img{i:03}.png"));
        std::fs::write(&p, PNG_BYTES).expect("write img");
        imgs.push(p);
    }
    let chunks = graphify_llm::pack_chunks_by_tokens(&imgs, 10_000_000).expect("packs");
    assert!(
        chunks.len() >= 3,
        "expected >=3 chunks, got {}",
        chunks.len()
    );
    for chunk in &chunks {
        let n_imgs = chunk.iter().filter(|p| is_vision_image(p)).count();
        assert!(n_imgs <= MAX_IMAGES_PER_CHUNK, "chunk has {n_imgs} images");
    }
}

// ── content builders ──────────────────────────────────────────────────────

#[test]
fn anthropic_content_has_base64_block() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (img, _, _) = make_corpus(tmp.path());
    let refs = build_image_refs(std::slice::from_ref(&img), tmp.path(), true);
    let content = anthropic_content("CORPUS", &refs);
    let arr = content.as_array().expect("block list");
    assert_eq!(arr[0]["type"], "image");
    assert_eq!(arr[0]["source"]["type"], "base64");
    assert_eq!(arr[0]["source"]["media_type"], "image/png");
    assert_eq!(arr[0]["source"]["data"], refs[0].b64());
    let last = arr.last().expect("text block");
    assert_eq!(last["type"], "text");
    assert!(last["text"].as_str().expect("text").contains("CORPUS"));
}

#[test]
fn openai_content_has_data_uri() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (img, _, _) = make_corpus(tmp.path());
    let refs = build_image_refs(std::slice::from_ref(&img), tmp.path(), true);
    let content = openai_content("CORPUS", &refs);
    let arr = content.as_array().expect("part list");
    assert_eq!(arr[0]["type"], "text");
    assert_eq!(arr[1]["type"], "image_url");
    assert_eq!(
        arr[1]["image_url"]["url"],
        format!("data:image/png;base64,{}", refs[0].b64())
    );
}

#[test]
fn bedrock_image_ref_exposes_format_and_bytes() {
    // `bedrock_content` emits base64 under an `image` block that
    // `bedrock::build_messages` decodes into a typed SDK `ContentBlock::Image`:
    // assert the pieces it consumes — bare format token + the raw bytes.
    let tmp = tempfile::tempdir().expect("tempdir");
    let (img, _, _) = make_corpus(tmp.path());
    let refs = build_image_refs(std::slice::from_ref(&img), tmp.path(), true);
    assert_eq!(refs[0].bedrock_format(), "png");
    assert_eq!(refs[0].raw.as_deref(), Some(PNG_BYTES));
    // b64 round-trips the raw bytes.
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(refs[0].b64())
        .expect("valid base64");
    assert_eq!(decoded, PNG_BYTES);
}

#[test]
fn bedrock_content_has_image_block_and_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (img, _, _) = make_corpus(tmp.path());
    let refs = build_image_refs(std::slice::from_ref(&img), tmp.path(), true);
    let content = bedrock_content("CORPUS", &refs);
    let arr = content.as_array().expect("block list");
    assert_eq!(arr[0]["image"]["format"], "png");
    assert_eq!(arr[0]["image"]["data_b64"], refs[0].b64());
    let last = arr.last().expect("text block");
    assert!(last["text"].as_str().expect("text").contains("CORPUS"));
}

#[test]
fn bedrock_content_without_pixels_is_text_only() {
    // A pixel-free ref (non-vision path) emits no image block — just the text
    // note, so the image still becomes a node.
    let tmp = tempfile::tempdir().expect("tempdir");
    let (img, _, _) = make_corpus(tmp.path());
    let stripped = strip_pixels(&build_image_refs(
        std::slice::from_ref(&img),
        tmp.path(),
        true,
    ));
    let content = bedrock_content("CORPUS", &stripped);
    let arr = content.as_array().expect("block list");
    assert_eq!(arr.len(), 1);
    assert!(arr[0]["text"].as_str().expect("text").contains("CORPUS"));
}

#[test]
fn builders_fall_back_to_string_without_pixels() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (img, _, _) = make_corpus(tmp.path());
    let stripped = strip_pixels(&build_image_refs(
        std::slice::from_ref(&img),
        tmp.path(),
        true,
    ));
    let ac = anthropic_content("CORPUS", &stripped);
    let oc = openai_content("CORPUS", &stripped);
    // No pixels -> a plain string carrying the note.
    assert!(
        ac.as_str().expect("string").contains("sub/diagram.png")
            || ac.as_str().expect("string").contains("sub\\diagram.png")
    );
    assert!(
        oc.as_str().expect("string").contains("sub/diagram.png")
            || oc.as_str().expect("string").contains("sub\\diagram.png")
    );
}

#[test]
fn with_paths_notes_ask_for_read_tool_and_list_path() {
    // claude-cli path mode: the notes instruct the model to Read each image and
    // list its absolute path (so it pairs with the `--add-dir` allowlist).
    let tmp = tempfile::tempdir().expect("tempdir");
    let (img, _, _) = make_corpus(tmp.path());
    let refs = build_image_refs(std::slice::from_ref(&img), tmp.path(), false);
    let noted = with_image_notes("CORPUS", &refs, true);
    assert!(noted.contains("CORPUS"));
    assert!(noted.contains("Read tool"));
    assert!(noted.contains("path:"));
    assert!(noted.contains("diagram.png"));
}

#[test]
fn no_images_is_byte_identical() {
    let none: Vec<ImageRef> = vec![];
    assert_eq!(
        anthropic_content("PLAIN", &none),
        serde_json::json!("PLAIN")
    );
    assert_eq!(openai_content("PLAIN", &none), serde_json::json!("PLAIN"));
}
