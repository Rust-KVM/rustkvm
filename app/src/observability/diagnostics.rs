pub async fn get_system_snapshot() -> anyhow::Result<serde_json::Value> {
    let uptime = tokio::fs::read_to_string("/proc/uptime")
        .await
        .ok()
        .and_then(|s| s.split_whitespace().next().map(|v| v.to_string()));
    Ok(serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "uptime": uptime,
    }))
}
