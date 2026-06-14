use std::sync::Arc;

use anyhow::Result;
use salvo::prelude::*;
use salvo::websocket::{Message, WebSocket, WebSocketUpgrade};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::global_app_state;
use super::types::{WebRTCSessionRequest, WebRTCSessionResponse};
use crate::state::AppState;
use crate::webrtc;

#[endpoint]
pub(super) async fn handle_webrtc_session(
    req: &mut Request,
) -> Result<Json<WebRTCSessionResponse>, StatusError> {
    info!("Received WebRTC session request");

    let request: WebRTCSessionRequest = req
        .parse_json()
        .await
        .map_err(|e| StatusError::bad_request().brief(format!("Invalid JSON: {e}")))?;

    let app_state = global_app_state()
        .ok_or_else(|| StatusError::internal_server_error().brief("AppState not initialized"))?;

    let webrtc_api = webrtc::get_webrtc_api().await;
    let session_config = webrtc::SessionConfig {
        ice_servers: request.ice_servers,
        local_ip: request.ip.and_then(|ip| ip.parse().ok()),
        is_cloud: false,
        app_state: app_state.clone(),
    };

    let session =
        webrtc_api.new_session(session_config, Uuid::new_v4().to_string()).await.map_err(|e| {
            warn!("Failed to create WebRTC session: {}", e);
            StatusError::internal_server_error().brief(format!("Failed to create session: {e}"))
        })?;

    let answer = session.exchange_offer(&request.sd).await.map_err(|e| {
        warn!("Failed to exchange offer: {}", e);
        StatusError::internal_server_error().brief(format!("Failed to exchange offer: {e}"))
    })?;

    info!("WebRTC session created successfully with id: {}", session.id);

    webrtc::handle_session_takeover(app_state.clone(), &session.id).await;

    app_state.add_session(session.clone()).await;
    app_state.set_current_session(Some(session.id.clone())).await;

    Ok(Json(WebRTCSessionResponse { sd: answer }))
}

#[endpoint]
pub(super) async fn handle_webrtc_signaling_client(
    req: &mut Request,
    res: &mut Response,
) -> Result<(), StatusError> {
    let source = req.remote_addr().to_string();
    let connection_id = Uuid::new_v4().to_string();

    info!("WebRTC WebSocket connection from {}", source);

    let _ = WebSocketUpgrade::new()
        .upgrade(req, res, |ws| async move {
            if let Err(e) = handle_webrtc_websocket(ws, connection_id, source).await {
                warn!("WebSocket handler error: {}", e);
            }
        })
        .await;

    Ok(())
}

async fn handle_webrtc_websocket(
    mut ws: WebSocket,
    connection_id: String,
    source: String,
) -> Result<()> {
    let device_metadata = serde_json::json!({
        "type": "device-metadata",
        "data": { "deviceVersion": env!("CARGO_PKG_VERSION") }
    });

    ws.send(Message::text(device_metadata.to_string())).await?;
    info!("WebRTC WebSocket connection {} established", connection_id);

    let app_state =
        global_app_state().ok_or_else(|| anyhow::anyhow!("Global AppState not initialized"))?;

    while let Some(msg) = ws.recv().await {
        let msg = match msg {
            Ok(m) => m,
            Err(_) => {
                info!("WebRTC WebSocket connection {} closed", connection_id);
                break;
            }
        };

        if msg.is_text() {
            if let Ok(text) = msg.as_str()
                && let Err(e) = handle_webrtc_websocket_message(
                    text,
                    &connection_id,
                    &source,
                    &app_state,
                    &mut ws,
                )
                .await
            {
                warn!("Failed to handle WebRTC message: {}", e);
            }
        } else if msg.is_close() {
            info!("WebRTC WebSocket connection {} closed", connection_id);
            break;
        } else if msg.is_ping() {
            let ping_data = msg.as_bytes().to_vec();
            if ws.send(Message::pong(ping_data)).await.is_err() {
                break;
            }
        }

        let ice_candidates = app_state.get_ice_candidates(&connection_id).await;
        if !ice_candidates.is_empty() {
            let mut sent_count = 0;
            for candidate in ice_candidates {
                if ws.send(Message::text(candidate)).await.is_ok() {
                    sent_count += 1;
                }
            }
            if sent_count > 0 {
                debug!("Sent {} ICE candidates for session {}", sent_count, connection_id);
            }
        }
    }

    if let Some(sess) = app_state.get_session(&connection_id).await {
        if let Some(pc) = sess.peer_connection {
            let _ = pc.close().await;
        }
        app_state.remove_session(&connection_id).await;
        info!("Removed session {} on websocket close", connection_id);
    }

    app_state.sockets.write().await.remove(&connection_id);
    app_state.websocket_ice_queue.write().await.remove(&connection_id);

    Ok(())
}

async fn handle_webrtc_websocket_message(
    message: &str,
    connection_id: &str,
    source: &str,
    app_state: &Arc<AppState>,
    ws: &mut WebSocket,
) -> Result<()> {
    if message == "ping" {
        ws.send(Message::text("pong")).await?;
        return Ok(());
    }

    let parsed: serde_json::Value = serde_json::from_str(message)?;

    let Some(msg_type) = parsed.get("type").and_then(|v| v.as_str()) else {
        return Ok(());
    };

    match msg_type {
        "offer" => {
            let Some(data) = parsed.get("data") else { return Ok(()) };
            let request: WebRTCSessionRequest = serde_json::from_value(data.clone())?;

            let config = webrtc::SessionConfig {
                ice_servers: request.ice_servers,
                local_ip: request.ip.and_then(|ip| ip.parse().ok()),
                is_cloud: false,
                app_state: app_state.clone(),
            };

            let session = match webrtc::get_webrtc_api()
                .await
                .new_session(config, connection_id.to_string())
                .await
            {
                Ok(s) => s,
                Err(e) => {
                    warn!("Failed to create WebRTC session: {}", e);
                    return Ok(());
                }
            };

            app_state.add_session(session.clone()).await;

            match session.exchange_offer(&request.sd).await {
                Ok(answer) => {
                    let response = serde_json::json!({ "type": "answer", "data": answer });
                    ws.send(Message::text(response.to_string())).await?;
                    info!("WebRTC session created for connection {}, answer sent", connection_id);

                    webrtc::handle_session_takeover(app_state.clone(), connection_id).await;
                    app_state.set_current_session(Some(connection_id.into())).await;
                }
                Err(e) => {
                    warn!("Failed to exchange offer: {}", e);
                    app_state.remove_session(connection_id).await;
                }
            }
        }
        "new-ice-candidate" => {
            if let Some(data) = parsed.get("data")
                && let Some(session) = app_state.get_session(connection_id).await
            {
                let candidate_json = serde_json::to_string(data)?;
                if let Err(e) = session.add_ice_candidate(&candidate_json).await {
                    warn!("Failed to add ICE candidate: {}", e);
                }
            }
        }
        _ => {
            warn!("Unknown message type from {}: {}", source, msg_type);
        }
    }

    Ok(())
}
