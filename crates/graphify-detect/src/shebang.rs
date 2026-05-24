//! Shebang-line interpreter resolution.
//!
//! Ports the new `_shebang_interpreter` / `_env_command_args` / `_split_env_s`
//! helpers added in `graphify-py/graphify/detect.py`. The parser handles
//! macOS/BSD short flags, GNU coreutils long flags, packed `-S` / `-vS` /
//! `--split-string` payloads, inline `NAME=value` assignments, and quoted
//! interpreter paths.

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Tokenise a `#!` line via POSIX-style shell splitting and resolve the
/// effective interpreter basename.
///
/// Returns `None` when there is no shebang, the file cannot be read, the
/// shebang line has no tokens, or `env(1)` option parsing rejects the
/// argument list. Reads up to the first 256 bytes to cover long shebangs
/// (Linux truncates at 128, macOS at 512).
#[must_use]
pub fn shebang_interpreter(path: &Path) -> Option<String> {
    let mut buf = [0u8; 256];
    let n = {
        let mut f = File::open(path).ok()?;
        f.read(&mut buf).ok()?
    };
    let first = &buf[..n];
    if !first.starts_with(b"#!") {
        return None;
    }
    let line_bytes = first.split(|&b| b == b'\n').next()?;
    let line = String::from_utf8_lossy(line_bytes);
    let stripped = line.get(2..)?.trim();
    let parts = shlex::split(stripped)?;
    if parts.is_empty() {
        return None;
    }
    let raw = parts.first()?;
    let mut interp = basename(raw);
    if interp == "env" {
        let env_args = env_command_args(&parts[1..], true);
        if env_args.is_empty() {
            return None;
        }
        interp = basename(&env_args[0]);
    }
    Some(interp)
}

/// Take an `env(1)` argument vector (everything after `env` in the
/// shebang) and return the trailing command argv after stripping
/// options, variable assignments, and recursively-unpacked
/// `-S` / `--split-string` payloads.
///
/// Mirrors `_env_command_args` in `graphify-py/graphify/detect.py`.
/// Setting `allow_split` to `false` rejects nested split-string payloads
/// to bound recursion (the outermost call must already have consumed
/// any `-S`).
#[must_use]
pub fn env_command_args(args: &[String], allow_split: bool) -> Vec<String> {
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();

        if arg == "--" {
            return args[i + 1..].to_vec();
        }

        if allow_split && let Some(unpacked) = split_string_dispatch(arg, args, i) {
            return env_command_args(&unpacked, false);
        }

        // Options with separate required operand.
        if matches!(
            arg,
            "-u" | "-C" | "-P" | "-a" | "--unset" | "--chdir" | "--argv0"
        ) {
            if i + 2 > args.len() {
                return Vec::new();
            }
            i += 2;
            continue;
        }

        // Clumped short option + operand: e.g. `-uPYTHONPATH`.
        if (arg.starts_with("-u")
            || arg.starts_with("-C")
            || arg.starts_with("-P")
            || arg.starts_with("-a"))
            && arg.len() > 2
            && !arg.starts_with("--")
        {
            i += 1;
            continue;
        }

        // Long option with `=` operand.
        if arg.starts_with("--unset=") || arg.starts_with("--chdir=") || arg.starts_with("--argv0=")
        {
            i += 1;
            continue;
        }

        // No-operand flags.
        if matches!(
            arg,
            "-" | "-i"
                | "-0"
                | "-v"
                | "--ignore-environment"
                | "--null"
                | "--debug"
                | "--list-signal-handling"
        ) {
            i += 1;
            continue;
        }

        // Signal-handling long flags (with or without `=SIG` operand).
        if arg.starts_with("--default-signal")
            || arg.starts_with("--ignore-signal")
            || arg.starts_with("--block-signal")
        {
            i += 1;
            continue;
        }

        // Unknown hyphen-prefixed argument: refuse to guess whether
        // the next token is an interpreter or an operand.
        if arg.starts_with('-') {
            return Vec::new();
        }

        // Inline `NAME=value` env assignment.
        if arg.contains('=') {
            i += 1;
            continue;
        }

        // First non-option, non-assignment token starts the command argv.
        return args[i..].to_vec();
    }
    Vec::new()
}

/// Match the various `-S` / `-vS` / `--split-string` spellings against
/// the current arg and, on match, return the re-tokenised packed payload
/// joined with any trailing args.
///
/// Returns `None` when the current arg is not a split-string spelling.
fn split_string_dispatch(arg: &str, args: &[String], i: usize) -> Option<Vec<String>> {
    if arg == "-S" {
        if i + 1 >= args.len() {
            return Some(Vec::new());
        }
        return Some(split_env_s(&args[i + 1..].join(" "), &[]));
    }
    if let Some(payload) = arg.strip_prefix("-S")
        && !payload.is_empty()
    {
        return Some(split_env_s(payload, &args[i + 1..]));
    }
    if arg == "-vS" {
        if i + 1 >= args.len() {
            return Some(Vec::new());
        }
        return Some(split_env_s(&args[i + 1..].join(" "), &[]));
    }
    if let Some(payload) = arg.strip_prefix("-vS")
        && !payload.is_empty()
    {
        return Some(split_env_s(payload, &args[i + 1..]));
    }
    if let Some(payload) = arg.strip_prefix("--split-string=") {
        return Some(split_env_s(payload, &args[i + 1..]));
    }
    if arg == "--split-string" {
        if i + 1 >= args.len() {
            return Some(Vec::new());
        }
        return Some(split_env_s(&args[i + 1], &args[i + 2..]));
    }
    None
}

/// Re-tokenise an `env -S` / `--split-string` packed payload, prepending
/// the operand to any trailing args. Mirrors `_split_env_s` in
/// `graphify-py/graphify/detect.py`.
fn split_env_s(value: &str, rest: &[String]) -> Vec<String> {
    let mut packed = value.to_owned();
    for r in rest {
        packed.push(' ');
        packed.push_str(r);
    }
    let trimmed = packed.trim();
    shlex::split(trimmed).unwrap_or_default()
}

/// Return the basename of a path-like interpreter string. Equivalent to
/// `pathlib.Path(s).name` for the slash-separated shebang strings we see.
fn basename(s: &str) -> String {
    Path::new(s)
        .file_name()
        .and_then(|n| n.to_str())
        .map_or_else(|| s.to_owned(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn env_command_args_passthrough() {
        assert_eq!(
            env_command_args(&split(&["python3"]), true),
            split(&["python3"])
        );
    }

    #[test]
    fn env_command_args_skips_split_string_flag() {
        let r = env_command_args(&split(&["-S", "python3 -u"]), true);
        assert_eq!(r, split(&["python3", "-u"]));
    }

    #[test]
    fn env_command_args_clumped_s() {
        let r = env_command_args(&split(&["-Spython3 -u"]), true);
        assert_eq!(r, split(&["python3", "-u"]));
    }

    #[test]
    fn env_command_args_long_split_string() {
        let r = env_command_args(&split(&["--split-string=python3 -u"]), true);
        assert_eq!(r, split(&["python3", "-u"]));
    }

    #[test]
    fn env_command_args_skips_assignment() {
        let r = env_command_args(&split(&["DEBUG=1", "python3"]), true);
        assert_eq!(r, split(&["python3"]));
    }

    #[test]
    fn env_command_args_skips_unset_operand() {
        let r = env_command_args(&split(&["-u", "PYTHONPATH", "python3"]), true);
        assert_eq!(r, split(&["python3"]));
    }

    #[test]
    fn env_command_args_skips_clumped_unset() {
        let r = env_command_args(&split(&["-uPYTHONPATH", "python3"]), true);
        assert_eq!(r, split(&["python3"]));
    }

    #[test]
    fn env_command_args_unknown_flag_returns_empty() {
        assert!(env_command_args(&split(&["--what", "python3"]), true).is_empty());
    }

    #[test]
    fn env_command_args_double_dash_terminates() {
        let r = env_command_args(&split(&["--", "--weird"]), true);
        assert_eq!(r, split(&["--weird"]));
    }
}
