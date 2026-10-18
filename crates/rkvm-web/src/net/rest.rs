use gloo_net::http::Request;
use serde_json::json;
use web_sys::RequestCredentials;

use crate::i18n::tr;

pub async fn auth_required() -> bool {
    match Request::get("/device").credentials(RequestCredentials::Include).send().await {
        Ok(resp) => resp.status() == 401,
        Err(_) => false,
    }
}

/// Whether the device runs without a password (`authMode: "noPassword"`), in
/// which case anyone who can reach the web UI gets full control.
pub async fn no_password() -> bool {
    match Request::get("/device").credentials(RequestCredentials::Include).send().await {
        Ok(resp) if resp.ok() => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("authMode").and_then(|m| m.as_str().map(|m| m == "noPassword")))
            .unwrap_or(false),
        _ => false,
    }
}

pub async fn is_setup() -> bool {
    match Request::get("/device/status").credentials(RequestCredentials::Include).send().await {
        Ok(resp) => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("isSetup").and_then(serde_json::Value::as_bool))
            .unwrap_or(true),
        Err(_) => true,
    }
}

pub async fn setup(mode: &str, password: Option<&str>) -> Result<(), String> {
    let body = match password {
        Some(pw) => json!({ "localAuthMode": mode, "password": pw }),
        None => json!({ "localAuthMode": mode }),
    };
    let resp = Request::post("/device/setup")
        .credentials(RequestCredentials::Include)
        .json(&body)
        .map_err(|e| format!("setup serialize: {e}"))?
        .send()
        .await
        .map_err(|e| format!("setup request failed: {e}"))?;
    if resp.ok() { Ok(()) } else { Err(format!("setup HTTP {}", resp.status())) }
}

/// Sets the first password on a device in no-password mode.
pub async fn set_password(password: &str) -> Result<(), String> {
    let resp = Request::post("/auth/password-local")
        .credentials(RequestCredentials::Include)
        .json(&json!({ "password": password }))
        .map_err(|e| format!("set-password serialize: {e}"))?
        .send()
        .await
        .map_err(|e| format!("set-password request failed: {e}"))?;
    if resp.ok() { Ok(()) } else { Err(format!("set-password HTTP {}", resp.status())) }
}

pub async fn change_password(old: &str, new: &str) -> Result<(), String> {
    let resp = Request::put("/auth/password-local")
        .credentials(RequestCredentials::Include)
        .json(&json!({ "oldPassword": old, "newPassword": new }))
        .map_err(|e| format!("change-password serialize: {e}"))?
        .send()
        .await
        .map_err(|e| format!("change-password request failed: {e}"))?;
    match resp.status() {
        s if (200..300).contains(&s) => Ok(()),
        401 => Err(tr("当前密码不正确", "Current password is incorrect").to_string()),
        s => Err(format!("change-password HTTP {s}")),
    }
}

pub async fn login_local(password: &str) -> Result<(), String> {
    let resp = Request::post("/auth/login-local")
        .credentials(RequestCredentials::Include)
        .json(&json!({ "password": password }))
        .map_err(|e| format!("login serialize: {e}"))?
        .send()
        .await
        .map_err(|e| format!("login request failed: {e}"))?;
    if resp.ok() {
        Ok(())
    } else if resp.status() == 401 {
        Err(tr("密码不正确", "Incorrect password").to_string())
    } else if resp.status() == 429 {
        Err(tr("尝试次数过多，请稍后再试", "Too many attempts, try again later").to_string())
    } else {
        Err(format!("login HTTP {}", resp.status()))
    }
}
