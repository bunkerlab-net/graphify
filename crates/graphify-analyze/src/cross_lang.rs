//! Cross-language and community mapping helpers.
//!
//! Extracted from `lib.rs` to isolate `cross_language` (cross-language
//! suppression logic) and `node_community_map` (community index inversion)
//! used by both `surprises` and `suggest`.

use indexmap::IndexMap;

use crate::classify::LANG_FAMILY;

/// Return true if the two source files belong to different language families.
pub(crate) fn cross_language(src_a: &str, src_b: &str) -> bool {
    use std::path::Path;
    let ext_a = Path::new(src_a)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()));
    let ext_b = Path::new(src_b)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{}", e.to_lowercase()));
    match (ext_a, ext_b) {
        (Some(a), Some(b)) => {
            let fam_a = LANG_FAMILY.get(a.as_str());
            let fam_b = LANG_FAMILY.get(b.as_str());
            matches!((fam_a, fam_b), (Some(a), Some(b)) if a != b)
        }
        _ => false,
    }
}

/// Build a `node_id → community_id` inversion of the communities map.
pub(crate) fn node_community_map(
    communities: &IndexMap<i64, Vec<String>>,
) -> IndexMap<String, i64> {
    let mut m = IndexMap::new();
    for (cid, nodes) in communities {
        for n in nodes {
            m.insert(n.clone(), *cid);
        }
    }
    m
}
