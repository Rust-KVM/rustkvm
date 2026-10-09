use std::time::Duration;

use reqwest::blocking::Client;
use serde_json::{Value, json};

struct Target {
    base: String,
    token: Option<String>,
    require_healthy: bool,
}

fn target() -> Option<Target> {
    let base = std::env::var("RKVM_DEVICE_URL").ok().filter(|u| !u.is_empty())?;
    Some(Target {
        base: base.trim_end_matches('/').to_owned(),
        token: std::env::var("RKVM_DEVICE_TOKEN").ok().filter(|t| !t.is_empty()),
        require_healthy: std::env::var("RKVM_REQUIRE_HEALTHY").is_ok_and(|v| v == "1"),
    })
}

fn client() -> Client {
    // Devices serve a self-signed certificate generated at startup.
    Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(20))
        .build()
        .expect("http client")
}

fn authed(
    target: &Target,
    builder: reqwest::blocking::RequestBuilder,
) -> reqwest::blocking::RequestBuilder {
    match &target.token {
        Some(token) => builder.bearer_auth(token),
        None => builder,
    }
}

#[test]
fn unauthenticated_surface_is_locked_down() {
    let Some(t) = target() else {
        eprintln!("RKVM_DEVICE_URL not set; skipping live device probe");
        return;
    };
    let http = client();

    let status: Value =
        http.get(format!("{}/device/status", t.base)).send().unwrap().json().unwrap();
    assert!(status.get("isSetup").is_some(), "{status}");

    let metrics = http.get(format!("{}/metrics", t.base)).send().unwrap().text().unwrap();
    assert!(metrics.contains("rustkvm_app_info"), "metrics lack build info");

    if t.token.is_some() {
        for path in ["/device/health", "/device/screenshot"] {
            let code = http.get(format!("{}{path}", t.base)).send().unwrap().status();
            assert!(
                code.is_client_error() || code.is_redirection(),
                "{path} reachable without credentials: {code}"
            );
        }
    }
}

#[test]
fn authenticated_api_contract_holds() {
    let Some(t) = target() else {
        eprintln!("RKVM_DEVICE_URL not set; skipping live device probe");
        return;
    };
    if t.token.is_none() {
        eprintln!("RKVM_DEVICE_TOKEN not set; skipping authenticated probe");
        return;
    }
    let http = client();

    let health: Value = authed(&t, http.get(format!("{}/device/health", t.base)))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap();
    for key in ["status", "issues", "version", "video", "usbState", "network", "failsafe"] {
        assert!(health.get(key).is_some(), "health lacks {key}: {health}");
    }
    if t.require_healthy {
        assert_eq!(health["status"], "ok", "device degraded: {}", health["issues"]);
    }

    let rpc: Value = authed(&t, http.post(format!("{}/device/rpc", t.base)))
        .json(&json!({"jsonrpc": "2.0", "method": "getHealth", "id": 1}))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(rpc["id"], 1);
    assert!(rpc.get("error").is_none_or(Value::is_null), "{rpc}");

    let mcp: Value = authed(&t, http.post(format!("{}/mcp", t.base)))
        .json(&json!({"jsonrpc": "2.0", "method": "tools/list", "id": 2}))
        .send()
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .unwrap();
    let tools = mcp["result"]["tools"].as_array().expect("tools array");
    for name in ["screenshot", "get_health", "type_text"] {
        assert!(tools.iter().any(|tool| tool["name"] == name), "MCP lacks {name}");
    }
}
