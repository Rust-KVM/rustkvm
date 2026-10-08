use std::time::Duration;

use rustkvm::api::{JsonRpcProcessor, default_registry};
use rustkvm::video::{is_video_pipeline_running, restart_backoff, restart_video_pipeline};
use serde_json::json;

#[test]
fn restart_backoff_doubles_and_caps() {
    assert_eq!(restart_backoff(0), Duration::from_secs(5));
    assert_eq!(restart_backoff(1), Duration::from_secs(10));
    assert_eq!(restart_backoff(3), Duration::from_secs(40));
    assert_eq!(restart_backoff(6), Duration::from_secs(300));
    assert_eq!(restart_backoff(u32::MAX), Duration::from_secs(300));
}

#[tokio::test]
async fn restart_without_configured_pipeline_fails_cleanly() {
    assert!(!is_video_pipeline_running().await);
    let err = restart_video_pipeline().await.expect_err("no pipeline configured");
    assert!(err.to_string().contains("never configured"), "{err}");

    let request = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "method": "restartVideoPipeline", "id": 1
    }))
    .expect("encode");
    let response = JsonRpcProcessor::new(default_registry()).dispatch(&request).await;
    assert_eq!(response.error.expect("rpc error").code, -32603);
}

#[tokio::test]
async fn health_reports_pipeline_restarts() {
    let value = serde_json::to_value(rustkvm::observability::health::health_report().await)
        .expect("serialize health");
    assert!(value["video"]["restarts"].is_u64(), "{value}");
}
