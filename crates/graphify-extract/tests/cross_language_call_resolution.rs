//! A call in one language must never bind by name to a definition in another
//! language family (bf7fa50).
//!
//! Ports the direct-call cases of
//! `graphify-py/tests/test_cross_language_call_resolution.py`. The cross-file
//! resolver matched raw-call callees against a repo-wide label index with no
//! language check, so a bare Python call bound to a same-named Kotlin `fun`.
//! The fix filters resolution candidates by language interop family; real
//! interop pairs (Kotlin↔Java) still resolve.
//!
//! The `indirect_call` (callback-by-name) cases from the same Python file are
//! ported alongside the direct-call cases now that the `indirect_call` feature
//! exists (#1565/#1566): the family guard applies to indirect dispatch too.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::path::Path;

use graphify_extract::{ExtractOutput, extract};
use serde_json::Value;
use tempfile::tempdir;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn extract_files(
    root: &Path,
    files: &[(&str, &str)],
) -> Result<ExtractOutput, Box<dyn std::error::Error>> {
    let mut paths = Vec::with_capacity(files.len());
    for (name, body) in files {
        let p = root.join(name);
        std::fs::create_dir_all(p.parent().ok_or("file has no parent")?)?;
        std::fs::write(&p, body)?;
        paths.push(p);
    }
    Ok(extract(&paths, Some(root)))
}

fn label_of(out: &ExtractOutput, id: &str) -> String {
    out.nodes
        .iter()
        .find(|n| n.get("id").and_then(Value::as_str) == Some(id))
        .and_then(|n| n.get("label").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

/// Target labels of every `calls` / `indirect_call` edge.
fn call_target_labels(out: &ExtractOutput) -> HashSet<String> {
    out.edges
        .iter()
        .filter(|e| {
            matches!(
                e.get("relation").and_then(Value::as_str),
                Some("calls" | "indirect_call")
            )
        })
        .filter_map(|e| e.get("target").and_then(Value::as_str))
        .map(|t| label_of(out, t))
        .collect()
}

#[test]
fn python_call_does_not_bind_to_kotlin_function() -> TestResult {
    // Direct-call path: a bare Python call must not resolve to the lone same-named
    // Kotlin definition (python and jvm are different interop families).
    let tmp = tempdir()?;
    let out = extract_files(
        tmp.path(),
        &[
            (
                "py/worker.py",
                "def process():\n    return refreshHeading()\n",
            ),
            (
                "android/HeadingSensorBridge.kt",
                "class HeadingSensorBridge {\n    fun refreshHeading() {\n        println(\"native sensor\")\n    }\n}\n",
            ),
        ],
    )?;
    let targets = call_target_labels(&out);
    assert!(
        !targets.iter().any(|t| t.contains("refreshHeading")),
        "Python call bound across language families: {targets:?}"
    );
    Ok(())
}

#[test]
fn jvm_interop_kotlin_call_to_java_still_resolves() -> TestResult {
    // Kotlin and Java share the JVM — same interop family, so a Kotlin call to a
    // Java method must keep resolving exactly as it did before the guard.
    let tmp = tempdir()?;
    let out = extract_files(
        tmp.path(),
        &[
            (
                "Alarm.java",
                "public class Alarm {\n    public static void ring() { System.out.println(\"ring\"); }\n}\n",
            ),
            ("Scheduler.kt", "fun schedule() {\n    ring()\n}\n"),
        ],
    )?;
    let targets = call_target_labels(&out);
    assert!(
        targets.iter().any(|t| t.contains("ring")),
        "Kotlin→Java (same JVM family) call must still resolve: {targets:?}"
    );
    Ok(())
}

#[test]
fn tsx_callback_does_not_bind_to_kotlin_method() -> TestResult {
    // A TSX component passes a callback by name; the only same-named definition
    // repo-wide is a Kotlin method (a different interop family). No edge — direct
    // or indirect — may bind across the boundary.
    let tmp = tempdir()?;
    let out = extract_files(
        tmp.path(),
        &[
            (
                "web/Upcoming.tsx",
                "declare function register(cb: () => void): void;\nexport function UpcomingPanel() {\n  register(refreshHeading);\n  return null;\n}\n",
            ),
            (
                "android/HeadingSensorBridge.kt",
                "class HeadingSensorBridge {\n    fun refreshHeading() {\n        println(\"native sensor\")\n    }\n}\n",
            ),
        ],
    )?;
    let targets = call_target_labels(&out);
    assert!(
        !targets.iter().any(|t| t.contains("refreshHeading")),
        "TSX callback bound across language families: {targets:?}"
    );
    Ok(())
}

#[test]
fn same_language_callback_still_resolves() -> TestResult {
    // Positive control: a TS callback passed by name with a same-language
    // definition and import evidence keeps resolving as an INFERRED indirect_call.
    let tmp = tempdir()?;
    let out = extract_files(
        tmp.path(),
        &[
            (
                "a.ts",
                "import { refreshHeading } from \"./b\";\ndeclare function register(cb: () => void): void;\nexport function run() { register(refreshHeading); }\n",
            ),
            ("b.ts", "export function refreshHeading(): void {}\n"),
        ],
    )?;
    let confidences: Vec<String> = out
        .edges
        .iter()
        .filter(|e| e.get("relation").and_then(Value::as_str) == Some("indirect_call"))
        .filter(|e| {
            e.get("target")
                .and_then(Value::as_str)
                .is_some_and(|t| label_of(&out, t).contains("refreshHeading"))
        })
        .filter_map(|e| {
            e.get("confidence")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    assert!(
        !confidences.is_empty(),
        "same-language indirect callback dropped"
    );
    assert_eq!(confidences[0], "INFERRED");
    Ok(())
}
