//! Parity port of `graphify-py/tests/test_office_limits.py`.
//!
//! Resource-cap guards for parsing untrusted office/PDF files (F2). `.docx`/
//! `.xlsx` are zip+XML containers; a few-KB zip-bomb can decompress to
//! gigabytes and OOM-kill the process during a corpus scan. These tests verify
//! the pre-parse screen rejects bombs before the parser ever decompresses them.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::cast_possible_truncation
)]

use std::io::Write as _;
use std::path::Path;

use graphify_detect::office::{
    OFFICE_MAX_RAW_BYTES, docx_to_markdown, extract_pdf_text, file_within_size_cap,
    xlsx_to_markdown, zip_within_caps, zip_within_caps_with,
};
use zip::write::SimpleFileOptions;

/// Write a single-member deflate zip with `payload` stored at `name`.
fn write_zip(path: &Path, name: &str, payload: &[u8]) {
    let file = std::fs::File::create(path).expect("create zip");
    let mut zf = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zf.start_file(name, opts).expect("start_file");
    zf.write_all(payload).expect("write payload");
    zf.finish().expect("finish zip");
}

#[test]
fn file_within_size_cap_basic() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("a.bin");
    std::fs::write(&f, vec![b'x'; 1024]).expect("write");
    assert!(file_within_size_cap(&f, OFFICE_MAX_RAW_BYTES)); // within default cap
    assert!(!file_within_size_cap(&f, 512)); // over an explicit small cap
    assert!(!file_within_size_cap(
        &tmp.path().join("missing"),
        OFFICE_MAX_RAW_BYTES
    ));
}

#[test]
fn zip_ratio_bomb_rejected() {
    // A tiny file that expands far past the ratio threshold is rejected.
    let tmp = tempfile::tempdir().expect("tempdir");
    let bomb = tmp.path().join("bomb.xlsx");
    write_zip(
        &bomb,
        "xl/worksheets/sheet1.xml",
        &vec![b'0'; 5 * 1024 * 1024],
    );
    assert!(
        std::fs::metadata(&bomb).expect("stat").len() < 100 * 1024,
        "5 MiB of zeros should compress to well under 100 KiB"
    );
    assert!(!zip_within_caps(&bomb));
}

#[test]
fn legit_zip_passes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let ok = tmp.path().join("ok.docx");
    write_zip(
        &ok,
        "word/document.xml",
        b"<xml>hello world</xml>".repeat(20).as_slice(),
    );
    assert!(zip_within_caps(&ok));
}

#[test]
fn non_zip_rejected() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let notzip = tmp.path().join("fake.xlsx");
    std::fs::write(&notzip, b"this is not a zip file").expect("write");
    assert!(!zip_within_caps(&notzip));
}

#[test]
fn converters_return_empty_for_bomb() {
    // The live converters bail out (return "") on a bomb before parsing.
    let tmp = tempfile::tempdir().expect("tempdir");
    for ext in [".docx", ".xlsx"] {
        let bomb = tmp.path().join(format!("bomb{ext}"));
        write_zip(&bomb, "x.xml", &vec![b'0'; 5 * 1024 * 1024]);
        assert_eq!(docx_to_markdown(&bomb), "");
        assert_eq!(xlsx_to_markdown(&bomb), "");
    }
}

#[test]
fn legit_multi_member_passes_streaming() {
    // A normal multi-member office zip passes the streaming-ceiling pass.
    let tmp = tempfile::tempdir().expect("tempdir");
    let ok = tmp.path().join("ok.xlsx");
    let file = std::fs::File::create(&ok).expect("create");
    let mut zf = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zf.start_file("[Content_Types].xml", opts).expect("start");
    zf.write_all(b"<types/>").expect("write");
    zf.start_file("xl/workbook.xml", opts).expect("start");
    zf.write_all(b"<workbook/>".repeat(100).as_slice())
        .expect("write");
    zf.start_file("xl/worksheets/sheet1.xml", opts)
        .expect("start");
    zf.write_all(b"<sheetData>rows</sheetData>".repeat(500).as_slice())
        .expect("write");
    zf.finish().expect("finish");
    assert!(zip_within_caps(&ok));
}

#[test]
fn streaming_ceiling_rejects_oversized_actual() {
    // With a low decompressed cap, content whose actual bytes exceed it is
    // rejected. This exercises the authoritative bounded-decompression pass:
    // the function reads real decompressed bytes (not the attacker-declared
    // central-directory sizes) and stops once the ceiling is crossed.
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("big.xlsx");
    // ~512 KiB of incompressible data: low ratio (passes the ratio pre-filter),
    // but real decompressed size far exceeds the 64 KiB ceiling.
    let mut data = vec![0u8; 512 * 1024];
    for (i, b) in data.iter_mut().enumerate() {
        // Deterministic pseudo-random so the bytes don't compress.
        *b = (i.wrapping_mul(2_654_435_761) >> 13) as u8;
    }
    write_zip(&f, "xl/x.xml", &data);
    // 64 KiB decompressed cap via the parameterized variant (Python monkeypatches
    // the module constant; the Rust seam is an explicit cap argument).
    assert!(!zip_within_caps_with(
        &f,
        OFFICE_MAX_RAW_BYTES,
        64 * 1024,
        graphify_detect::office::OFFICE_MAX_COMPRESSION_RATIO,
    ));
}

#[test]
fn pdf_over_cap_returns_empty() {
    // A PDF larger than the raw cap is skipped before the parser opens it.
    // Use a sparse file so the over-cap size is created instantly.
    let tmp = tempfile::tempdir().expect("tempdir");
    let big = tmp.path().join("big.pdf");
    let f = std::fs::File::create(&big).expect("create");
    f.set_len(OFFICE_MAX_RAW_BYTES + 4096).expect("set_len");
    drop(f);
    assert!(!file_within_size_cap(&big, OFFICE_MAX_RAW_BYTES));
    assert_eq!(extract_pdf_text(&big), "");
}
