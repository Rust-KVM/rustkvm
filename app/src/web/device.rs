use salvo::http::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue};
use salvo::prelude::*;
use uuid::Uuid;

use super::auth::set_auth_cookie;
use super::global_app_state;
use super::types::{
    ApiResponse, CloudStateResponse, DeviceStatusResponse, LocalDeviceResponse, SetupRequest,
};
use crate::cloud::types::CloudRegisterRequest;
use crate::config::get_config_manager;

#[endpoint]
pub(super) async fn handle_device() -> Result<Json<LocalDeviceResponse>, StatusError> {
    let config_manager = get_config_manager();
    let config = config_manager.get().await;

    Ok(Json(LocalDeviceResponse {
        auth_mode: Some(config.local_auth_mode),
        device_id: config.device_id,
        loopback_only: config.local_loopback_only,
    }))
}

#[endpoint]
pub(super) async fn handle_device_status() -> Result<Json<DeviceStatusResponse>, StatusError> {
    let config_manager = get_config_manager();
    let is_setup = config_manager.is_setup().await;

    Ok(Json(DeviceStatusResponse { is_setup }))
}

#[endpoint]
pub(super) async fn handle_device_setup(
    req: &mut Request,
    res: &mut Response,
) -> Result<Json<ApiResponse>, StatusError> {
    let config_manager = get_config_manager();
    let config = config_manager.get().await;

    if !config.local_auth_mode.is_empty() || config.hashed_password.is_some() {
        return Err(StatusError::bad_request().brief("Device is already set up"));
    }

    let setup_req: SetupRequest = req
        .parse_json()
        .await
        .map_err(|e| StatusError::bad_request().brief(format!("Invalid JSON: {e}")))?;

    if setup_req.local_auth_mode != "password" && setup_req.local_auth_mode != "noPassword" {
        return Err(StatusError::bad_request().brief("Invalid localAuthMode"));
    }

    config_manager
        .set_auth_mode(setup_req.local_auth_mode.clone())
        .await
        .map_err(|_| StatusError::internal_server_error().brief("Failed to save config"))?;

    if setup_req.local_auth_mode == "password" {
        let password = setup_req.password.ok_or_else(|| {
            StatusError::bad_request().brief("Password is required for password mode")
        })?;

        if password.is_empty() {
            return Err(StatusError::bad_request().brief("Password is required for password mode"));
        }

        let hashed_password = config_manager
            .hash_password(&password)
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
            .map_err(|_| StatusError::internal_server_error().brief("Failed to save config"))?;

        set_auth_cookie(res, &auth_token, time::Duration::days(7));
    } else {
        config_manager
            .set_hashed_password(None)
            .await
            .map_err(|_| StatusError::internal_server_error().brief("Failed to save config"))?;
        config_manager
            .set_auth_token(None)
            .await
            .map_err(|_| StatusError::internal_server_error().brief("Failed to save config"))?;
    }

    Ok(Json(ApiResponse { message: "Device setup completed successfully".to_string() }))
}

#[endpoint]
pub(super) async fn handle_cloud_register(
    req: &mut Request,
) -> Result<Json<ApiResponse>, StatusError> {
    global_app_state().ok_or_else(StatusError::internal_server_error)?;
    let cloud_manager = crate::cloud::manager::get_cloud_manager();

    let register_req: CloudRegisterRequest = match req.parse_json().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("Invalid register JSON: {}", e);
            return Err(StatusError::bad_request().brief("Invalid request body"));
        }
    };

    match cloud_manager.register_device(register_req).await {
        Ok(_) => Ok(Json(ApiResponse { message: "Cloud registration successful".to_string() })),
        Err(e) => {
            tracing::error!("Cloud registration failed: {}", e);
            Err(StatusError::bad_request().brief(format!("Cloud registration failed: {}", e)))
        }
    }
}

#[endpoint]
pub(super) async fn handle_cloud_status() -> Result<Json<CloudStateResponse>, StatusError> {
    global_app_state().ok_or_else(StatusError::internal_server_error)?;
    let cloud_state = crate::cloud::manager::get_cloud_manager().get_cloud_state().await;

    Ok(Json(CloudStateResponse {
        connected: cloud_state.connected,
        url: cloud_state.url,
        app_url: cloud_state.app_url,
    }))
}

#[endpoint]
pub(super) async fn handle_robots_txt(res: &mut Response) {
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    res.headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("public, max-age=31536000, immutable"));
    res.write_body("User-agent: *\nDisallow: /").ok();
}
