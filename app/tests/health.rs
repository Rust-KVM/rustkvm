use rustkvm::api::{JsonRpcProcessor, default_registry};
use rustkvm::observability::health::{HealthStatus, health_report};
use serde_json::json;

#[tokio::test]
async fn idle_device_reports_degraded_with_reasons() {
    let report = health_report().await;
    assert_eq!(report.status, HealthStatus::Degraded);
    assert!(report.issues.iter().any(|i| i.starts_with("video:")), "{:?}", report.issues);
    assert!(report.issues.iter().any(|i| i.starts_with("usb:")), "{:?}", report.issues);
    assert!(!report.video.ready);
    assert!(!report.version.version.is_empty());
}

#[tokio::test]
async fn health_serializes_with_stable_camel_case_keys() {
    let value = serde_json::to_value(health_report().await).unwrap();
    for key in [
        "status",
        "issues",
        "timestamp",
        "version",
        "video",
        "audioFramesTotal",
        "usbState",
        "network",
        "failsafe",
        "virtualMediaMounted",
    ] {
        assert!(value.get(key).is_some(), "missing key {key} in {value}");
    }
    assert_eq!(value["status"], json!("degraded"));
    assert!(value["video"].get("framesTotal").is_some());
    assert!(value["version"].get("buildDate").is_some());
}

#[tokio::test]
async fn health_is_reachable_over_rpc() {
    let request =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": "getHealth", "id": 1})).unwrap();
    let resp = JsonRpcProcessor::new(default_registry()).dispatch(&request).await;
    let result = resp.result.expect("getHealth must succeed off-device");
    assert!(result.get("status").is_some());
}
