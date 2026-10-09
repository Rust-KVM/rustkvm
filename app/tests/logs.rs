use rustkvm::api::{JsonRpcProcessor, default_registry};
use rustkvm::mcp::rpc_method_for_tool;
use rustkvm::observability::log_buffer::{self, CAPACITY};
use serde_json::{Value, json};
use tracing::Level;
use tracing_subscriber::layer::SubscriberExt;

async fn rpc(params: Value) -> Value {
    let request = json!({"jsonrpc": "2.0", "method": "getRecentLogs", "params": params, "id": 1});
    let resp = JsonRpcProcessor::new(default_registry())
        .dispatch(&serde_json::to_vec(&request).unwrap())
        .await;
    serde_json::to_value(resp).unwrap()
}

// One sequential test: the ring buffer is process-global, and the capacity
// phase evicts everything recorded before it.
#[tokio::test]
async fn buffer_captures_filters_and_serves_logs() {
    let subscriber = tracing_subscriber::registry().with(log_buffer::layer());
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(attempt = 3, "probe-alpha started");
        tracing::warn!("probe-beta degraded");
        tracing::error!(target: "probe_target", "probe-gamma failed");
        tracing::debug!("probe-delta noise");
    });

    let all = log_buffer::recent(CAPACITY, None, Some("probe-"));
    let messages: Vec<&str> = all.iter().map(|e| e.message.as_str()).collect();
    assert_eq!(
        messages,
        [
            "probe-alpha started attempt=3",
            "probe-beta degraded",
            "probe-gamma failed",
            "probe-delta noise"
        ]
    );
    assert_eq!(all[2].target, "probe_target");
    assert_eq!(all[2].level, "ERROR");

    let warn_up = log_buffer::recent(CAPACITY, Some(Level::WARN), Some("probe-"));
    assert_eq!(warn_up.len(), 2);
    let newest = log_buffer::recent(1, None, Some("probe-"));
    assert_eq!(newest[0].message, "probe-delta noise");
    assert_eq!(log_buffer::recent(CAPACITY, None, Some("probe_target")).len(), 1);

    let resp = rpc(json!({"level": "WARN", "contains": "probe-"})).await;
    let entries = resp["result"].as_array().expect("result array");
    assert_eq!(entries.len(), 2, "{resp}");
    assert_eq!(entries[0]["level"], "WARN");
    assert!(entries[0]["timestampMs"].as_u64().is_some_and(|t| t > 0));

    let resp = rpc(json!({"level": "LOUD"})).await;
    assert!(resp["error"].is_object(), "{resp}");
    let resp = rpc(json!({"limit": 1, "contains": "probe-"})).await;
    assert_eq!(resp["result"].as_array().map(Vec::len), Some(1));
    assert!(rpc(json!({})).await["result"].is_array());

    assert_eq!(rpc_method_for_tool("get_logs"), Some("getRecentLogs"));

    let subscriber = tracing_subscriber::registry().with(log_buffer::layer());
    tracing::subscriber::with_default(subscriber, || {
        for i in 0..CAPACITY + 200 {
            tracing::info!("flood {i}");
        }
    });
    let all = log_buffer::recent(usize::MAX, None, None);
    assert_eq!(all.len(), CAPACITY);
    assert_eq!(all[0].message, "flood 200");
    assert_eq!(all[CAPACITY - 1].message, format!("flood {}", CAPACITY + 199));
}
