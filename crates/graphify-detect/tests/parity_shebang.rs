//! Parity tests for shebang interpreter resolution.
//!
//! Mirrors the `_shebang_interpreter` / shebang-driven `classify_file`
//! tests added in `graphify-py/tests/test_detect.py`.
#![allow(clippy::expect_used)]

use std::fs;

use graphify_detect::{FileType, classify_file, env_command_args, shebang_interpreter};
use tempfile::tempdir;

fn write(name: &str, contents: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join(name);
    fs::write(&path, contents).expect("write");
    (dir, path)
}

#[test]
fn shebang_interpreter_plain() {
    let (_d, p) = write("plain", b"#!/usr/bin/python3\nprint('x')\n");
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_single_arg() {
    let (_d, p) = write("env_single", b"#!/usr/bin/env python3\nprint('x')\n");
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_dash_s() {
    let (_d, p) = write("env_dashs", b"#!/usr/bin/env -S python3 -u\nprint('x')\n");
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_with_flags() {
    let (_d, p) = write("env_flags", b"#!/usr/bin/env -i bash\necho hi\n");
    assert_eq!(shebang_interpreter(&p), Some("bash".to_owned()));
}

#[test]
fn shebang_interpreter_env_with_assignment() {
    let (_d, p) = write(
        "env_assign",
        b"#!/usr/bin/env DEBUG=1 python3\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_no_shebang() {
    let (_d, p) = write("no_shebang", b"print('x')\n");
    assert_eq!(shebang_interpreter(&p), None);
}

#[test]
fn shebang_interpreter_quoted_path() {
    let (_d, p) = write("quoted", b"#!\"/usr/local/bin/python3\"\nprint('x')\n");
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_file_type_classifies_via_interpreter() {
    let (_d, p) = write("tool", b"#!/usr/bin/env -S python3 -u\nprint('x')\n");
    assert_eq!(classify_file(&p), Some(FileType::Code));
}

#[test]
fn shebang_interpreter_unreadable_returns_none() {
    let dir = tempdir().expect("tempdir");
    let missing = dir.path().join("does_not_exist");
    assert_eq!(shebang_interpreter(&missing), None);
}

#[test]
fn shebang_interpreter_env_unset_with_operand() {
    let (_d, p) = write(
        "env_unset",
        b"#!/usr/bin/env -u PYTHONPATH python3\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
    assert_eq!(classify_file(&p), Some(FileType::Code));
}

#[test]
fn shebang_interpreter_env_chdir_with_operand() {
    let (_d, p) = write("env_chdir", b"#!/usr/bin/env -C /tmp python3\nprint('x')\n");
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_path_with_operand() {
    let (_d, p) = write("env_path", b"#!/usr/bin/env -P /bin python3\nprint('x')\n");
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_dash_s_after_flag() {
    let (_d, p) = write(
        "env_flag_dash_s",
        b"#!/usr/bin/env -i -S \"python3 -u\"\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_clumped_u_operand() {
    let (_d, p) = write(
        "env_clumped",
        b"#!/usr/bin/env -uPYTHONPATH python3\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_missing_operand_returns_none() {
    let (_d, p) = write("env_missing_op", b"#!/usr/bin/env -u\n");
    assert_eq!(shebang_interpreter(&p), None);
}

#[test]
fn shebang_interpreter_env_gnu_split_string_equals() {
    let (_d, p) = write(
        "env_split_eq",
        b"#!/usr/bin/env --split-string='python3 -u'\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_gnu_split_string_separate() {
    let (_d, p) = write(
        "env_split_sep",
        b"#!/usr/bin/env --split-string \"python3 -u\"\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_gnu_argv0_operand() {
    let (_d, p) = write(
        "env_argv0",
        b"#!/usr/bin/env -a alias python3\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_compact_dash_s() {
    let (_d, p) = write(
        "env_compact_dash_s",
        b"#!/usr/bin/env -Spython3 -u\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_compact_v_then_s() {
    let (_d, p) = write(
        "env_compact_vs",
        b"#!/usr/bin/env -vSpython3 -u\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_long_unset_separate_operand() {
    let (_d, p) = write(
        "env_long_unset",
        b"#!/usr/bin/env --unset PYTHONPATH python3\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_long_unset_equals() {
    let (_d, p) = write(
        "env_long_unset_eq",
        b"#!/usr/bin/env --unset=PYTHONPATH python3\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_long_chdir_separate_operand() {
    let (_d, p) = write(
        "env_long_chdir",
        b"#!/usr/bin/env --chdir /tmp python3\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_long_chdir_equals() {
    let (_d, p) = write(
        "env_long_chdir_eq",
        b"#!/usr/bin/env --chdir=/tmp python3\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_signal_flags() {
    let (_d, p) = write(
        "env_signal",
        b"#!/usr/bin/env --default-signal=TERM --ignore-signal=PIPE python3\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_unknown_option_returns_none() {
    let (_d, p) = write("env_unknown", b"#!/usr/bin/env --no-such-flag python3\n");
    assert_eq!(shebang_interpreter(&p), None);
}

#[test]
fn shebang_interpreter_env_dash_s_assignment_before_interpreter() {
    let (_d, p) = write(
        "env_s_assignment",
        b"#!/usr/bin/env -S PYTHONPATH=/opt/custom python3\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_dash_s_flag_before_interpreter() {
    let (_d, p) = write("env_s_flag", b"#!/usr/bin/env -S -i python3\nprint('x')\n");
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_long_split_assignment_before_interpreter() {
    let (_d, p) = write(
        "env_long_split_assignment",
        b"#!/usr/bin/env --split-string='PYTHONPATH=/opt/custom python3 -u'\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_long_split_flag_before_interpreter() {
    let (_d, p) = write(
        "env_long_split_flag",
        b"#!/usr/bin/env --split-string='-i python3 -u'\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn shebang_interpreter_env_nested_split_string_rejected() {
    let (_d, p) = write(
        "env_nested_split",
        b"#!/usr/bin/env -S -S python3 -u\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), None);
}

#[test]
fn shebang_interpreter_env_vs_assignment_before_interpreter() {
    let (_d, p) = write(
        "env_vs_assignment",
        b"#!/usr/bin/env -vS DEBUG=1 python3 -u\nprint('x')\n",
    );
    assert_eq!(shebang_interpreter(&p), Some("python3".to_owned()));
}

#[test]
fn classify_file_ets_extension() {
    use std::path::Path;
    assert_eq!(
        classify_file(Path::new("foo.ets")),
        Some(FileType::Code),
        ".ets (ArkTS / HarmonyOS) should be classified as code",
    );
}

// ---------------------------------------------------------------------------
// env_command_args — unit tests for the env(1) argv parser. These were
// originally inline `#[cfg(test)] mod tests` in `src/shebang.rs`; moved to
// the integration test file to match the project convention that all tests
// live under `tests/` (see CLAUDE.md).
// ---------------------------------------------------------------------------

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
