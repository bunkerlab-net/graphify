//! .NET project-file extractors: `.sln`, `.csproj` / `.fsproj` / `.vbproj`, `.razor` / `.cshtml`.
//!
//! Ports `graphify-py/graphify/extract.py::extract_sln`,
//! `extract_csproj`, and `extract_razor`. The Python originals are three
//! discrete top-level helpers; in Rust they're co-located here because they
//! share the same target ecosystem and small helpers.

mod csproj;
mod razor;
mod sln;
mod slnx;

pub use csproj::extract_csproj;
pub use razor::extract_razor;
pub use sln::extract_sln;
pub use slnx::extract_slnx;

use quick_xml::events::BytesStart;

/// `MSBuild` project files (`.csproj` / `.fsproj` / `.vbproj`) larger than this
/// are skipped with an error. Real-world projects are well under 2 MiB; the
/// cap protects the extractor against accidentally being pointed at a
/// committed binary or a multi-megabyte generated artefact. Matches the
/// literal 2 MiB constant in `graphify-py` `extract.py::extract_csproj`,
/// so the cap is intentionally not configurable — raising or lowering it
/// across the Python/Rust pair belongs in a separate parity-bumping change.
const CSPROJ_MAX_BYTES: u64 = 2_097_152;

/// Strip an XML element's namespace prefix so callers can match on the local
/// tag name. Matches Python's `tag.split('}')[1]` pattern.
fn local_name(start: &BytesStart<'_>) -> String {
    let name = start.name();
    let raw = name.as_ref();
    let local = raw
        .iter()
        .rposition(|&b| b == b':')
        .map_or(raw, |i| &raw[i + 1..]);
    String::from_utf8_lossy(local).into_owned()
}

/// Find `attr` on a `BytesStart`, falling back to its lowercased variant —
/// mirrors Python's case-insensitive `Include`/`include` lookup. Returns
/// `None` when neither attribute is present.
fn attr_ci(start: &BytesStart<'_>, attr: &str) -> Option<String> {
    start
        .try_get_attribute(attr)
        .ok()
        .flatten()
        .or_else(|| {
            start
                .try_get_attribute(attr.to_lowercase().as_str())
                .ok()
                .flatten()
        })
        // `normalized_value` decodes XML entities (`&amp;` → `&`, `&#x2F;`
        // → `/`, etc.) and collapses whitespace per the XML attribute-value
        // normalization rules. Python's ElementTree returns already-decoded
        // attribute text, so we match that here — a
        // `PackageReference Include="A&amp;B"` becomes the literal `A&B`
        // node label instead of `A&amp;B`.
        .and_then(|a| {
            // Treat the document as XML 1.0 when the declaration was
            // omitted (csproj files almost never carry an `<?xml ?>` prolog).
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(std::borrow::Cow::into_owned)
        })
}
