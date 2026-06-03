//! Parity port of `graphify-py/tests/test_cpp_preprocess.py`.
//!
//! The Fortran C-preprocessor path is hardened against argument injection (F5).
//! A corpus file is attacker-named; `cpp` does not accept a `--` end-of-options
//! terminator, so the path passed to `cpp` is resolved to an absolute path which
//! can never be parsed as a `cpp` option.

#![allow(clippy::expect_used)]

use graphify_extract::resolve_cpp_path;

#[test]
fn cpp_preprocess_passes_absolute_path() {
    // Python mocks subprocess.run and asserts argv[-1] is absolute. The Rust
    // seam is `resolve_cpp_path`, which builds that argument; assert it is
    // absolute and never looks like an option for an attacker-named file.
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("-I-weird.F90");
    std::fs::write(&f, "program x\nend program x\n").expect("write fixture");

    let resolved = resolve_cpp_path(&f);
    let s = resolved.to_string_lossy();
    // A POSIX absolute path begins with `/`; this is the property `cpp` relies on
    // to never see the arg as an option. Windows absolute paths look like `C:\…`,
    // so the leading-`/` check is Unix-only.
    #[cfg(unix)]
    assert!(s.starts_with('/'), "path arg must be absolute, got {s:?}");
    assert!(
        !s.starts_with('-'),
        "path arg must never look like an option, got {s:?}"
    );
    assert!(resolved.is_absolute());
}

#[test]
fn resolve_cpp_path_absolute_input_unchanged_shape() {
    // An already-absolute, existing path resolves to an absolute path.
    let tmp = tempfile::tempdir().expect("tempdir");
    let f = tmp.path().join("plain.F90");
    std::fs::write(&f, "program y\nend program y\n").expect("write");
    let resolved = resolve_cpp_path(&f);
    assert!(resolved.is_absolute());
}
