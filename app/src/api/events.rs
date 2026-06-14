use tracing::warn;

use super::registry::JsonRpcProcessor;
use super::registry_builder::default_registry;
use crate::hardware::usb::KeyboardState as HidKeyboardState;
use crate::session::Session;
use crate::webrtc::{get_current_session, get_rpc_channel};

pub async fn broadcast_usb_state(state: String) {
    let _ = super::handlers::usb::set_usb_state(state.clone());
    if let Some(session_id) = get_current_session().await {
        let mut session = Session::new(session_id.clone());
        if let Some(rpc_channel) = get_rpc_channel(&session_id).await {
            session.rpc_channel = Some(rpc_channel);
        }
        let processor = JsonRpcProcessor::new(default_registry());
        let params = serde_json::to_value(state).unwrap_or(serde_json::json!({}));
        if let Err(e) = processor.send_event("usbState", Some(params), &session).await {
            warn!("Failed to send usbState event: {}", e);
        }
    }
}

pub async fn broadcast_network_state(state: crate::network::InterfaceState) {
    let Some(session_id) = get_current_session().await else { return };
    let mut session = Session::new(session_id.clone());
    if let Some(rpc_channel) = get_rpc_channel(&session_id).await {
        session.rpc_channel = Some(rpc_channel);
    }
    let processor = JsonRpcProcessor::new(default_registry());
    let params = serde_json::to_value(state).unwrap_or(serde_json::json!({}));
    if let Err(e) = processor.send_event("networkState", Some(params), &session).await {
        warn!("Failed to send networkState event: {}", e);
    }
}

pub async fn broadcast_will_reboot(reason: &str) {
    let Some(session_id) = get_current_session().await else { return };
    let mut session = Session::new(session_id.clone());
    if let Some(rpc_channel) = get_rpc_channel(&session_id).await {
        session.rpc_channel = Some(rpc_channel);
    }
    let processor = JsonRpcProcessor::new(default_registry());
    let params = serde_json::json!({ "reason": reason });
    if let Err(e) = processor.send_event("willReboot", Some(params), &session).await {
        warn!("Failed to send willReboot event: {}", e);
    }
}

pub async fn broadcast_failsafe_mode() {
    let Some(notification) = crate::failsafe::notification() else { return };
    let Some(session_id) = get_current_session().await else { return };
    let mut session = Session::new(session_id.clone());
    if let Some(rpc_channel) = get_rpc_channel(&session_id).await {
        session.rpc_channel = Some(rpc_channel);
    }
    let processor = JsonRpcProcessor::new(default_registry());
    let params = serde_json::to_value(notification).unwrap_or(serde_json::json!({}));
    if let Err(e) = processor.send_event("failsafeMode", Some(params), &session).await {
        warn!("Failed to send failsafeMode event: {}", e);
    }
}

pub async fn broadcast_keyboard_led_state(state: HidKeyboardState) {
    let _ = super::handlers::hid::set_keyboard_led_state(state);
    if let Some(session_id) = get_current_session().await {
        let mut session = Session::new(session_id.clone());
        if let Some(rpc_channel) = get_rpc_channel(&session_id).await {
            session.rpc_channel = Some(rpc_channel);
        }
        let processor = JsonRpcProcessor::new(default_registry());
        let params = serde_json::to_value(state).unwrap_or(serde_json::json!({}));
        if let Err(e) = processor.send_event("keyboardLedState", Some(params), &session).await {
            warn!("Failed to send keyboardLedState event: {}", e);
        }
    }
}
