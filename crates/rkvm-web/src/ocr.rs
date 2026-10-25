use serde_json::{Value, json};

use crate::rpc::RpcClient;

/// Reads the screen text on the device (NPU OCR, `readScreenText`), one
/// recognized line per output line in reading order.
pub async fn recognize(client: &RpcClient) -> Result<String, String> {
    // The first call loads the models, and a busy 4K screen takes a few
    // seconds on the NPU; a retry would only queue a second full pass.
    let result = client
        .call_with("readScreenText", json!({}), 20_000, 1)
        .await
        .map_err(|e| e.to_string())?;
    let lines = result.get("lines").and_then(Value::as_array).ok_or("malformed OCR result")?;
    let text: Vec<&str> =
        lines.iter().filter_map(|l| l.get("text").and_then(Value::as_str)).collect();
    Ok(text.join("\n"))
}
