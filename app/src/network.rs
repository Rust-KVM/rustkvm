use std::net::IpAddr;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

pub const NET_IF_NAME: &str = "eth0";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DhcpLease {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub netmask: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub broadcast: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routers: Vec<String>,
    #[serde(rename = "dns_servers", default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<String>,
    #[serde(rename = "ntp_servers", default, skip_serializing_if = "Vec::is_empty")]
    pub ntp: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expiry: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dhcp_client: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcIPv6Address {
    pub address: String,
    pub prefix: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_lifetime: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_lifetime: Option<DateTime<Utc>>,
    pub scope: i32,
    pub flags: i32,
    pub flag_secondary: bool,
    pub flag_permanent: bool,
    pub flag_temporary: bool,
    pub flag_stable_privacy: bool,
    pub flag_deprecated: bool,
    pub flag_optimistic: bool,
    pub flag_dad_failed: bool,
    pub flag_tentative: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InterfaceState {
    pub interface_name: String,
    pub hostname: String,
    pub mac_address: String,
    pub up: bool,
    pub online: bool,
    pub ipv4_ready: bool,
    pub ipv6_ready: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ipv4_address: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ipv6_address: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ipv6_link_local: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ipv6_gateway: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ipv4_addresses: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ipv6_addresses: Vec<RpcIPv6Address>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ntp_servers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dhcp_lease: Option<DhcpLease>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dhcp_lease6: Option<DhcpLease>,
    pub last_updated: DateTime<Utc>,
}

pub fn get_network_state() -> InterfaceState {
    let mut state = InterfaceState {
        interface_name: NET_IF_NAME.to_string(),
        last_updated: Utc::now(),
        ..Default::default()
    };

    let addrs = match if_addrs::get_if_addrs() {
        Ok(v) => v,
        Err(e) => {
            warn!("get_if_addrs failed: {e}");
            return state;
        }
    };

    for iface in addrs {
        if iface.name != NET_IF_NAME && !iface.name.starts_with("eth") {
            continue;
        }
        state.up = !iface.is_loopback();
        match iface.addr {
            if_addrs::IfAddr::V4(v) => {
                state.ipv4_addresses.push(v.ip.to_string());
                if state.ipv4_address.is_empty() {
                    state.ipv4_address = v.ip.to_string();
                }
                state.ipv4_ready = true;
            }
            if_addrs::IfAddr::V6(v) => {
                let ip = IpAddr::V6(v.ip);
                let addr = v.ip.to_string();
                if v.ip.segments()[0] & 0xffc0 == 0xfe80 {
                    if state.ipv6_link_local.is_empty() {
                        state.ipv6_link_local = addr;
                    }
                } else {
                    if state.ipv6_address.is_empty() {
                        state.ipv6_address = addr.clone();
                    }
                    state.ipv6_ready = true;
                }
                state.ipv6_addresses.push(RpcIPv6Address {
                    address: ip.to_string(),
                    prefix: String::new(),
                    valid_lifetime: None,
                    preferred_lifetime: None,
                    scope: 0,
                    flags: 0,
                    flag_secondary: false,
                    flag_permanent: false,
                    flag_temporary: false,
                    flag_stable_privacy: false,
                    flag_deprecated: false,
                    flag_optimistic: false,
                    flag_dad_failed: false,
                    flag_tentative: false,
                });
            }
        }
    }

    state.online = state.ipv4_ready || state.ipv6_ready;
    state
}

pub fn renew_dhcp_lease() -> Result<()> {
    debug!("renew_dhcp_lease: noop stub");
    Ok(())
}

pub fn spawn_state_monitor(period: std::time::Duration) {
    use tokio::time::{MissedTickBehavior, interval};
    tokio::spawn(async move {
        let mut tick = interval(period);
        tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
        let mut prev: Option<InterfaceState> = None;
        loop {
            tick.tick().await;
            let cur = get_network_state();
            let changed = match &prev {
                None => true,
                Some(p) => {
                    p.up != cur.up
                        || p.online != cur.online
                        || p.ipv4_address != cur.ipv4_address
                        || p.ipv6_address != cur.ipv6_address
                        || p.ipv4_addresses != cur.ipv4_addresses
                }
            };
            if changed {
                debug!(?cur, "network state changed, broadcasting");
                crate::api::broadcast_network_state(cur.clone()).await;
            }
            prev = Some(cur);
        }
    });
}

pub async fn init_network(fallback_hostname: &str) -> Result<()> {
    let cfg = crate::config::get_config_manager().get().await;
    let hostname = cfg
        .network_config
        .hostname
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_hostname);

    rustix::system::sethostname(hostname.as_bytes())
        .with_context(|| format!("sethostname({hostname}) failed"))?;

    info!(hostname, "system hostname set");
    Ok(())
}
