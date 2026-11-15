use serde::Serialize;
use serde_json::Value;
use tracing::{debug, warn};
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_state::RTCDataChannelState;

use crate::hardware::usb::KeyboardState as HidKeyboardState;
use crate::webrtc::{get_current_session, get_rpc_channel};

/// Sends a JSON-RPC notification on one RPC data channel.
pub async fn notify(channel: &RTCDataChannel, method: &str, params: Value) -> anyhow::Result<()> {
    let event = serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params });
    channel.send_text(event.to_string()).await?;
    Ok(())
}

/// Sends a JSON-RPC notification to the current session. Events raised while
/// nobody is connected, or before the RPC channel opens, are dropped: the
/// channel's open handler pushes the full state again.
pub async fn emit(method: &str, params: impl Serialize) {
    let Some(session_id) = get_current_session().await else { return };
    let Some(channel) = get_rpc_channel(&session_id).await else { return };
    if channel.ready_state() != RTCDataChannelState::Open {
        debug!(method, "event dropped: RPC channel not open");
        return;
    }
    let params = match serde_json::to_value(params) {
        Ok(params) => params,
        Err(e) => {
            warn!(method, "cannot serialize event: {e}");
            return;
        }
    };
    if let Err(e) = notify(&channel, method, params).await {
        warn!(method, "failed to send event: {e}");
    }
}

pub async fn broadcast_usb_state(state: String) {
    let _ = super::handlers::usb::set_usb_state(state.clone());
    emit("usbState", state).await;
}

pub async fn broadcast_network_state(state: crate::network::InterfaceState) {
    emit("networkState", state).await;
}

pub async fn broadcast_will_reboot(reason: &str) {
    emit("willReboot", serde_json::json!({ "reason": reason })).await;
}

pub async fn broadcast_failsafe_mode() {
    if let Some(notification) = crate::failsafe::notification() {
        emit("failsafeMode", notification).await;
    }
}

pub async fn broadcast_keyboard_led_state(state: HidKeyboardState) {
    let _ = super::handlers::hid::set_keyboard_led_state(state);
    emit("keyboardLedState", state).await;
}
