/// Build script: embeds the current git short SHA as the `GIT_SHORT_SHA` env
/// var so `graphify --version` can render `<crate-version>-<short-sha>`.
/// Falls back to `"unknown"` when the git directory or refs are missing.
///
/// Reads ref files directly from the filesystem instead of spawning an external
/// `git` process. Handles plain repos, submodules, and worktrees by resolving
/// the real git dir from the `.git` pointer file when necessary.
fn main() {
    let git_dir = resolve_git_dir();
    println!(
        "cargo:rustc-env=GIT_SHORT_SHA={}",
        git_dir
            .as_deref()
            .and_then(read_short_sha)
            .unwrap_or_else(|| "unknown".to_owned())
    );
    let watch_dir = git_dir.unwrap_or_else(|| std::path::PathBuf::from(".git"));
    println!(
        "cargo:rerun-if-changed={}",
        watch_dir.join("HEAD").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        watch_dir.join("refs/heads").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        watch_dir.join("packed-refs").display()
    );
}

/// Resolve the real git directory for plain repos (`.git/` dir), submodules,
/// and worktrees (`.git` file containing `gitdir: <path>`).
fn resolve_git_dir() -> Option<std::path::PathBuf> {
    let git_path = std::path::Path::new(".git");
    if git_path.is_dir() {
        return Some(git_path.to_path_buf());
    }
    if git_path.is_file() {
        let content = std::fs::read_to_string(git_path).ok()?;
        let target = content.trim().strip_prefix("gitdir: ")?;
        let resolved = std::path::Path::new(target.trim());
        if resolved.is_dir() {
            return Some(resolved.to_path_buf());
        }
    }
    None
}

fn is_hex_sha(candidate: &str) -> bool {
    !candidate.is_empty() && candidate.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Read HEAD from `git_dir` and return the first 7 hex characters of the current
/// SHA, or `None` if HEAD is missing, unresolvable, or contains non-hex content.
fn read_short_sha(git_dir: &std::path::Path) -> Option<String> {
    if !git_dir.is_dir() {
        return None;
    }

    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();

    let full_sha = if let Some(ref_path) = head.strip_prefix("ref: ") {
        resolve_ref(git_dir, ref_path.trim())?
    } else {
        if !is_hex_sha(head) {
            return None;
        }
        head.to_owned()
    };

    if full_sha.len() < 7 || !is_hex_sha(&full_sha) {
        return None;
    }

    Some(full_sha[..7].to_owned())
}

fn resolve_ref(git_dir: &std::path::Path, ref_name: &str) -> Option<String> {
    if ref_name.is_empty() {
        return None;
    }

    if let Ok(content) = std::fs::read_to_string(git_dir.join(ref_name)) {
        let sha = content.trim();
        if is_hex_sha(sha) {
            return Some(sha.to_owned());
        }
    }

    let packed = std::fs::read_to_string(git_dir.join("packed-refs")).ok()?;
    for line in packed.lines() {
        if line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        if let Some((sha, name)) = line.split_once(' ')
            && name.trim() == ref_name
        {
            let sha = sha.trim();
            if is_hex_sha(sha) {
                return Some(sha.to_owned());
            }
        }
    }

    None
}
