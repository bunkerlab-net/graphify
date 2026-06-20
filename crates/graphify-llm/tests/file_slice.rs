//! Parity tests for `graphify_llm::file_slice` and the unit-aware pipeline
//! helpers (#1369). Mirrors `graphify-py/tests/test_file_slice.py`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};

use graphify_llm::{
    FILE_CHAR_CAP, FileSlice, Unit, bisect_slice, estimate_unit_tokens, expand_oversized_files,
    is_splittable_text, pack_chunks_by_tokens_units, partition_semantic_units, read_slice_text,
    read_units, slice_boundaries, unit_path,
};

fn write_file(path: &Path, text: &str) -> PathBuf {
    fs::write(path, text).expect("write fixture");
    path.to_path_buf()
}

fn slices_of(units: &[Unit]) -> Vec<&FileSlice> {
    units
        .iter()
        .filter_map(|u| match u {
            Unit::Slice(fs) => Some(fs),
            Unit::Whole(_) => None,
        })
        .collect()
}

// ── slice_boundaries: coverage + bounds invariants ──────────────────────────

#[test]
fn slice_boundaries_small_text_is_one_range() {
    let text = "short doc";
    assert_eq!(slice_boundaries(text, 100), vec![(0, text.len())]);
}

#[test]
fn slice_boundaries_full_coverage_and_bounds() {
    let block = "# Heading\n\n".to_string() + &"lorem ipsum ".repeat(40) + "\n\n";
    let text = block.repeat(20);
    for max_chars in [50usize, 100, 500, 1000] {
        let bounds = slice_boundaries(&text, max_chars);
        // contiguous, gap-free, covering the whole text
        assert_eq!(bounds[0].0, 0);
        assert_eq!(bounds[bounds.len() - 1].1, text.len());
        for w in bounds.windows(2) {
            assert_eq!(w[0].1, w[1].0);
        }
        // concatenation reproduces the text exactly (no dropped content)
        let joined: String = bounds.iter().map(|&(s, e)| &text[s..e]).collect();
        assert_eq!(joined, text);
        // every slice respects the budget
        assert!(bounds.iter().all(|&(s, e)| e - s <= max_chars));
    }
}

#[test]
fn slice_boundaries_single_huge_line_still_progresses() {
    // No newline at all -> must hard-cut and still cover everything.
    let text = "x".repeat(5000);
    let bounds = slice_boundaries(&text, 1000);
    let joined: String = bounds.iter().map(|&(s, e)| &text[s..e]).collect();
    assert_eq!(joined, text);
    assert!(bounds.iter().all(|&(s, e)| e - s <= 1000));
}

#[test]
fn slice_boundaries_prefers_heading_boundary() {
    let a = "# A\n".to_string() + &"a".repeat(30) + "\n";
    let b = "# B\n".to_string() + &"b".repeat(30) + "\n";
    let text = format!("{a}{b}");
    // Force a split near the A/B seam; the second slice must lead with "# B".
    let bounds = slice_boundaries(&text, a.len() + 5);
    let second_start = bounds[1].0;
    assert_eq!(&text[second_start..second_start + 3], "# B");
}

// ── expand_oversized_files ──────────────────────────────────────────────────

#[test]
fn expand_small_file_stays_whole() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = write_file(&tmp.path().join("small.md"), "# Tiny\n\nhi\n");
    let units = expand_oversized_files(std::slice::from_ref(&f), 1000);
    assert_eq!(units, vec![Unit::Whole(f)]);
}

#[test]
fn expand_oversized_markdown_is_sliced_with_full_coverage() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let block = "# Section\n\n".to_string() + &"word ".repeat(200) + "\n\n";
    let text = block.repeat(30);
    let f = write_file(&tmp.path().join("big.md"), &text);
    let units = expand_oversized_files(std::slice::from_ref(&f), 2000);
    let slices = slices_of(&units);
    assert!(slices.len() >= 2);
    assert!(units.iter().all(|u| matches!(u, Unit::Slice(_))));
    // slices reconstruct the whole file
    let joined: String = slices
        .iter()
        .map(|s| read_slice_text(s).expect("readable"))
        .collect();
    assert_eq!(joined, text);
    assert!(slices.iter().all(|s| s.end - s.start <= 2000));
    // every slice points back at the parent file (anti-fragmentation)
    assert!(slices.iter().all(|s| s.path == f));
    assert_eq!(slices[0].total, slices.len());
}

#[test]
fn expand_does_not_slice_code_even_when_oversized() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Far over the cap, but code is never sliced (needs whole-symbol context).
    let f = write_file(&tmp.path().join("mod.py"), &"x = 1\n".repeat(6000));
    assert!(!is_splittable_text(&f));
    let units = expand_oversized_files(std::slice::from_ref(&f), 2000);
    assert_eq!(units, vec![Unit::Whole(f)]);
}

#[test]
fn expand_unreadable_file_passes_through() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let missing = tmp.path().join("nope.md");
    let units = expand_oversized_files(std::slice::from_ref(&missing), 10);
    assert_eq!(units, vec![Unit::Whole(missing)]);
}

// ── anti-fragmentation: slices share one source_file in the prompt ──────────

#[test]
fn read_units_keys_every_slice_to_parent_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let block = "# H\n\n".to_string() + &"lorem ".repeat(300) + "\n\n";
    let text = block.repeat(20);
    let f = write_file(&tmp.path().join("doc.md"), &text);
    let units = expand_oversized_files(std::slice::from_ref(&f), FILE_CHAR_CAP);
    let n_slices = slices_of(&units).len();
    assert!(n_slices >= 2);

    let prompt = read_units(&units, tmp.path());
    // One untrusted_source block per slice, every one keyed to the parent path.
    let total_blocks = prompt.matches("<untrusted_source path=\"").count();
    let parent_blocks = prompt.matches("<untrusted_source path=\"doc.md\"").count();
    assert_eq!(total_blocks, n_slices);
    assert_eq!(parent_blocks, n_slices);
}

// ── unit helpers, estimation, partition, packing ────────────────────────────

#[test]
fn unit_path_resolves_slice_and_path() {
    let f = PathBuf::from("/tmp/a.md");
    let fs = FileSlice {
        path: f.clone(),
        start: 0,
        end: 5,
        index: 0,
        total: 1,
    };
    assert_eq!(unit_path(&Unit::Slice(fs)), f.as_path());
    assert_eq!(unit_path(&Unit::Whole(f.clone())), f.as_path());
}

#[test]
fn estimate_tokens_for_slice_scales_with_range() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = write_file(&tmp.path().join("a.md"), &"z".repeat(10_000));
    let small = Unit::Slice(FileSlice {
        path: f.clone(),
        start: 0,
        end: 100,
        index: 0,
        total: 2,
    });
    let big = Unit::Slice(FileSlice {
        path: f,
        start: 0,
        end: 8000,
        index: 1,
        total: 2,
    });
    assert!(estimate_unit_tokens(&small) < estimate_unit_tokens(&big));
}

#[test]
fn partition_keeps_slices_as_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("a.md");
    let fs = FileSlice {
        path: f,
        start: 0,
        end: 5,
        index: 0,
        total: 1,
    };
    let img = tmp.path().join("pic.png");
    let (text_units, image_files) =
        partition_semantic_units(&[Unit::Slice(fs.clone()), Unit::Whole(img.clone())]);
    assert!(text_units.contains(&Unit::Slice(fs)));
    assert_eq!(image_files, vec![img]);
}

#[test]
fn pack_chunks_handles_slices() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let block = "# H\n\n".to_string() + &"word ".repeat(300) + "\n\n";
    let text = block.repeat(20);
    let f = write_file(&tmp.path().join("big.md"), &text);
    let units = expand_oversized_files(std::slice::from_ref(&f), FILE_CHAR_CAP);
    let chunks = pack_chunks_by_tokens_units(&units, 2000).expect("packs");
    // All units land in some chunk; flattening recovers them all.
    let flat: usize = chunks.iter().map(Vec::len).sum();
    assert_eq!(flat, units.len());
}

// ── bisect_slice (adaptive-retry path) ──────────────────────────────────────

#[test]
fn bisect_slice_splits_at_newline() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = write_file(&tmp.path().join("a.md"), &"alpha\n".repeat(100));
    let fs = FileSlice {
        path: f,
        start: 0,
        end: 600,
        index: 0,
        total: 1,
    };
    let (left, right) = bisect_slice(&fs).expect("splits");
    assert_eq!(left.start, fs.start);
    assert_eq!(right.end, fs.end);
    assert_eq!(left.end, right.start); // contiguous, no gap
    assert!(fs.start < left.end && left.end < fs.end);
}

#[test]
fn bisect_slice_returns_none_for_tiny() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = write_file(&tmp.path().join("a.md"), "ab");
    assert!(
        bisect_slice(&FileSlice {
            path: f,
            start: 0,
            end: 1,
            index: 0,
            total: 1,
        })
        .is_none()
    );
}
