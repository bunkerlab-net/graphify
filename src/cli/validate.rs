//! `validate` command — validate an extraction JSON file against the graphify schema.

use anyhow::Result;

/// Validate an extraction JSON file against the graphify schema.
///
/// Prints `OK: <path>` on success. Mirrors Python's `validate` command.
pub(crate) fn cmd_validate(path: &std::path::Path) -> Result<()> {
    let contents = std::fs::read_to_string(path)?;
    let value: serde_json::Value = serde_json::from_str(&contents)?;
    graphify_validate::assert_valid(&value)?;
    outln!("OK: {}", path.display());
    Ok(())
}
