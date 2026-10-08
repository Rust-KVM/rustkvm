use rustkvm::api::{JsonRpcProcessor, JsonRpcResponse, create_default_registry, default_registry};
use serde_json::{Value, json};

const EXPECTED_METHODS: &str = include_str!("rpc_methods.txt");

async fn call(raw: &[u8]) -> JsonRpcResponse {
    JsonRpcProcessor::new(default_registry()).dispatch(raw).await
}

async fn call_json(request: Value) -> JsonRpcResponse {
    call(&serde_json::to_vec(&request).unwrap()).await
}

#[test]
fn registered_methods_match_snapshot() {
    let registry = create_default_registry();
    let actual = registry.method_names().join("\n");
    let expected: Vec<&str> = EXPECTED_METHODS.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(
        actual,
        expected.join("\n"),
        "RPC method set changed; update app/tests/rpc_methods.txt if this is intentional"
    );
}

#[tokio::test]
async fn ping_returns_pong_and_echoes_id() {
    let resp = call_json(json!({"jsonrpc": "2.0", "method": "ping", "id": 42})).await;
    assert_eq!(resp.jsonrpc, "2.0");
    assert_eq!(resp.result, Some(json!("pong")));
    assert!(resp.error.is_none());
    assert_eq!(resp.id, Some(json!(42)));
}

#[tokio::test]
async fn string_ids_are_echoed() {
    let resp = call_json(json!({"jsonrpc": "2.0", "method": "ping", "id": "abc"})).await;
    assert_eq!(resp.id, Some(json!("abc")));
}

#[tokio::test]
async fn unknown_method_returns_method_not_found() {
    let resp = call_json(json!({"jsonrpc": "2.0", "method": "noSuchMethod", "id": 1})).await;
    assert!(resp.result.is_none());
    assert_eq!(resp.error.map(|e| e.code), Some(-32601));
    assert_eq!(resp.id, Some(json!(1)));
}

#[tokio::test]
async fn malformed_json_returns_parse_error() {
    for raw in [&b"{not json"[..], b"", b"[]", b"{\"jsonrpc\":\"2.0\",\"id\":1}"] {
        let resp = call(raw).await;
        assert_eq!(resp.error.map(|e| e.code), Some(-32700), "input {raw:?}");
        assert!(resp.id.is_none());
    }
}

#[tokio::test]
async fn invalid_params_return_internal_error_with_detail() {
    let resp = call_json(json!({
        "jsonrpc": "2.0",
        "method": "setJigglerState",
        "params": {"enabled": "not-a-bool"},
        "id": 3
    }))
    .await;
    let error = resp.error.expect("invalid params must produce an error");
    assert_eq!(error.code, -32603);
    assert!(error.data.is_some());
    assert_eq!(resp.id, Some(json!(3)));
}
