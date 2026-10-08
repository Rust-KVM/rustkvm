use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};
use tracing::{debug, warn};

use crate::api::RpcRegistry;
use crate::api::types::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};

pub const LATEST_PROTOCOL_VERSION: &str = "2025-06-18";
const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "RustKVM controls a physical computer over HDMI capture and USB HID. \
Call screenshot to see the screen, then type_text / press_combo / mouse_click to act. \
Pointer coordinates are video pixels unless normalized=true (0..32767). \
get_health reports video, USB, network and failsafe state. \
rpc_call reaches any device JSON-RPC method listed by list_rpc_methods.";

pub struct McpServer {
    registry: Arc<RpcRegistry>,
}

impl McpServer {
    pub fn new(registry: Arc<RpcRegistry>) -> Self {
        Self { registry }
    }

    /// Returns `None` for notifications, which get no JSON-RPC response.
    pub async fn handle(&self, body: &[u8]) -> Option<JsonRpcResponse> {
        let request: JsonRpcRequest = match serde_json::from_slice(body) {
            Ok(req) => req,
            Err(e) => {
                warn!(error = %e, "mcp request parse failed");
                return Some(error_response(None, JsonRpcError::parse_error()));
            }
        };
        let id = request.id?;
        debug!(method = %request.method, "mcp request");

        let result = match request.method.as_str() {
            "initialize" => Ok(initialize_result(request.params.as_ref())),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tool_definitions() })),
            "tools/call" => Ok(self.call_tool(request.params).await),
            _ => Err(JsonRpcError::method_not_found()),
        };
        Some(match result {
            Ok(result) => JsonRpcResponse {
                jsonrpc: "2.0".to_string(),
                result: Some(result),
                error: None,
                id: Some(id),
            },
            Err(error) => error_response(Some(id), error),
        })
    }

    async fn call_tool(&self, params: Option<Value>) -> Value {
        let params = params.unwrap_or(Value::Null);
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return tool_error("missing tool name");
        };
        let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

        let outcome = match name {
            "screenshot" => return screenshot().await,
            "list_rpc_methods" => Ok(json!(self.registry.method_names())),
            "rpc_call" => match args.get("method").and_then(Value::as_str) {
                Some(method) => self.registry.call(method, args.get("params").cloned()).await,
                None => return tool_error("rpc_call requires a string `method`"),
            },
            _ => match rpc_method_for_tool(name) {
                Some(method) => {
                    let params = if args.as_object().is_some_and(|o| o.is_empty()) {
                        None
                    } else {
                        Some(args)
                    };
                    self.registry.call(method, params).await
                }
                None => return tool_error(&format!("unknown tool: {name}")),
            },
        };

        match outcome {
            Ok(value) => tool_text(&value_to_text(&value), false),
            Err(e) => tool_error(&format!("{e:#}")),
        }
    }
}

pub fn rpc_method_for_tool(name: &str) -> Option<&'static str> {
    Some(match name {
        "get_health" => "getHealth",
        "type_text" => "typeText",
        "press_combo" => "pressCombo",
        "mouse_move" => "mouseMove",
        "mouse_click" => "mouseClick",
        "atx_power" => "setATXPowerAction",
        "get_atx_state" => "getATXState",
        "dc_power" => "setDCPowerState",
        "get_dc_power_state" => "getDCPowerState",
        "wake_on_lan" => "sendWOLMagicPacket",
        _ => return None,
    })
}

pub fn tool_definitions() -> Vec<Value> {
    let pointer = |extra: Value| {
        let mut props = json!({
            "x": { "type": "integer", "minimum": 0 },
            "y": { "type": "integer", "minimum": 0 },
            "normalized": {
                "type": "boolean",
                "description": "Treat x/y as 0..32767 absolute HID units instead of video pixels"
            }
        });
        if let (Some(props), Some(extra)) = (props.as_object_mut(), extra.as_object()) {
            props.extend(extra.clone());
        }
        json!({ "type": "object", "properties": props, "required": ["x", "y"] })
    };
    let empty = json!({ "type": "object", "properties": {} });

    vec![
        tool("screenshot", "Capture the current HDMI screen as a JPEG image", empty.clone(), true),
        tool(
            "get_health",
            "Device health: video signal, USB gadget, network, failsafe, versions",
            empty.clone(),
            true,
        ),
        tool(
            "type_text",
            "Type text on the target using the US keyboard layout",
            json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "maxLength": 10000 },
                    "delayMs": { "type": "integer", "minimum": 5, "maximum": 1000 }
                },
                "required": ["text"]
            }),
            false,
        ),
        tool(
            "press_combo",
            "Press a key combination such as ctrl+alt+delete, win+r, enter or f12",
            json!({
                "type": "object",
                "properties": {
                    "combo": { "type": "string" },
                    "holdMs": { "type": "integer", "minimum": 10, "maximum": 5000 }
                },
                "required": ["combo"]
            }),
            false,
        ),
        tool("mouse_move", "Move the absolute pointer", pointer(json!({})), false),
        tool(
            "mouse_click",
            "Move the pointer and click",
            pointer(json!({
                "button": { "type": "string", "enum": ["left", "right", "middle"] },
                "double": { "type": "boolean" }
            })),
            false,
        ),
        tool(
            "atx_power",
            "Press the ATX power or reset button of the target",
            json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["power-short", "power-long", "reset"] }
                },
                "required": ["action"]
            }),
            false,
        ),
        tool("get_atx_state", "Read ATX power and HDD LED state", empty.clone(), true),
        tool(
            "dc_power",
            "Switch the DC power extension on or off",
            json!({
                "type": "object",
                "properties": { "enabled": { "type": "boolean" } },
                "required": ["enabled"]
            }),
            false,
        ),
        tool("get_dc_power_state", "Read the DC power extension state", empty.clone(), true),
        tool(
            "wake_on_lan",
            "Send a Wake-on-LAN magic packet",
            json!({
                "type": "object",
                "properties": { "macAddress": { "type": "string" } },
                "required": ["macAddress"]
            }),
            false,
        ),
        tool("list_rpc_methods", "List every device JSON-RPC method", empty, true),
        tool(
            "rpc_call",
            "Call any device JSON-RPC method by name",
            json!({
                "type": "object",
                "properties": {
                    "method": { "type": "string" },
                    "params": { "description": "Method parameters, if any" }
                },
                "required": ["method"]
            }),
            false,
        ),
    ]
}

fn tool(name: &str, description: &str, input_schema: Value, read_only: bool) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": { "readOnlyHint": read_only }
    })
}

fn initialize_result(params: Option<&Value>) -> Value {
    let requested = params.and_then(|p| p.get("protocolVersion")).and_then(Value::as_str);
    let version = requested
        .filter(|v| SUPPORTED_PROTOCOL_VERSIONS.contains(v))
        .unwrap_or(LATEST_PROTOCOL_VERSION);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false } },
        "serverInfo": { "name": "rustkvm", "version": crate::version::built_app_version() },
        "instructions": INSTRUCTIONS
    })
}

async fn screenshot() -> Value {
    match crate::video::capture_screenshot_jpeg().await {
        Ok(jpeg) => json!({
            "content": [{ "type": "image", "data": STANDARD.encode(&jpeg), "mimeType": "image/jpeg" }],
            "isError": false
        }),
        Err(e) => tool_error(&format!("screenshot failed: {e:#}")),
    }
}

fn value_to_text(value: &Value) -> String {
    match value {
        Value::Null => "ok".to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn tool_text(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn tool_error(message: &str) -> Value {
    tool_text(message, true)
}

fn error_response(id: Option<Value>, error: JsonRpcError) -> JsonRpcResponse {
    JsonRpcResponse { jsonrpc: "2.0".to_string(), result: None, error: Some(error), id }
}
