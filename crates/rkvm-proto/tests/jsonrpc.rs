use rkvm_proto::jsonrpc::{JsonRpcError, JsonRpcEvent, JsonRpcRequest, JsonRpcResponse};
use serde_json::{Value, json};

#[test]
fn request_params_and_id_are_optional() {
    let req: JsonRpcRequest =
        serde_json::from_value(json!({"jsonrpc": "2.0", "method": "ping"})).unwrap();
    assert_eq!(req.method, "ping");
    assert!(req.params.is_none());
    assert!(req.id.is_none());

    let req: JsonRpcRequest = serde_json::from_value(
        json!({"jsonrpc": "2.0", "method": "setJigglerState", "params": {"enabled": true}, "id": 7}),
    )
    .unwrap();
    assert_eq!(req.params, Some(json!({"enabled": true})));
    assert_eq!(req.id, Some(json!(7)));
}

#[test]
fn request_without_method_is_rejected() {
    assert!(serde_json::from_value::<JsonRpcRequest>(json!({"jsonrpc": "2.0", "id": 1})).is_err());
}

#[test]
fn success_response_omits_error_field() {
    let resp = JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result: Some(json!("pong")),
        error: None,
        id: Some(json!(1)),
    };
    assert_eq!(
        serde_json::to_value(&resp).unwrap(),
        json!({"jsonrpc": "2.0", "result": "pong", "id": 1})
    );
}

#[test]
fn error_response_omits_result_and_keeps_null_id() {
    let resp = JsonRpcResponse {
        jsonrpc: "2.0".into(),
        result: None,
        error: Some(JsonRpcError::parse_error()),
        id: None,
    };
    assert_eq!(
        serde_json::to_value(&resp).unwrap(),
        json!({"jsonrpc": "2.0", "error": {"code": -32700, "message": "Parse error"}, "id": null})
    );
}

#[test]
fn standard_error_codes() {
    assert_eq!(JsonRpcError::parse_error().code, -32700);
    assert_eq!(JsonRpcError::method_not_found().code, -32601);
    let internal = JsonRpcError::internal_error(Some("boom".into()));
    assert_eq!(internal.code, -32603);
    assert_eq!(internal.data, Some(Value::String("boom".into())));
    assert!(JsonRpcError::internal_error(None).data.is_none());
}

#[test]
fn event_serializes_without_id() {
    let event = JsonRpcEvent {
        jsonrpc: "2.0".into(),
        method: "usbState".into(),
        params: Some(json!("configured")),
    };
    assert_eq!(
        serde_json::to_value(&event).unwrap(),
        json!({"jsonrpc": "2.0", "method": "usbState", "params": "configured"})
    );
}
