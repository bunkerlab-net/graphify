//! Integration tests for the MCP stdio server dispatch loop.

#![allow(clippy::expect_used, clippy::items_after_statements)]

use std::fs;

use graphify_serve::server::run_server;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Write a minimal graph to `graphify-out/graph.json` inside `dir`.
fn write_graph(dir: &std::path::Path) -> std::path::PathBuf {
    let out = dir.join("graphify-out");
    fs::create_dir_all(&out).expect("create_dir_all");
    let graph = json!({
        "nodes": [
            {"id": "n1", "label": "alpha", "source_file": "a.py", "community": 0},
            {"id": "n2", "label": "beta", "source_file": "b.py", "community": 0},
        ],
        "edges": [
            {"source": "n1", "target": "n2", "relation": "calls", "confidence": "EXTRACTED"}
        ]
    });
    let path = out.join("graph.json");
    fs::write(&path, serde_json::to_string(&graph).expect("write fixture"))
        .expect("test invariant");
    path
}

/// Write a single distinctly-labelled node graph to `dir/graphify-out/graph.json`.
fn write_graph_labeled(dir: &std::path::Path, id: &str, label: &str) -> std::path::PathBuf {
    let out = dir.join("graphify-out");
    fs::create_dir_all(&out).expect("create_dir_all");
    let graph = json!({
        "nodes": [{"id": id, "label": label, "source_file": "x.py", "community": 0}],
        "edges": []
    });
    let path = out.join("graph.json");
    fs::write(&path, serde_json::to_string(&graph).expect("fixture")).expect("write");
    path
}

/// Run the server with a scripted input and return all parsed JSON-RPC responses.
async fn run_with_input(graph_path: &std::path::Path, input: &str) -> Vec<Value> {
    // Use two separate duplex streams: one for input (test → server), one for output.
    let (mut test_input_writer, server_input_reader) = tokio::io::duplex(8192);
    let (server_output_writer, mut test_output_reader) = tokio::io::duplex(65_536);

    test_input_writer
        .write_all(input.as_bytes())
        .await
        .expect("test invariant");
    // Drop the writer so the server sees EOF.
    drop(test_input_writer);

    let server_path = graph_path.to_string_lossy().into_owned();
    let server_handle = tokio::spawn(async move {
        run_server(server_input_reader, server_output_writer, &server_path)
            .await
            .expect("test invariant");
    });

    let mut buf = String::new();
    test_output_reader
        .read_to_string(&mut buf)
        .await
        .expect("test invariant");
    server_handle.await.expect("test invariant");

    buf.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[tokio::test]
async fn server_responds_to_initialize() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n";
    let responses = run_with_input(&gp, input).await;
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["id"], 1);
    assert!(responses[0]["result"]["protocolVersion"].is_string());
}

#[tokio::test]
async fn server_lists_tools() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n";
    let responses = run_with_input(&gp, input).await;
    let tools = responses[0]["result"]["tools"]
        .as_array()
        .expect("array field");
    assert_ne!(tools.len(), 0);
    assert!(
        tools
            .iter()
            .any(|t| t["name"].as_str() == Some("query_graph"))
    );
}

#[tokio::test]
async fn server_calls_tool_get_node() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\
        \"params\":{\"name\":\"get_node\",\"arguments\":{\"label\":\"alpha\"}}}\n";
    let responses = run_with_input(&gp, input).await;
    let text = responses[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("test invariant");
    assert!(text.contains("alpha"));
}

#[tokio::test]
async fn server_unknown_tool_returns_error_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\
        \"params\":{\"name\":\"no_such_tool\",\"arguments\":{}}}\n";
    let responses = run_with_input(&gp, input).await;
    let text = responses[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("test invariant");
    assert!(text.contains("Unknown tool"));
}

#[tokio::test]
async fn server_lists_resources() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"resources/list\"}\n";
    let responses = run_with_input(&gp, input).await;
    let resources = responses[0]["result"]["resources"]
        .as_array()
        .expect("array field");
    assert_ne!(resources.len(), 0);
}

#[tokio::test]
async fn server_reads_graphify_stats_resource() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"resources/read\",\
        \"params\":{\"uri\":\"graphify://stats\"}}\n";
    let responses = run_with_input(&gp, input).await;
    let text = responses[0]["result"]["contents"][0]["text"]
        .as_str()
        .expect("test invariant");
    assert!(text.contains("Nodes:"));
}

#[tokio::test]
async fn server_unknown_resource_returns_error_text() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"resources/read\",\
        \"params\":{\"uri\":\"graphify://nope\"}}\n";
    let responses = run_with_input(&gp, input).await;
    let text = responses[0]["result"]["contents"][0]["text"]
        .as_str()
        .expect("test invariant");
    assert!(text.contains("Unknown resource"));
}

#[tokio::test]
async fn server_unknown_method_returns_error() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"frobnicate\"}\n";
    let responses = run_with_input(&gp, input).await;
    assert_eq!(responses[0]["error"]["code"], -32_601);
}

#[tokio::test]
async fn server_notification_returns_no_reply() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    // No `id` means notification → must not get a reply.
    let input = "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n";
    let responses = run_with_input(&gp, input).await;
    assert!(responses.is_empty(), "notification must not be replied to");
}

#[tokio::test]
async fn server_blank_and_malformed_lines_skipped() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input =
        "\n   \n{not valid json\n{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"initialize\"}\n";
    let responses = run_with_input(&gp, input).await;
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["id"], 9);
}

#[tokio::test]
async fn server_calls_graph_stats_tool() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":10,\"method\":\"tools/call\",\
        \"params\":{\"name\":\"graph_stats\",\"arguments\":{}}}\n";
    let responses = run_with_input(&gp, input).await;
    let text = responses[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("test invariant");
    assert!(text.contains("Nodes:"));
}

#[tokio::test]
async fn server_calls_god_nodes_tool() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"tools/call\",\
        \"params\":{\"name\":\"god_nodes\",\"arguments\":{\"top_n\":5}}}\n";
    let responses = run_with_input(&gp, input).await;
    let text = responses[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("test invariant");
    assert!(text.starts_with("God nodes"));
}

#[tokio::test]
async fn server_tools_carry_optional_project_path() {
    // #1594: every tool schema gains an optional `project_path` string.
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":20,\"method\":\"tools/list\"}\n";
    let responses = run_with_input(&gp, input).await;
    let tools = responses[0]["result"]["tools"].as_array().expect("array");
    assert_ne!(tools.len(), 0);
    for t in tools {
        assert_eq!(
            t["inputSchema"]["properties"]["project_path"]["type"].as_str(),
            Some("string"),
            "tool {} missing optional project_path",
            t["name"]
        );
        let required = t["inputSchema"]["required"].as_array();
        assert!(
            required.is_none_or(|r| !r.iter().any(|v| v.as_str() == Some("project_path"))),
            "tool {} must not list the optional project_path as required",
            t["name"]
        );
    }
}

#[tokio::test]
async fn server_project_path_routes_to_that_projects_graph() {
    // #1594: project_path routes a call to that project's graph; omitting it
    // hits the default graph the server was started with.
    let tmp1 = tempfile::tempdir().expect("tempdir");
    let default_gp = write_graph(tmp1.path()); // default graph has node "alpha"
    let tmp2 = tempfile::tempdir().expect("tempdir");
    write_graph_labeled(tmp2.path(), "g1", "gamma");
    let proj = tmp2.path().to_string_lossy().into_owned();
    let input = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":21,\"method\":\"tools/call\",\"params\":{{\"name\":\"get_node\",\"arguments\":{{\"label\":\"gamma\",\"project_path\":{proj:?}}}}}}}\n{{\"jsonrpc\":\"2.0\",\"id\":22,\"method\":\"tools/call\",\"params\":{{\"name\":\"get_node\",\"arguments\":{{\"label\":\"alpha\"}}}}}}\n"
    );
    let responses = run_with_input(&default_gp, &input).await;
    let text_of = |id: i64| -> String {
        responses
            .iter()
            .find(|r| r["id"] == id)
            .map(|r| {
                r["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or_default()
    };
    assert!(
        text_of(21).contains("gamma"),
        "project_path routed: {}",
        text_of(21)
    );
    assert!(
        text_of(22).contains("alpha"),
        "default graph: {}",
        text_of(22)
    );
}

#[tokio::test]
async fn server_bad_project_path_errors_without_killing_server() {
    // #1594: a bad project_path yields a tool error, not a crash — the server
    // still answers the next call against the default graph.
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let missing = tmp.path().join("no-such-project");
    let missing_json = serde_json::to_string(&missing.to_string_lossy()).expect("json");
    let input = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":23,\"method\":\"tools/call\",\"params\":{{\"name\":\"get_node\",\"arguments\":{{\"label\":\"alpha\",\"project_path\":{missing_json}}}}}}}\n{{\"jsonrpc\":\"2.0\",\"id\":24,\"method\":\"tools/call\",\"params\":{{\"name\":\"get_node\",\"arguments\":{{\"label\":\"alpha\"}}}}}}\n"
    );
    let responses = run_with_input(&gp, &input).await;
    let text_of = |id: i64| -> String {
        responses
            .iter()
            .find(|r| r["id"] == id)
            .map(|r| {
                r["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or_default()
    };
    assert!(
        text_of(23).to_lowercase().contains("error"),
        "bad project_path should return an error text: {}",
        text_of(23)
    );
    assert!(
        text_of(24).contains("alpha"),
        "server still serving default: {}",
        text_of(24)
    );
}

#[tokio::test]
async fn server_non_string_project_path_is_a_tool_error() {
    // A non-string project_path (here a number) must surface a tool error, not
    // silently fall back to the default graph — matching graphify-py, where
    // `Path(project_path)` raises on a non-str.
    let tmp = tempfile::tempdir().expect("tempdir");
    let gp = write_graph(tmp.path());
    let input = "{\"jsonrpc\":\"2.0\",\"id\":30,\"method\":\"tools/call\",\"params\":{\"name\":\"get_node\",\"arguments\":{\"label\":\"alpha\",\"project_path\":123}}}\n{\"jsonrpc\":\"2.0\",\"id\":31,\"method\":\"tools/call\",\"params\":{\"name\":\"get_node\",\"arguments\":{\"label\":\"alpha\"}}}\n";
    let responses = run_with_input(&gp, input).await;
    let text_of = |id: i64| -> String {
        responses
            .iter()
            .find(|r| r["id"] == id)
            .map(|r| {
                r["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string()
            })
            .unwrap_or_default()
    };
    assert!(
        text_of(30).contains("project_path must be a string"),
        "non-string project_path should error: {}",
        text_of(30)
    );
    assert!(
        text_of(31).contains("alpha"),
        "server still serves the default graph afterwards: {}",
        text_of(31)
    );
}
