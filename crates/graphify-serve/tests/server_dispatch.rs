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
    assert!(!tools.is_empty());
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
    assert!(!resources.is_empty());
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
