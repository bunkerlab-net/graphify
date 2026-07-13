//! `.dmi` icon-sheet extractor (PNG with BYOND metadata).

use crate::ids::{file_stem, make_id, make_id1};
use crate::types::{Edge, FileResult, Node as GNode};
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;

/// Decompress up to 1 MiB of a zTXt zlib stream (best effort).
///
/// graphify-py lets a corrupt zlib stream raise; we degrade gracefully and keep
/// whatever decompressed cleanly. The 1 MiB cap mirrors graphify-py's
/// `max_length` guard against decompression bombs.
fn decompress_capped(compressed: &[u8]) -> String {
    let mut out = Vec::new();
    let mut decoder = flate2::read::ZlibDecoder::new(compressed).take(1024 * 1024);
    let _ = decoder.read_to_end(&mut out);
    String::from_utf8_lossy(&out).into_owned()
}

/// Pull the BYOND metadata text out of a `.dmi` PNG, or `""` on failure.
///
/// Scans PNG chunks for a `tEXt`/`zTXt` chunk keyed `Description`; zTXt payloads
/// are zlib-decompressed (capped). Mirrors graphify-py `_read_dmi_description`.
fn read_dmi_description(data: &[u8]) -> String {
    const PNG_SIG: &[u8] = b"\x89PNG\r\n\x1a\n";
    if !data.starts_with(PNG_SIG) {
        return String::new();
    }
    let mut i = 8usize;
    while i + 8 <= data.len() {
        let length = usize::try_from(u32::from_be_bytes([
            data[i],
            data[i + 1],
            data[i + 2],
            data[i + 3],
        ]))
        .unwrap_or(0);
        let chunk_type = &data[i + 4..i + 8];
        let payload_start = i + 8;
        let payload_end = payload_start.saturating_add(length).min(data.len());
        let payload = &data[payload_start..payload_end];
        if chunk_type == b"tEXt" || chunk_type == b"zTXt" {
            let Some(nul) = payload.iter().position(|&b| b == 0) else {
                return String::new();
            };
            if &payload[..nul] == b"Description" {
                if chunk_type == b"zTXt" {
                    // zTXt: keyword \0 compression_method(1 byte) compressed_data
                    return decompress_capped(payload.get(nul + 2..).unwrap_or(&[]));
                }
                // tEXt: keyword \0 text
                return String::from_utf8_lossy(payload.get(nul + 1..).unwrap_or(&[])).into_owned();
            }
        }
        i = i.saturating_add(8).saturating_add(length).saturating_add(4);
    }
    String::new()
}

/// Extract icon state names from a `.dmi` (BYOND PNG icon sheet).
#[must_use]
pub fn extract_dmi(path: &Path) -> FileResult {
    let data = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return FileResult::error(e.to_string()),
    };
    let str_path = path.to_string_lossy().into_owned();
    let stem = file_stem(path);
    let file_nid = make_id1(&str_path);
    let file_label = path
        .file_name()
        .map_or(String::new(), |f| f.to_string_lossy().into_owned());

    let mut nodes = vec![GNode {
        id: file_nid.clone(),
        label: file_label,
        file_type: "code".to_string(),
        source_file: str_path.clone(),
        source_location: Some("L1".to_string()),
        metadata: None,
        origin_file: None,
        node_type: None,
    }];
    let mut edges: Vec<Edge> = Vec::new();
    let mut seen: HashSet<String> = HashSet::from([file_nid.clone()]);

    let description = read_dmi_description(&data);
    if description.is_empty() {
        return FileResult {
            nodes,
            edges,
            raw_calls: Vec::new(),
            error: None,
        };
    }

    let mut line_no: u32 = 0;
    for raw_line in description.lines() {
        line_no += 1;
        let stripped = raw_line.trim();
        if !stripped.starts_with("state =") {
            continue;
        }
        let value = stripped.split_once('=').map_or("", |(_, v)| v).trim();
        let state_name = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            &value[1..value.len() - 1]
        } else {
            value
        };
        if state_name.is_empty() {
            continue;
        }
        let nid = make_id(&[&stem, "state", state_name]);
        if !seen.insert(nid.clone()) {
            continue;
        }
        nodes.push(GNode {
            id: nid.clone(),
            label: format!("\"{state_name}\""),
            file_type: "code".to_string(),
            source_file: str_path.clone(),
            source_location: Some(format!("L{line_no}")),
            metadata: None,
            origin_file: None,
            node_type: None,
        });
        edges.push(Edge {
            source: file_nid.clone(),
            target: nid,
            relation: "contains".to_string(),
            confidence: "EXTRACTED".to_string(),
            source_file: str_path.clone(),
            source_location: Some(format!("L{line_no}")),
            weight: 1.0,
            context: None,
            confidence_score: None,
            external: false,
            deferred: false,
            metadata: None,
        });
    }

    FileResult {
        nodes,
        edges,
        raw_calls: Vec::new(),
        error: None,
    }
}

// ── .dmm (BYOND map files) ─────────────────────────────────────────────────────
