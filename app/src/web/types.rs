use salvo::oapi::ToSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WebRTCSessionRequest {
    pub sd: String,
    #[serde(default)]
    pub ice_servers: Option<Vec<String>>,
    #[serde(default)]
    pub ip: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct WebRTCSessionResponse {
    pub sd: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LoginRequest {
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SetPasswordRequest {
    pub password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SetupRequest {
    pub local_auth_mode: String,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct DeviceStatusResponse {
    pub is_setup: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct LocalDeviceResponse {
    pub auth_mode: Option<String>,
    pub device_id: String,
    pub loopback_only: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct CloudStateResponse {
    pub connected: bool,
    pub url: Option<String>,
    pub app_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct ApiResponse {
    pub message: String,
}
