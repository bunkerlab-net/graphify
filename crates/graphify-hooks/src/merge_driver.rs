//! `graph.json` union merge-driver registration (#1902).
//!
//! `hook install` registers a git merge driver + `.gitattributes` entry so
//! `graph.json` merges by union (`graphify merge-driver`) instead of raising a
//! conflict on every branch merge. README/CHANGELOG 0.7.0 documented this as
//! part of `hook install`, but the git-hook install never actually registered
//! it — this closes that gap. Ports `_register_merge_driver`,
//! `_unregister_merge_driver`, and `_merge_driver_status` from graphify-py
//! `hooks.py`.

use std::path::Path;
use std::process::Command;

/// The git config value for `merge.graphify.driver`. The Rust binary is not a
/// Python interpreter, so — like the git hooks — the driver relies on the
/// `graphify` launcher being on PATH at merge time rather than pinning an
/// interpreter path.
const DRIVER: &str = "graphify merge-driver %O %A %B";

/// The repo-relative `graph.json` path graphify assigns the merge driver to,
/// e.g. `graphify-out/graph.json`. The graph lives under the configured output
/// dir (`GRAPHIFY_OUT`); gitattributes patterns are repo-relative, so an
/// absolute override, one with a backslash, or one containing WHITESPACE (which
/// would split the space-delimited attribute line) cannot be expressed there —
/// fall back to the default name in that case. DIVERGENCE from graphify-py
/// `_merge_attr_line` (`hooks.py:508`), which does not reject whitespace and so
/// emits a malformed line for `GRAPHIFY_OUT="my dir"` (AGENTS.md: fix reference
/// bugs, do not replicate).
#[must_use]
fn merge_attr_path() -> String {
    let raw = std::env::var("GRAPHIFY_OUT").unwrap_or_default();
    let out = if raw.is_empty()
        || Path::new(&raw).is_absolute()
        || raw.contains('\\')
        || raw.chars().any(char::is_whitespace)
    {
        "graphify-out"
    } else {
        &raw
    };
    format!("{}/graph.json", out.trim_end_matches('/'))
}

/// The full `.gitattributes` line assigning the graphify merge driver.
#[must_use]
fn merge_attr_line() -> String {
    format!("{} merge=graphify", merge_attr_path())
}

/// True if a (non-comment) line assigns the graphify merge driver to graphify's
/// EXACT `graph.json` path. Matches the whole first field against
/// [`merge_attr_path`] — not a loose `graph.json` suffix — so an unrelated
/// `othergraph.json merge=graphify` or a user's own `docs/graph.json` entry is
/// neither counted as registered nor removed on uninstall. DIVERGENCE from
/// graphify-py `_has_merge_attr` (`hooks.py:520`, `endswith("graph.json")`),
/// whose loose suffix match false-positives and would delete unrelated lines
/// (AGENTS.md: fix reference bugs, do not replicate).
#[must_use]
fn has_merge_attr(content: &str) -> bool {
    let expected = merge_attr_path();
    content.lines().any(|raw| {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some(first) if first == expected => fields.any(|f| f == "merge=graphify"),
            _ => false,
        }
    })
}

/// Run `git -C <root> config <args>`.
fn git_config(root: &Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .arg("config")
        .args(args)
        .output()
}

/// Register the `graph.json` union merge driver in git config + `.gitattributes`
/// (#1902). Writes go through `git config` (never hand-edit `.git/config` — in a
/// linked worktree the effective config is not at `root/.git/config`).
///
/// # Errors
///
/// Returns [`HooksError::Io`] when a `.gitattributes` read or write fails — the
/// same way graphify-py's unguarded `write_text` raises (`hooks.py:562`), so a
/// read-only vault fails `hook install` rather than reporting a phantom success.
/// A failed `git config` invocation is reported in the returned status string
/// (mirroring the reference's caught `CalledProcessError`), not raised.
pub fn register_merge_driver(root: &Path) -> Result<String, crate::error::HooksError> {
    for (key, value) in [
        ("merge.graphify.name", "graphify graph.json union merge"),
        ("merge.graphify.driver", DRIVER),
    ] {
        match git_config(root, &[key, value]) {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                return Ok(format!(
                    "not registered (git config failed: {})",
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
            Err(e) => return Ok(format!("not registered (git config failed: {e})")),
        }
    }

    let line = merge_attr_line();
    let attrs = root.join(".gitattributes");
    if attrs.exists() {
        let content = std::fs::read_to_string(&attrs)?;
        if has_merge_attr(&content) {
            return Ok(format!("already registered ({line})"));
        }
        // Never clobber other entries; preserve a trailing newline.
        let mut content = content;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&line);
        content.push('\n');
        std::fs::write(&attrs, content)?;
    } else {
        std::fs::write(&attrs, format!("{line}\n"))?;
    }
    Ok(format!("registered ({line})"))
}

/// Remove the merge-driver git config keys and the `.gitattributes` line.
/// Mirrors `_unregister_merge_driver`.
///
/// # Errors
///
/// Returns [`HooksError::Io`] when a `.gitattributes` read, write, or delete
/// fails (graphify-py's `write_text`/`unlink` at `hooks.py:592,594` are
/// unguarded and raise). A `git config --unset` on an absent key is expected to
/// exit nonzero and is ignored, matching the reference.
pub fn unregister_merge_driver(root: &Path) -> Result<String, crate::error::HooksError> {
    for key in ["merge.graphify.name", "merge.graphify.driver"] {
        // Best-effort, matching graphify-py `hooks.py:571-579`: `_sp.run(--unset)`
        // runs WITHOUT `check=True` and catches only `OSError`, so every outcome
        // (missing key, launch failure, any nonzero exit) is ignored and the
        // `.gitattributes` line is then removed regardless. Propagating these
        // would diverge from the reference and fail uninstall on transient git
        // errors. (Disputes CodeRabbit's "propagate non-missing-key --unset
        // failures" finding.)
        let _ = git_config(root, &["--unset", key]);
    }
    let attrs = root.join(".gitattributes");
    if !attrs.exists() {
        return Ok("not registered - nothing to remove.".to_string());
    }
    let content = std::fs::read_to_string(&attrs)?;
    let kept: Vec<&str> = content.lines().filter(|raw| !has_merge_attr(raw)).collect();
    if kept.len() == content.lines().count() {
        return Ok("gitattributes entry not found - nothing to remove.".to_string());
    }
    if kept.is_empty() {
        std::fs::remove_file(&attrs)?;
        return Ok("removed (.gitattributes deleted - no other entries)".to_string());
    }
    std::fs::write(&attrs, format!("{}\n", kept.join("\n")))?;
    Ok("removed from .gitattributes (other entries preserved)".to_string())
}

/// Report whether the merge driver is registered (config + gitattributes).
/// Mirrors `_merge_driver_status`.
#[must_use]
pub fn merge_driver_status(root: &Path) -> String {
    // The config value must be graphify's EXACT driver command; an unrelated
    // `merge.graphify.driver` must not read as healthy. DIVERGENCE from
    // graphify-py (`hooks.py:606`, nonempty check), which reports any command as
    // registered (AGENTS.md: fix reference bugs, do not replicate).
    let cfg_ok = git_config(root, &["--get", "merge.graphify.driver"])
        .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == DRIVER);
    let attrs = root.join(".gitattributes");
    let attr_ok =
        attrs.exists() && std::fs::read_to_string(&attrs).is_ok_and(|c| has_merge_attr(&c));
    match (cfg_ok, attr_ok) {
        (true, true) => "registered".to_string(),
        (true, false) => {
            "partially registered (git config set, .gitattributes line missing)".to_string()
        }
        (false, true) => {
            "partially registered (.gitattributes line set, git config missing)".to_string()
        }
        (false, false) => "not registered".to_string(),
    }
}
