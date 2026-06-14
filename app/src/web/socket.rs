use std::sync::Arc;

use tracing::{info, warn};

use super::sio;
use super::types::WebRTCSessionRequest;
use crate::state::AppState;
use crate::webrtc;

pub(super) async fn handle_session_connect(
    socket: sio::SocketRef,
    sio::State(state): sio::State<Arc<AppState>>,
) {
    info!("[sid={}] connected", socket.id);
    let session_id = socket.id.to_string();
    socket.on_disconnect(handle_session_disconnect);

    state.sockets.write().await.insert(session_id, socket.clone());

    socket.on("offer", handle_socket_offer);
    socket.on("ice-candidate", handle_socket_ice_candidate);
}

async fn handle_session_disconnect(
    socket: sio::SocketRef,
    sio::State(state): sio::State<Arc<AppState>>,
) {
    info!("[sid={}] disconnected", socket.id);
    let session_id = socket.id.to_string();
    state.remove_session(&session_id).await;
}

async fn handle_socket_offer(
    socket: sio::SocketRef,
    sio::Data(data): sio::Data<String>,
    sio::State(state): sio::State<Arc<AppState>>,
    ack: sio::AckSender,
) {
    info!("[sid={}] received offer", socket.id);

    let request = match serde_json::from_str::<WebRTCSessionRequest>(&data) {
        Ok(req) => req,
        Err(e) => {
            warn!("[sid={}] Failed to parse offer: {}", socket.id, e);
            let _ = ack.send(&serde_json::json!({"error": "Invalid offer format"}).to_string());
            return;
        }
    };

    let session_id = socket.id.to_string();
    let config = webrtc::SessionConfig {
        ice_servers: request.ice_servers,
        local_ip: request.ip.and_then(|ip| ip.parse().ok()),
        is_cloud: false,
        app_state: state.clone(),
    };

    let session = match webrtc::get_webrtc_api().await.new_session(config, session_id.clone()).await
    {
        Ok(s) => s,
        Err(e) => {
            warn!("[sid={}] Failed to create WebRTC session: {}", socket.id, e);
            let _ = ack.send(&serde_json::json!({"error": e.to_string()}).to_string());
            return;
        }
    };

    state.add_session(session.clone()).await;

    match session.exchange_offer(&request.sd).await {
        Ok(answer) => {
            let response = serde_json::json!({ "type": "answer", "data": answer });
            let _ = ack.send(&response.to_string());
            info!("[sid={}] WebRTC session created successfully", socket.id);
        }
        Err(e) => {
            warn!("[sid={}] Failed to exchange offer: {}", socket.id, e);
            state.remove_session(&session_id).await;
            let _ = ack.send(&serde_json::json!({"error": e.to_string()}).to_string());
        }
    }
}

async fn handle_socket_ice_candidate(
    socket: sio::SocketRef,
    sio::Data(data): sio::Data<String>,
    sio::State(state): sio::State<Arc<AppState>>,
) {
    info!("[sid={}] received ICE candidate: {}", socket.id, data);

    let message: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(e) => {
            warn!("[sid={}] Failed to parse ICE candidate JSON: {}", socket.id, e);
            return;
        }
    };

    if message.get("type").and_then(|v| v.as_str()) != Some("new-ice-candidate") {
        return;
    }

    let Some(candidate_data) = message.get("data") else {
        warn!("[sid={}] Missing candidate data in new-ice-candidate message", socket.id);
        return;
    };

    let session_id = socket.id.to_string();
    let Some(session) = state.get_session(&session_id).await else {
        warn!("[sid={}] No session found for ICE candidate", socket.id);
        return;
    };

    let candidate_json = match serde_json::to_string(candidate_data) {
        Ok(s) => s,
        Err(e) => {
            warn!("[sid={}] Failed to serialize candidate: {}", socket.id, e);
            return;
        }
    };

    match session.add_ice_candidate(&candidate_json).await {
        Ok(_) => info!("[sid={}] ICE candidate added successfully", socket.id),
        Err(e) => warn!("[sid={}] Failed to add ICE candidate: {}", socket.id, e),
    }
}
