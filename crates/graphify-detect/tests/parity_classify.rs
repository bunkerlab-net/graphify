//! Parity tests for file classification.
//!
//! Mirrors `graphify-py/tests/test_detect.py` — classification tests.
#![allow(clippy::expect_used)]

use graphify_detect::{FileType, classify_file, is_package_manifest_path};
use std::path::Path;
use tempfile::tempdir;

#[test]
fn classify_python() {
    assert_eq!(classify_file(Path::new("foo.py")), Some(FileType::Code));
}

#[test]
fn classify_typescript() {
    assert_eq!(classify_file(Path::new("bar.ts")), Some(FileType::Code));
}

/// #1315: `.psm1` PowerShell modules were never indexed (a `CODE_EXTENSIONS` gap).
#[test]
fn classify_powershell_module() {
    assert_eq!(classify_file(Path::new("Utils.psm1")), Some(FileType::Code));
}

/// #1331: `.psd1` PowerShell manifests must classify as code so the manifest
/// extractor runs.
#[test]
fn classify_powershell_manifest() {
    assert_eq!(
        classify_file(Path::new("MyModule.psd1")),
        Some(FileType::Code)
    );
}

/// CUDA sources classify as code so `.cu`/`.cuh` route through the C++
/// extractor (graphify-py: `.cu`/`.cuh` added to `CODE_EXTENSIONS`).
#[test]
fn classify_cuda_cu() {
    assert_eq!(classify_file(Path::new("kernel.cu")), Some(FileType::Code));
}

#[test]
fn classify_cuda_cuh() {
    assert_eq!(classify_file(Path::new("kernel.cuh")), Some(FileType::Code));
}

/// #1377: package manifests route to the deterministic AST/code path, not the
/// LLM document path — even when their extension (`.yml`/`.toml`/`.xml`) would
/// otherwise classify as a document. A generic yaml stays a document. Mirrors
/// Python `test_manifests_classify_as_code_not_document`.
#[test]
fn manifests_classify_as_code_not_document() {
    let tmp = tempdir().expect("tempdir");
    for name in ["apm.yml", "pyproject.toml", "go.mod", "pom.xml"] {
        let p = tmp.path().join(name);
        std::fs::write(&p, "x").expect("write manifest");
        assert!(is_package_manifest_path(&p), "{name} is a manifest");
        assert_eq!(classify_file(&p), Some(FileType::Code), "{name} -> Code");
    }
    // A generic yaml stays a document.
    let cfg = tmp.path().join("config.yaml");
    std::fs::write(&cfg, "a: 1").expect("write config");
    assert_eq!(classify_file(&cfg), Some(FileType::Document));
}

#[test]
fn classify_byond_dreammaker() {
    // BYOND DreamMaker source + asset extensions added in graphify-py v0.8.22.
    for name in [
        "code.dm",
        "env.dme",
        "icons.dmi",
        "map.dmm",
        "interface.dmf",
    ] {
        assert_eq!(
            classify_file(Path::new(name)),
            Some(FileType::Code),
            "{name} should classify as code",
        );
    }
}

#[test]
fn classify_markdown() {
    assert_eq!(
        classify_file(Path::new("README.md")),
        Some(FileType::Document)
    );
}

#[test]
fn classify_pdf() {
    assert_eq!(classify_file(Path::new("paper.pdf")), Some(FileType::Paper));
}

#[test]
fn classify_pdf_in_xcassets_skipped() {
    let p = Path::new("MyApp/Images.xcassets/icon.imageset/icon.pdf");
    assert_eq!(classify_file(p), None);
}

#[test]
fn classify_pdf_in_xcassets_root_skipped() {
    let p = Path::new("Pods/HXPHPicker/Assets.xcassets/photo.pdf");
    assert_eq!(classify_file(p), None);
}

#[test]
fn classify_unknown_returns_none() {
    assert_eq!(classify_file(Path::new("archive.zip")), None);
}

#[test]
fn classify_image_png() {
    assert_eq!(
        classify_file(Path::new("screenshot.png")),
        Some(FileType::Image)
    );
}

#[test]
fn classify_image_jpg() {
    assert_eq!(
        classify_file(Path::new("design.jpg")),
        Some(FileType::Image)
    );
}

#[test]
fn classify_image_webp() {
    assert_eq!(
        classify_file(Path::new("diagram.webp")),
        Some(FileType::Image)
    );
}

#[test]
fn classify_video_mp4() {
    assert_eq!(
        classify_file(Path::new("lecture.mp4")),
        Some(FileType::Video)
    );
}

#[test]
fn classify_video_mp3() {
    assert_eq!(
        classify_file(Path::new("podcast.mp3")),
        Some(FileType::Video)
    );
}

#[test]
fn classify_video_mov() {
    assert_eq!(classify_file(Path::new("talk.mov")), Some(FileType::Video));
}

#[test]
fn classify_video_wav() {
    assert_eq!(
        classify_file(Path::new("recording.wav")),
        Some(FileType::Video)
    );
}

#[test]
fn classify_video_webm() {
    assert_eq!(
        classify_file(Path::new("webinar.webm")),
        Some(FileType::Video)
    );
}

#[test]
fn classify_video_m4a() {
    assert_eq!(classify_file(Path::new("audio.m4a")), Some(FileType::Video));
}

#[test]
fn classify_google_workspace_gdoc() {
    assert_eq!(
        classify_file(Path::new("notes.gdoc")),
        Some(FileType::Document)
    );
}

#[test]
fn classify_google_workspace_gsheet() {
    assert_eq!(
        classify_file(Path::new("budget.gsheet")),
        Some(FileType::Document)
    );
}

#[test]
fn classify_google_workspace_gslides() {
    assert_eq!(
        classify_file(Path::new("deck.gslides")),
        Some(FileType::Document)
    );
}

/// Only `.gdoc`/`.gsheet`/`.gslides` are Google Workspace shortcuts (Python
/// parity: `GOOGLE_WORKSPACE_EXTENSIONS = {".gdoc", ".gsheet", ".gslides"}`).
/// Other Drive shortcut types are unrecognized and classify as `None`, so they
/// are ignored exactly as graphify-py ignores them.
#[test]
fn classify_other_google_drive_shortcuts_return_none() {
    for name in ["diagram.gdraw", "survey.gform", "places.gmap", "site.gsite"] {
        assert_eq!(classify_file(Path::new(name)), None, "{name} -> None");
    }
}

#[test]
fn classify_md_paper_by_signals() {
    let tmp = tempdir().expect("tempdir");
    let paper = tmp.path().join("paper.md");
    std::fs::write(
        &paper,
        "# Abstract\n\nWe propose a new method. See [1] and [23].\n\
         This work was published in the Journal of AI. ArXiv preprint.\n\
         See Equation 3 for details. \\cite{vaswani2017}.\n",
    )
    .expect("test invariant");
    assert_eq!(classify_file(&paper), Some(FileType::Paper));
}

#[test]
fn classify_md_doc_without_signals() {
    let tmp = tempdir().expect("tempdir");
    let doc = tmp.path().join("notes.md");
    std::fs::write(
        &doc,
        "# My Notes\n\nHere are some notes about the project.\n",
    )
    .expect("test invariant");
    assert_eq!(classify_file(&doc), Some(FileType::Document));
}
