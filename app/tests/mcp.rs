use std::collections::HashSet;

use rustkvm::api::create_default_registry;
use rustkvm::mcp::{LATEST_PROTOCOL_VERSION, McpServer, rpc_method_for_tool, tool_definitions};
use serde_json::{Value, json};

fn server() -> McpServer {
    McpServer::new(std::sync::Arc::new(create_default_registry()))
}

async fn call(request: Value) -> Value {
    let body = serde_json::to_vec(&request).expect("encode request");
    let response = server().handle(&body).await.expect("request expects a response");
    serde_json::to_value(response).expect("encode response")
}

#[tokio::test]
async fn initialize_negotiates_protocol_version() {
    let resp = call(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "t", "version": "0" } }
    }))
    .await;
    assert_eq!(resp["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(resp["result"]["serverInfo"]["name"], "rustkvm");
    assert!(resp["result"]["capabilities"]["tools"].is_object());

    let resp = call(json!({
        "jsonrpc": "2.0", "id": 2, "method": "initialize",
        "params": { "protocolVersion": "1999-01-01" }
    }))
    .await;
    assert_eq!(resp["result"]["protocolVersion"], LATEST_PROTOCOL_VERSION);
}

#[tokio::test]
async fn notifications_get_no_response() {
    let body = br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    assert!(server().handle(body).await.is_none());
}

#[tokio::test]
async fn parse_errors_and_unknown_methods() {
    let resp = server().handle(b"{not json").await.expect("parse error response");
    assert_eq!(resp.error.expect("error").code, -32700);

    let resp = call(json!({ "jsonrpc": "2.0", "id": 3, "method": "resources/list" })).await;
    assert_eq!(resp["error"]["code"], -32601);
}

#[tokio::test]
async fn tools_list_is_well_formed_and_backed_by_rpc_methods() {
    let resp = call(json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/list" })).await;
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), tool_definitions().len());

    let registry = create_default_registry();
    let methods: HashSet<&str> = registry.method_names().into_iter().collect();
    let mut names = HashSet::new();
    for tool in tools {
        let name = tool["name"].as_str().expect("tool name");
        assert!(names.insert(name.to_string()), "duplicate tool {name}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{name} schema");
        assert!(tool["description"].as_str().is_some_and(|d| !d.is_empty()));
        if let Some(method) = rpc_method_for_tool(name) {
            assert!(methods.contains(method), "{name} maps to unregistered {method}");
        }
    }
    for builtin in ["screenshot", "rpc_call", "list_rpc_methods", "type_text", "mouse_click"] {
        assert!(names.contains(builtin), "missing {builtin}");
    }
}

#[tokio::test]
async fn tools_call_routes_to_rpc_registry() {
    let resp = call(json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": { "name": "rpc_call", "arguments": { "method": "ping" } }
    }))
    .await;
    assert_eq!(resp["result"]["isError"], false);
    assert_eq!(resp["result"]["content"][0]["text"], "pong");

    let resp = call(json!({
        "jsonrpc": "2.0", "id": 6, "method": "tools/call",
        "params": { "name": "list_rpc_methods", "arguments": {} }
    }))
    .await;
    assert!(resp["result"]["content"][0]["text"].as_str().is_some_and(|t| t.contains("\"ping\"")));
}

#[tokio::test]
async fn tool_failures_are_reported_in_band() {
    for (name, args) in [
        ("no_such_tool", json!({})),
        ("rpc_call", json!({ "method": "noSuchMethod" })),
        ("rpc_call", json!({})),
        ("type_text", json!({})),
    ] {
        let resp = call(json!({
            "jsonrpc": "2.0", "id": 7, "method": "tools/call",
            "params": { "name": name, "arguments": args }
        }))
        .await;
        assert!(resp["error"].is_null(), "{name} should not be a protocol error");
        assert_eq!(resp["result"]["isError"], true, "{name}");
    }
}
