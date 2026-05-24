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
