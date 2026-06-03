//! Builders for the JSON hook entries written into per-platform settings files.

use serde_json::Value;

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
