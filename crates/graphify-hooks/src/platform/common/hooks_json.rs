//! Builders for the JSON hook entries written into per-platform settings files.

use serde_json::Value;

use crate::HooksError;

use super::markdown::{READ_SETTINGS_HOOK_MATCHER, SETTINGS_HOOK_MATCHER};

/// Build the Claude Code `PreToolUse` hook JSON entry.
///
/// The hook intercepts `Bash` tool calls and, when the user is running a
/// search command like `grep`/`rg`/`find`, prepends a reminder pointing the
/// agent at `graphify-out/` if a graph exists.
pub(in crate::platform) fn settings_hook() -> Value {
    serde_json::json!({
        "matcher": "Bash",
        "hooks": [
            {
                "type": "command",
                "command": concat!(
                    "CMD=$(python3 -c \"",
                    "import json,sys; d=json.load(sys.stdin); ",
                    "print(d.get('tool_input',d).get('command',''))\" 2>/dev/null || true); ",
                    "case \"$CMD\" in ",
                    r"*grep*|*rg\ *|*ripgrep*|*find\ *|*fd\ *|*ack\ *|*ag\ *) ",
                    "  [ -f graphify-out/graph.json ] && ",
                    r#"  echo '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"graphify: knowledge graph at graphify-out/. For focused questions, run `graphify query \"<question>\"` (scoped subgraph, usually much smaller than GRAPH_REPORT.md) instead of grepping raw files. Read GRAPH_REPORT.md only for broad architecture context."}}' "#,
                    "  || true ;; ",
                    "esac"
                )
            }
        ]
    })
}

/// Build the Claude Code Read/Glob `PreToolUse` hook JSON entry (#1114).
///
/// The Bash search hook never sees a file read through the native Read tool or
/// a Glob — the most common way an agent skips the graph (answering a codebase
/// question by reading source files one by one). This hook matches `Read|Glob`,
/// inspects the target path, and nudges (never blocks) only for a source/doc
/// file outside `graphify-out/` when a graph exists. The parser is `python3`,
/// the shell is POSIX, and every branch fails open, so a legitimate read always
/// goes through. Reading the graph's own report under `graphify-out/` is
/// suppressed so it never starts a feedback loop. The command is byte-identical
/// to the Python reference so the rendered settings file matches exactly.
///
/// The command is deliberately kept as one whole literal rather than composed
/// from fragments (a reviewer suggested decomposing it): it must stay
/// byte-for-byte identical to graphify-py's `_READ_SETTINGS_HOOK["command"]`,
/// and a single literal makes that correspondence verifiable at a glance. Its
/// runtime behaviour is validated by `tests/read_hook.rs`, which executes it via
/// `sh -c` against crafted stdin.
#[must_use]
pub(in crate::platform) fn read_settings_hook() -> Value {
    serde_json::json!({
        "matcher": "Read|Glob",
        "hooks": [
            {
                "type": "command",
                "command": r#"HIT=$(python3 -c "import json,sys;d=json.load(sys.stdin);t=d.get('tool_input',d);s=(str(t.get('file_path') or '')+' '+str(t.get('pattern') or '')+' '+str(t.get('path') or '')).lower().replace(chr(92),'/');exts=('.py','.js','.ts','.tsx','.jsx','.go','.rs','.java','.rb','.c','.h','.cpp','.hpp','.cc','.cs','.kt','.swift','.php','.scala','.lua','.sh','.md','.rst','.txt','.mdx');sys.stdout.write('1' if 'graphify-out/' not in s and any(e in s for e in exts) else '')" 2>/dev/null || true); if [ "$HIT" = 1 ] && [ -f graphify-out/graph.json ]; then echo '{"hookSpecificOutput":{"hookEventName":"PreToolUse","additionalContext":"graphify: knowledge graph at graphify-out/. For codebase questions, run `graphify query \"<question>\"` (scoped subgraph, usually much smaller than reading files one by one), `graphify explain \"<concept>\"`, or `graphify path \"<A>\" \"<B>\"`, instead of reading source files to answer. Read raw files to modify or debug specific code, or when the graph lacks the detail."}}'; fi || true"#
            }
        ]
    })
}

/// True when a `PreToolUse` entry's nested `hooks[].command` mentions graphify.
///
/// Inspecting the command strings (not the whole serialized entry via
/// `to_string()`) avoids a stray match on an unrelated field that merely
/// contains the substring "graphify", mirroring the precise matching the
/// install path uses.
pub(in crate::platform) fn hook_targets_graphify(hook: &Value) -> bool {
    hook.get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|steps| {
            steps.iter().any(|step| {
                step.get("command")
                    .and_then(Value::as_str)
                    .is_some_and(|c| c.contains("graphify"))
            })
        })
}

/// Drop any stale graphify `PreToolUse` entries from `settings` and append the
/// current Bash-search + Read/Glob hooks.
///
/// Shared by Claude Code and `CodeBuddy`: both register the byte-identical
/// `settings_hook()` + `read_settings_hook()` pair, differing only in which
/// settings file they write. Two hooks are appended: the Bash search nudge and
/// the Read/Glob nudge (#1114).
///
/// # Errors
///
/// Returns `HooksError::Json` when `settings` is not a JSON object (or its
/// `hooks` member is some non-object value the installer cannot extend).
pub(in crate::platform) fn register_pretooluse_hooks(
    settings: &mut Value,
) -> Result<(), HooksError> {
    let hooks = settings
        .as_object_mut()
        .and_then(|o| {
            o.entry("hooks")
                .or_insert_with(|| Value::Object(serde_json::Map::new()))
                .as_object_mut()
        })
        .ok_or_else(|| HooksError::Json("hooks is not an object".to_string()))?;

    let pre_tool = hooks
        .entry("PreToolUse")
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(arr) = pre_tool {
        arr.retain(|h| {
            let matcher = h.get("matcher").and_then(Value::as_str).unwrap_or("");
            let is_stale_matcher = matcher == "Glob|Grep"
                || matcher == SETTINGS_HOOK_MATCHER
                || matcher == READ_SETTINGS_HOOK_MATCHER;
            !(is_stale_matcher && hook_targets_graphify(h))
        });
        arr.push(settings_hook());
        arr.push(read_settings_hook());
    }
    Ok(())
}

/// Remove stale graphify `PreToolUse` entries from `settings` in place.
///
/// Returns `true` when at least one entry was removed (so callers know whether
/// the settings file needs rewriting). Shared by Claude Code and `CodeBuddy`.
pub(in crate::platform) fn remove_pretooluse_hooks(settings: &mut Value) -> bool {
    let Some(arr) = settings
        .pointer_mut("/hooks/PreToolUse")
        .and_then(Value::as_array_mut)
    else {
        return false;
    };
    let before = arr.len();
    arr.retain(|h| {
        let matcher = h.get("matcher").and_then(Value::as_str).unwrap_or("");
        let is_stale = matcher == "Glob|Grep"
            || matcher == SETTINGS_HOOK_MATCHER
            || matcher == READ_SETTINGS_HOOK_MATCHER;
        !(is_stale && hook_targets_graphify(h))
    });
    arr.len() != before
}

/// Build the Gemini CLI `BeforeTool` hook JSON entry.
///
/// The hook intercepts `read_file` and `list_directory` tool calls and
/// injects a reminder pointing the agent at `graphify-out/` if a graph
/// exists.
pub(in crate::platform) fn gemini_hook() -> Value {
    serde_json::json!({
        "matcher": "read_file|list_directory",
        "hooks": [
            {
                "type": "command",
                "command": concat!(
                    "python -c \"",
                    "import sys,pathlib,json;",
                    "e=pathlib.Path('graphify-out/graph.json').exists();",
                    "d={'decision':'allow'};",
                    r#"e and d.update({'additionalContext':'graphify: knowledge graph at graphify-out/. For focused questions, run `graphify query "<question>"` (scoped subgraph, usually much smaller than GRAPH_REPORT.md) instead of grepping raw files. Read GRAPH_REPORT.md only for broad architecture context.'});"#,
                    "sys.stdout.write(json.dumps(d))",
                    "\""
                )
            }
        ]
    })
}
