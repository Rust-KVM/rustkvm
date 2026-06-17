use gloo_net::http::Request;
use serde_json::json;
use web_sys::RequestCredentials;

pub async fn auth_required() -> bool {
    match Request::get("/device").credentials(RequestCredentials::Include).send().await {
        Ok(resp) => resp.status() == 401,
        Err(_) => false,
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
        401 => Err("Current password is incorrect".to_string()),
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
        Err("Incorrect password".to_string())
    } else if resp.status() == 429 {
        Err("Too many attempts — try again later".to_string())
    } else {
        Err(format!("login HTTP {}", resp.status()))
    }
}
