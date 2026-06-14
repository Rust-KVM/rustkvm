use std::str::FromStr;

use macaddr::MacAddr6;
use tokio::net::UdpSocket;
use tracing::{info, warn};

use crate::error::{Error, Result};

pub async fn send_magic_packet(mac_address: &str, broadcast_ip: Option<&str>) -> Result<()> {
    let mac = MacAddr6::from_str(mac_address)
        .map_err(|e| Error::InvalidMac { mac: mac_address.to_string(), reason: e.to_string() })?;

    let mut packet: Vec<u8> = Vec::with_capacity(6 + 16 * 6);
    packet.extend_from_slice(&[0xFF; 6]);
    for _ in 0..16 {
        packet.extend_from_slice(mac.as_bytes());
    }

    let socket = UdpSocket::bind(("0.0.0.0", 0)).await?;
    socket.set_broadcast(true)?;

    let target_ip = broadcast_ip.unwrap_or("255.255.255.255");
    let target = (target_ip, 9u16);
    let sent = socket.send_to(&packet, target).await?;
    if sent != packet.len() {
        warn!("partial WOL packet sent");
    }

    info!(mac = mac_address, target = target_ip, "WOL magic packet sent");
    Ok(())
}
