use salvo::http::cookie::{Cookie, SameSite};
use salvo::prelude::*;
use tracing::warn;
use uuid::Uuid;

use super::ratelimit::{PASSWORD_RATE_LIMITER, client_ip, too_many_requests_error};
use super::types::{ApiResponse, ChangePasswordRequest, LoginRequest, SetPasswordRequest};
use crate::config::get_config_manager;

pub(super) fn set_auth_cookie(res: &mut Response, token: &str, max_age: time::Duration) {
    let cookie = Cookie::build(("authToken", token.to_string()))
        .max_age(max_age)
        .path("/")
        .http_only(true)
        .secure(false)
        .same_site(SameSite::Lax)
        .build();
    if let Ok(value) = salvo::http::HeaderValue::from_str(&cookie.to_string()) {
        res.headers_mut().append("Set-Cookie", value);
    }
}

#[endpoint]
pub(super) async fn handle_login_local(
    req: &mut Request,
    res: &mut Response,
) -> Result<Json<ApiResponse>, StatusError> {
    let config_manager = get_config_manager();
    let config = config_manager.get().await;

    if config.local_auth_mode == "noPassword" {
        return Err(StatusError::bad_request().brief("Login is disabled in noPassword mode"));
    }

    let ip = client_ip(req);
    if let Some(ip) = ip
        && let Err(retry_after) = PASSWORD_RATE_LIMITER.is_allowed(ip)
    {
        return Err(too_many_requests_error(res, retry_after));
    }

    let login_req: LoginRequest = req
        .parse_json()
        .await
        .map_err(|e| StatusError::bad_request().brief(format!("Invalid JSON: {e}")))?;

    if !config_manager.validate_password(&login_req.password).await {
        if let Some(ip) = ip {
            PASSWORD_RATE_LIMITER.record_failure(ip);
        }
        return Err(StatusError::unauthorized().brief("Invalid password"));
    }
    if let Some(ip) = ip {
        PASSWORD_RATE_LIMITER.record_success(ip);
    }

    let auth_token = Uuid::new_v4().to_string();
    config_manager
        .set_auth_token(Some(auth_token.clone()))
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to save configuration"))?;

    let updated_config = config_manager.get().await;
    if updated_config.local_auth_token != Some(auth_token.clone()) {
        warn!("Auth token not properly saved to configuration");
        return Err(StatusError::internal_server_error().brief("Failed to save configuration"));
    }

    set_auth_cookie(res, &auth_token, time::Duration::days(7));

    Ok(Json(ApiResponse { message: "Login successful".to_string() }))
}

#[endpoint]
pub(super) async fn handle_logout(res: &mut Response) -> Result<Json<ApiResponse>, StatusError> {
    let config_manager = get_config_manager();

    config_manager
        .set_auth_token(None)
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to save configuration"))?;

    set_auth_cookie(res, "", time::Duration::seconds(-1));

    Ok(Json(ApiResponse { message: "Logout successful".to_string() }))
}

#[endpoint]
pub(super) async fn create_password_local(
    req: &mut Request,
    res: &mut Response,
) -> Result<Json<ApiResponse>, StatusError> {
    let config_manager = get_config_manager();
    let config = config_manager.get().await;

    if config.hashed_password.is_some() {
        return Err(StatusError::bad_request().brief("Password already set"));
    }

    if config.local_auth_mode != "noPassword" {
        return Err(StatusError::bad_request().brief("Password mode is not enabled"));
    }

    let password_req: SetPasswordRequest = req
        .parse_json()
        .await
        .map_err(|e| StatusError::bad_request().brief(format!("Invalid JSON: {e}")))?;

    if password_req.password.is_empty() {
        return Err(StatusError::bad_request().brief("Invalid request"));
    }

    let hashed_password = config_manager
        .hash_password(&password_req.password)
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to hash password"))?;
    let auth_token = Uuid::new_v4().to_string();

    config_manager
        .set_hashed_password(Some(hashed_password))
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to save password"))?;
    config_manager
        .set_auth_token(Some(auth_token.clone()))
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to save configuration"))?;
    config_manager
        .set_auth_mode("password".to_string())
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to save configuration"))?;

    set_auth_cookie(res, &auth_token, time::Duration::days(7));

    Ok(Json(ApiResponse { message: "Password set successfully".to_string() }))
}

#[endpoint]
pub(super) async fn modify_password_local(
    req: &mut Request,
    res: &mut Response,
) -> Result<Json<ApiResponse>, StatusError> {
    let config_manager = get_config_manager();
    let config = config_manager.get().await;

    if config.hashed_password.is_none() {
        return Err(StatusError::bad_request().brief("Password is not set"));
    }

    if config.local_auth_mode != "password" {
        return Err(StatusError::bad_request().brief("Password mode is not enabled"));
    }

    let change_req: ChangePasswordRequest = req
        .parse_json()
        .await
        .map_err(|e| StatusError::bad_request().brief(format!("Invalid JSON: {e}")))?;

    if change_req.old_password.is_empty() || change_req.new_password.is_empty() {
        return Err(StatusError::bad_request().brief("Invalid request"));
    }

    if !config_manager.validate_password(&change_req.old_password).await {
        return Err(StatusError::unauthorized().brief("Incorrect old password"));
    }

    let new_hashed_password = config_manager
        .hash_password(&change_req.new_password)
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to hash new password"))?;
    let new_auth_token = Uuid::new_v4().to_string();

    config_manager
        .set_hashed_password(Some(new_hashed_password))
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to save new password"))?;
    config_manager
        .set_auth_token(Some(new_auth_token.clone()))
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to save configuration"))?;

    set_auth_cookie(res, &new_auth_token, time::Duration::days(7));

    Ok(Json(ApiResponse { message: "Password updated successfully".to_string() }))
}

#[endpoint]
pub(super) async fn disable_local_password(
    req: &mut Request,
    res: &mut Response,
) -> Result<Json<ApiResponse>, StatusError> {
    let config_manager = get_config_manager();
    let config = config_manager.get().await;

    if config.hashed_password.is_none() {
        return Err(StatusError::bad_request().brief("Password is not set"));
    }

    if config.local_auth_mode != "password" {
        return Err(StatusError::bad_request().brief("Password mode is not enabled"));
    }

    let login_req: LoginRequest = req
        .parse_json()
        .await
        .map_err(|e| StatusError::bad_request().brief(format!("Invalid JSON: {e}")))?;

    if login_req.password.is_empty() {
        return Err(StatusError::bad_request().brief("Invalid request"));
    }

    if !config_manager.validate_password(&login_req.password).await {
        return Err(StatusError::unauthorized().brief("Incorrect password"));
    }

    config_manager
        .set_hashed_password(None)
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to save configuration"))?;
    config_manager
        .set_auth_token(None)
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to save configuration"))?;
    config_manager
        .set_auth_mode("noPassword".to_string())
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to save configuration"))?;

    set_auth_cookie(res, "", time::Duration::seconds(-1));

    Ok(Json(ApiResponse { message: "Password disabled successfully".to_string() }))
}
