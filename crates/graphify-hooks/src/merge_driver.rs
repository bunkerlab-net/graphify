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

/// The `.gitattributes` line assigning the graphify merge driver to
/// `graph.json`. The graph lives under the configured output dir
/// (`GRAPHIFY_OUT`); gitattributes patterns are repo-relative, so an absolute
/// override (or one with a backslash) cannot be expressed there — fall back to
/// the default name in that case. Mirrors `_merge_attr_line`.
fn merge_attr_line() -> String {
    let raw = std::env::var("GRAPHIFY_OUT").unwrap_or_default();
    let out = if raw.is_empty() || Path::new(&raw).is_absolute() || raw.contains('\\') {
        "graphify-out"
    } else {
        &raw
    };
    format!("{}/graph.json merge=graphify", out.trim_end_matches('/'))
}

/// True if a (non-comment) `<...>graph.json ... merge=graphify` line exists.
/// Mirrors `_has_merge_attr`.
#[must_use]
fn has_merge_attr(content: &str) -> bool {
    content.lines().any(|raw| {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some(first) if first.ends_with("graph.json") => fields.any(|f| f == "merge=graphify"),
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
/// The Rust binary is not a Python interpreter, so — like the git hooks — the
/// driver relies on the `graphify` launcher being on PATH at merge time rather
/// than pinning an interpreter path.
#[must_use]
pub fn register_merge_driver(root: &Path) -> String {
    let driver = "graphify merge-driver %O %A %B";
    for (key, value) in [
        ("merge.graphify.name", "graphify graph.json union merge"),
        ("merge.graphify.driver", driver),
    ] {
        match git_config(root, &[key, value]) {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                return format!(
                    "not registered (git config failed: {})",
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            }
            Err(e) => return format!("not registered (git config failed: {e})"),
        }
    }

    let line = merge_attr_line();
    let attrs = root.join(".gitattributes");
    if attrs.exists() {
        let Ok(content) = std::fs::read_to_string(&attrs) else {
            return "not registered (could not read .gitattributes)".to_string();
        };
        if has_merge_attr(&content) {
            return format!("already registered ({line})");
        }
        // Never clobber other entries; preserve a trailing newline.
        let mut content = content;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&line);
        content.push('\n');
        if std::fs::write(&attrs, content).is_err() {
            return "not registered (could not write .gitattributes)".to_string();
        }
    } else if std::fs::write(&attrs, format!("{line}\n")).is_err() {
        return "not registered (could not write .gitattributes)".to_string();
    }
    format!("registered ({line})")
}

/// Remove the merge-driver git config keys and the `.gitattributes` line.
/// Mirrors `_unregister_merge_driver`.
#[must_use]
pub fn unregister_merge_driver(root: &Path) -> String {
    for key in ["merge.graphify.name", "merge.graphify.driver"] {
        // `--unset` exits nonzero if the key is absent; that is fine.
        let _ = git_config(root, &["--unset", key]);
    }
    let attrs = root.join(".gitattributes");
    if !attrs.exists() {
        return "not registered - nothing to remove.".to_string();
    }
    let Ok(content) = std::fs::read_to_string(&attrs) else {
        return "not registered - nothing to remove.".to_string();
    };
    let kept: Vec<&str> = content.lines().filter(|raw| !has_merge_attr(raw)).collect();
    if kept.len() == content.lines().count() {
        return "gitattributes entry not found - nothing to remove.".to_string();
    }
    if kept.is_empty() {
        let _ = std::fs::remove_file(&attrs);
        return "removed (.gitattributes deleted - no other entries)".to_string();
    }
    let _ = std::fs::write(&attrs, format!("{}\n", kept.join("\n")));
    "removed from .gitattributes (other entries preserved)".to_string()
}

/// Report whether the merge driver is registered (config + gitattributes).
/// Mirrors `_merge_driver_status`.
#[must_use]
pub fn merge_driver_status(root: &Path) -> String {
    let cfg_ok = git_config(root, &["--get", "merge.graphify.driver"])
        .is_ok_and(|o| o.status.success() && !String::from_utf8_lossy(&o.stdout).trim().is_empty());
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
