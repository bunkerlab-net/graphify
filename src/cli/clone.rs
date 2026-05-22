//! `clone` command — clone a GitHub repo locally and print its path.

use std::path::PathBuf;

use anyhow::Result;

/// Clone a GitHub URL into a local cache directory, or pull if already present.
///
/// Mirrors `_clone_repo` from `__main__.py:1139`. When `target` already exists
/// as a directory, `git -C <target> pull` is run instead of `git clone`, so
/// repeated calls on the same URL reuse the existing checkout.
pub(crate) fn cmd_clone(
    url: &str,
    branch: Option<&str>,
    out: Option<&std::path::Path>,
) -> Result<()> {
    use std::process::Command as Proc;
    let target = out.map_or_else(
        || {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let safe = url
                .replace("https://", "")
                .replace("http://", "")
                .replace('/', "_");
            PathBuf::from(home)
                .join(".graphify")
                .join("repos")
                .join(safe)
        },
        std::path::Path::to_path_buf,
    );
    if target.exists() {
        // Repo already present — pull to update. Mirrors Python's
        // `git -C <dest> pull [origin -- <branch>]` at __main__.py:1174.
        eprintln!(
            "repo already cloned at {} — pulling latest ...",
            target.display()
        );
        let mut cmd = Proc::new("git");
        cmd.arg("-C").arg(&target).arg("pull");
        if let Some(b) = branch {
            cmd.arg("origin").arg("--").arg(b);
        }
        let status = cmd.status()?;
        if !status.success() {
            eprintln!("warning: git pull failed (exit {status}); local copy may be stale");
        }
    } else {
        eprintln!("cloning {url} → {} ...", target.display());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut cmd = Proc::new("git");
        cmd.arg("clone").arg("--depth").arg("1");
        if let Some(b) = branch {
            cmd.arg("--branch").arg(b);
        }
        cmd.arg("--").arg(url).arg(&target);
        let status = cmd.status()?;
        if !status.success() {
            anyhow::bail!("git clone failed with status {status}");
        }
    }
    println!("{}", target.display());
    Ok(())
}
