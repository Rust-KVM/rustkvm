use anyhow::{Result, anyhow};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::info;

use crate::cloud::types::{CloudConnectionState, CloudState};

static NETWORK_IP: once_cell::sync::OnceCell<RwLock<String>> = once_cell::sync::OnceCell::new();

pub fn get_network_ip_address() -> Result<String> {
    let state = crate::network::get_network_state();
    if !state.ipv4_address.is_empty() {
        return Ok(state.ipv4_address);
    }
    let cell = NETWORK_IP.get_or_init(|| RwLock::new(String::new()));
    Ok(cell.read().clone())
}

#[derive(Deserialize)]
pub struct NetworkIpParam {
    pub ip: String,
}

pub fn set_network_ip_address(param: NetworkIpParam) -> Result<Value> {
    let cell = NETWORK_IP.get_or_init(|| RwLock::new(String::new()));
    *cell.write() = param.ip;
    Ok(Value::Null)
}

#[derive(Serialize)]
pub struct NetworkStateResponse {
    pub online: bool,
    pub ip: String,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
}

pub fn get_network_state() -> Result<NetworkStateResponse> {
    let state = crate::network::get_network_state();
    Ok(NetworkStateResponse {
        online: state.online,
        ip: state.ipv4_address,
        gateway: if state.ipv6_gateway.is_empty() { None } else { Some(state.ipv6_gateway) },
        dns: state.dhcp_lease.as_ref().map(|l| l.dns.clone()).unwrap_or_default(),
    })
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NetworkSettingsResponse {
    pub dhcp_enabled: bool,
    pub static_ip: Option<String>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
}

pub async fn get_network_settings() -> Result<NetworkSettingsResponse> {
    let cfg = crate::config::get_config_manager().get().await;
    Ok(NetworkSettingsResponse {
        dhcp_enabled: cfg.network_config.ipv4_mode != "static",
        static_ip: None,
        gateway: None,
        dns: vec![],
    })
}

#[derive(Deserialize)]
pub struct NetworkSettingsParams {
    pub settings: NetworkSettingsResponse,
}

pub async fn set_network_settings(params: NetworkSettingsParams) -> Result<Value> {
    info!("Network settings update requested: {:?}", params.settings);
    let new_ipv4_mode = if params.settings.dhcp_enabled { "dhcp" } else { "static" };
    let mgr = crate::config::get_config_manager();
    let ipv4_mode_changed = mgr.get().await.network_config.ipv4_mode != new_ipv4_mode;

    mgr.update(|cfg| {
        cfg.network_config.ipv4_mode = new_ipv4_mode.to_string();
    })
    .await?;

    if ipv4_mode_changed {
        crate::api::broadcast_will_reboot("network_settings_changed").await;
    }
    Ok(Value::Null)
}

pub fn renew_dhcp_lease() -> Result<Value> {
    info!("DHCP lease renewal requested");
    let if_name = crate::network::NET_IF_NAME;
    let status = std::process::Command::new("/sbin/udhcpc")
        .args(["-i", if_name, "-q", "-n", "-t", "3"])
        .status();
    match status {
        Ok(s) if s.success() => Ok(Value::Null),
        Ok(s) => Err(anyhow::anyhow!("udhcpc exited with status {}", s)),
        Err(e) => {
            tracing::warn!("DHCP renewal via udhcpc failed: {e}");
            Err(anyhow::anyhow!("udhcpc not available: {e}"))
        }
    }
}

pub async fn get_wake_on_lan_devices() -> Result<Vec<crate::config::types::WakeOnLanDevice>> {
    let mgr = crate::config::get_config_manager();
    let cfg = mgr.get().await;
    Ok(cfg.wake_on_lan_devices)
}

#[derive(Deserialize)]
pub struct SetWakeOnLanDevicesParams {
    pub devices: Vec<crate::config::types::WakeOnLanDevice>,
}

pub async fn set_wake_on_lan_devices(params: SetWakeOnLanDevicesParams) -> Result<Value> {
    let mgr = crate::config::get_config_manager();
    let devices = params.devices;
    mgr.update(move |cfg| {
        cfg.wake_on_lan_devices = devices;
    })
    .await?;
    info!("Wake-on-LAN devices updated");
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct SendWOLMagicPacketParams {
    #[serde(rename = "macAddress")]
    pub mac_address: String,
    #[serde(default, rename = "broadcastIP")]
    pub broadcast_ip: Option<String>,
}

pub async fn send_wol_magic_packet(params: SendWOLMagicPacketParams) -> Result<Value> {
    rkvm_net::wol::send_magic_packet(&params.mac_address, params.broadcast_ip.as_deref()).await?;
    Ok(Value::Null)
}

pub async fn get_local_loopback_only() -> Result<bool> {
    let config_manager = crate::config::get_config_manager();
    let config = config_manager.get().await;
    Ok(config.local_loopback_only)
}

#[derive(Deserialize)]
pub struct LocalLoopbackParams {
    pub enabled: bool,
}

pub async fn set_local_loopback_only(params: LocalLoopbackParams) -> Result<Value> {
    let config_manager = crate::config::get_config_manager();

    config_manager
        .update(|config| {
            config.local_loopback_only = params.enabled;
        })
        .await?;

    let new_state = config_manager.get().await.local_loopback_only;

    info!("Local loopback only mode set to: {}", new_state);
    Ok(serde_json::to_value(new_state)?)
}

#[derive(Serialize)]
pub struct PublicIPResponse {
    pub ipv4: Option<String>,
    pub ipv6: Option<String>,
}

#[derive(Deserialize)]
pub struct CheckPublicIPParams {
    pub refresh: Option<bool>,
}

async fn fetch_text(url: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .use_rustls_tls()
        .build()
        .ok()?;
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let text = resp.text().await.ok()?;
    let trimmed = text.trim();
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
}

pub async fn get_public_ip_addresses(_params: CheckPublicIPParams) -> Result<PublicIPResponse> {
    let (ipv4, ipv6) =
        tokio::join!(fetch_text("https://api.ipify.org"), fetch_text("https://api6.ipify.org"),);
    let ipv6 = ipv6.filter(|v| v.contains(':'));
    Ok(PublicIPResponse { ipv4, ipv6 })
}

pub async fn check_public_ip_addresses() -> Result<Value> {
    let _ = get_public_ip_addresses(CheckPublicIPParams { refresh: Some(true) }).await?;
    Ok(Value::Null)
}

pub async fn get_cloud_state() -> Result<CloudState> {
    Ok(crate::cloud::manager::get_cloud_manager().get_cloud_state().await)
}

pub async fn deregister_device() -> Result<Value> {
    crate::cloud::manager::get_cloud_manager().deregister_device().await?;
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct CloudStateParam {
    pub state: String,
}

pub fn set_cloud_state(param: CloudStateParam) -> Result<Value> {
    let new_state = param
        .state
        .parse::<CloudConnectionState>()
        .map_err(|e| anyhow!("invalid cloud state: {}", e))?;
    crate::cloud::manager::get_cloud_manager().set_state(new_state);
    Ok(Value::Null)
}

#[derive(Deserialize)]
pub struct CloudUrlParams {
    #[serde(rename = "apiUrl")]
    pub api_url: String,
    #[serde(rename = "appUrl")]
    pub app_url: String,
}

pub async fn set_cloud_url(params: CloudUrlParams) -> Result<Value> {
    crate::cloud::manager::get_cloud_manager()
        .set_cloud_url(&params.api_url, &params.app_url)
        .await?;
    Ok(Value::Null)
}

pub fn get_tailscale_status() -> Result<Value> {
    let control_url = rkvm_net::tailscale::get_control_url();
    let status = rkvm_net::tailscale::get_status(&control_url)?;
    Ok(serde_json::to_value(status)?)
}

pub fn get_tailscale_control_url() -> Result<String> {
    Ok(rkvm_net::tailscale::get_control_url())
}

#[derive(Deserialize)]
pub struct TailscaleControlUrlParams {
    #[serde(rename = "controlURL")]
    pub control_url: String,
}

pub fn set_tailscale_control_url(params: TailscaleControlUrlParams) -> Result<Value> {
    let resolved = rkvm_net::tailscale::set_control_url(&params.control_url)?;
    info!("Tailscale control URL set to {}", resolved);
    Ok(Value::Null)
}
