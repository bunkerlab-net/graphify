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
/// dir (`GRAPHIFY_OUT`); gitattributes patterns are repo-relative, so a value
/// that cannot be expressed as a literal pattern — absolute, backslash,
/// whitespace (splits the space-delimited line), or a gitattributes glob
/// metacharacter (`*`/`?`/`[`, which would turn the path into a wildcard) —
/// falls back to the default name. DIVERGENCE from graphify-py `_merge_attr_line`
/// (`hooks.py:508`), which rejects only absolute/backslash and so emits a
/// malformed or wildcard line for such values (AGENTS.md: fix reference bugs,
/// do not replicate).
#[must_use]
fn merge_attr_path() -> String {
    let raw = std::env::var("GRAPHIFY_OUT").unwrap_or_default();
    // Unsafe anywhere: whitespace/backslash and glob metacharacters (`*?[`).
    let unsafe_char = |c: char| c.is_whitespace() || matches!(c, '\\' | '*' | '?' | '[');
    // Unsafe only line-leading: gitattributes treats `#` as a comment, `!` as a
    // negation, and `"` as a quoted-pattern opener.
    let leads_control = matches!(raw.chars().next(), Some('#' | '!' | '"'));
    let out = if raw.is_empty()
        || Path::new(&raw).is_absolute()
        || raw.contains(unsafe_char)
        || leads_control
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

/// True when graphify's merge driver is the EFFECTIVE `merge` setting for its
/// exact `graph.json` path. Only the whole first field is matched against
/// [`merge_attr_path`] (not a loose `graph.json` suffix), and — because git
/// resolves attributes last-match-wins — the LAST `merge`-related token across
/// matching lines decides: a later `merge=other`, `-merge`, or `!merge` means
/// graphify's driver is NOT active, so `install` must re-append and `status`
/// must report it missing. DIVERGENCE from graphify-py `_has_merge_attr`
/// (`hooks.py:520`): its `endswith("graph.json")` + `"merge=graphify" in fields`
/// both false-positives on unrelated paths AND misreports "registered" when a
/// later token overrides graphify's (AGENTS.md: fix reference bugs).
#[must_use]
fn has_merge_attr(content: &str) -> bool {
    let expected = merge_attr_path();
    // `Some(true)` = the last merge token for the path is `merge=graphify`;
    // `Some(false)` = a different/unset merge token overrode it; `None` = none.
    let mut effective: Option<bool> = None;
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        if fields.next() != Some(expected.as_str()) {
            continue;
        }
        for f in fields {
            if f == "merge=graphify" {
                effective = Some(true);
            } else if f == "-merge" || f == "!merge" || f.starts_with("merge=") {
                effective = Some(false);
            }
        }
    }
    effective == Some(true)
}

/// Outcome of classifying a `.gitattributes` line for uninstall.
enum MergeLine {
    /// Not graphify's merge attribute — keep the line verbatim.
    Other,
    /// The line was ONLY graphify's `<path> merge=graphify` — drop it.
    OnlyMerge,
    /// The line carried other attributes — keep the path plus those, dropping
    /// only the `merge=graphify` token so user attributes (e.g. `text eol=lf`)
    /// survive. DIVERGENCE from graphify-py, whose uninstall drops the whole
    /// matching line and loses co-located attributes.
    Rewritten(String),
}

/// Classify a `.gitattributes` line for token-level uninstall removal.
#[must_use]
fn strip_graphify_merge_attr(raw: &str, expected_path: &str) -> MergeLine {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
        return MergeLine::Other;
    }
    let mut fields = line.split_whitespace();
    if fields.next() != Some(expected_path) {
        return MergeLine::Other;
    }
    let rest: Vec<&str> = fields.collect();
    if !rest.contains(&"merge=graphify") {
        return MergeLine::Other;
    }
    let remaining: Vec<&str> = rest
        .into_iter()
        .filter(|f| *f != "merge=graphify")
        .collect();
    if remaining.is_empty() {
        MergeLine::OnlyMerge
    } else {
        MergeLine::Rewritten(format!("{expected_path} {}", remaining.join(" ")))
    }
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

/// True when `.gitattributes` is a symlink; following it could read or write a
/// file outside the repo.
fn attrs_is_symlink(attrs: &Path) -> bool {
    attrs
        .symlink_metadata()
        .is_ok_and(|m| m.file_type().is_symlink())
}

/// Refuse to modify a symlinked `.gitattributes`, so `hook install`/`uninstall`
/// never writes THROUGH the link to a file outside the repo — the same
/// persistent-symlink guard the Obsidian prune uses. graphify-py follows the
/// link (`read_text`/`write_text`); this is a deliberate hardening. Rejected at
/// each function's start, before any git-config mutation, so a symlinked
/// `.gitattributes` never leaves config partially changed. This is a
/// check-then-use guard (the workspace's `path_guard` model), NOT a
/// descriptor-relative atomic one, so a residual TOCTOU race is knowingly
/// accepted; a capability-scoped (`cap-std`) rewrite would close it but the repo
/// depends on no such crate. (Disputes the atomic-handle review finding.)
fn reject_symlinked_attrs(attrs: &Path) -> Result<(), crate::error::HooksError> {
    if attrs_is_symlink(attrs) {
        return Err(crate::error::HooksError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "{} is a symlink; refusing to modify it through the link",
                attrs.display()
            ),
        )));
    }
    Ok(())
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
    let attrs = root.join(".gitattributes");
    reject_symlinked_attrs(&attrs)?;
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
    let attrs = root.join(".gitattributes");
    reject_symlinked_attrs(&attrs)?;
    // Do the fallible `.gitattributes` mutation FIRST: a read/write/delete error
    // aborts here (via `?`) BEFORE any git-config change, so a filesystem failure
    // never leaves config half-unset. The best-effort config unset then always
    // runs below (matching graphify-py's unconditional `--unset`), even when
    // there is no attribute to remove, so a config-only partial registration is
    // still cleaned.
    let msg = if attrs.exists() {
        let content = std::fs::read_to_string(&attrs)?;
        let expected = merge_attr_path();
        let mut changed = false;
        let mut kept: Vec<String> = Vec::with_capacity(content.lines().count());
        for raw in content.lines() {
            match strip_graphify_merge_attr(raw, &expected) {
                MergeLine::Other => kept.push(raw.to_string()),
                MergeLine::Rewritten(rewritten) => {
                    changed = true;
                    kept.push(rewritten);
                }
                MergeLine::OnlyMerge => changed = true,
            }
        }
        if !changed {
            "gitattributes entry not found - nothing to remove.".to_string()
        } else if kept.iter().all(|l| l.trim().is_empty()) {
            std::fs::remove_file(&attrs)?;
            "removed (.gitattributes deleted - no other entries)".to_string()
        } else {
            std::fs::write(&attrs, format!("{}\n", kept.join("\n")))?;
            "removed from .gitattributes (other entries preserved)".to_string()
        }
    } else {
        "not registered - nothing to remove.".to_string()
    };
    for key in ["merge.graphify.name", "merge.graphify.driver"] {
        // Best-effort, matching graphify-py `hooks.py:571-579`: `--unset` runs
        // WITHOUT `check=True` and catches only `OSError`, so every outcome
        // (missing key, launch failure, any nonzero exit) is ignored.
        // (Disputes CodeRabbit's "propagate non-missing-key --unset" finding.)
        let _ = git_config(root, &["--unset", key]);
    }
    Ok(msg)
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
    let attr_ok = attrs.exists()
        && !attrs_is_symlink(&attrs)
        && std::fs::read_to_string(&attrs).is_ok_and(|c| has_merge_attr(&c));
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
