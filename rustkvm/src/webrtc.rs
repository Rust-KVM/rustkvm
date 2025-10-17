use std::net::IpAddr;
use std::sync::Arc;

use anyhow::Context;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use webrtc::api::APIBuilder;
use webrtc::api::interceptor_registry::register_default_interceptors;
use webrtc::api::media_engine::{MIME_TYPE_H264, MIME_TYPE_OPUS, MediaEngine};
use webrtc::api::setting_engine::SettingEngine;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::RTCIceCandidate;
use webrtc::ice_transport::ice_connection_state::RTCIceConnectionState;
use webrtc::interceptor::registry::Registry;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::rtp_transceiver::rtp_codec::RTCRtpCodecCapability;
use webrtc::track::track_local::TrackLocal;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

use crate::data_channel::DataChannelManager;
use crate::hardware::usb::storage as storage_mod;
use crate::jsonrpc::{JsonRpcProcessor, create_default_registry};
use crate::session::Session;
use crate::state::AppState;
use crate::video;

/// Session configuration for WebRTC connections
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub ice_servers: Option<Vec<String>>,
    pub local_ip: Option<IpAddr>,
    pub is_cloud: bool,
    pub app_state: Arc<AppState>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ice_servers: None,
            local_ip: None,
            is_cloud: false,
            app_state: Arc::new(AppState::default()),
        }
    }
}

/// WebRTC API instance shared across sessions
pub struct WebRTCApi;

impl WebRTCApi {
    /// Create new WebRTC API instance
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self)
    }

    /// Create new WebRTC session with the given configuration
    pub async fn new_session(
        &self,
        config: SessionConfig,
        session_id: String,
    ) -> anyhow::Result<Session> {
        let mut setting_engine = SettingEngine::default();
        let mut ice_servers = vec![];

        info!("Creating new WebRTC session with id: {}", session_id);

        if config.is_cloud {
            if let Some(servers) = &config.ice_servers {
                ice_servers = servers
                    .iter()
                    .map(|url| webrtc::ice_transport::ice_server::RTCIceServer {
                        urls: vec![url.clone()],
                        ..Default::default()
                    })
                    .collect();
                info!("Using ICE servers provided by cloud: {:?}", servers);
            } else {
                info!("ICE servers not provided by cloud");
            }

            if let Some(local_ip) = config.local_ip {
                setting_engine.set_nat_1to1_ips(
                    vec![local_ip.to_string()],
                    webrtc::ice_transport::ice_candidate_type::RTCIceCandidateType::Srflx,
                );
                info!("Setting NAT 1-to-1 IPs with local IP: {}", local_ip);
            } else {
                info!("Local IP address not provided, won't set NAT 1-to-1 IPs");
            }
        }

        // Build API with media engine (codecs) and default interceptors
        let mut media_engine = MediaEngine::default();
        media_engine.register_default_codecs().context("register_default_codecs failed")?;

        let mut registry = Registry::new();
        registry = register_default_interceptors(registry, &mut media_engine)
            .context("register_default_interceptors failed")?;

        let api = APIBuilder::new()
            .with_setting_engine(setting_engine)
            .with_media_engine(media_engine)
            .with_interceptor_registry(registry)
            .build();

        let configuration = RTCConfiguration { ice_servers, ..Default::default() };

        let peer_connection = Arc::new(api.new_peer_connection(configuration).await?);

        // Create video track
        let video_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability { mime_type: MIME_TYPE_H264.to_owned(), ..Default::default() },
            "video".to_owned(),
            "rustkvm".to_owned(),
        ));

        // Create audio track (Opus)
        let audio_track = Arc::new(TrackLocalStaticSample::new(
            RTCRtpCodecCapability {
                mime_type: MIME_TYPE_OPUS.to_owned(),
                clock_rate: 48000,
                channels: 2,
                sdp_fmtp_line: "minptime=10;useinbandfec=1".to_string(),
                ..Default::default()
            },
            "audio".to_owned(),
            "rustkvm".to_owned(),
        ));

        info!("Created video and audio tracks");

        // Add video track to peer connection
        let video_sender = peer_connection
            .add_track(Arc::clone(&video_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;

        // Add audio track to peer connection
        let audio_sender = peer_connection
            .add_track(Arc::clone(&audio_track) as Arc<dyn TrackLocal + Send + Sync>)
            .await?;

        info!("Added video and audio tracks to peer connection");

        // Verify track was added by checking senders immediately
        let senders = peer_connection.get_senders().await;
        info!("Peer connection senders after adding video track: {}", senders.len());
        for (i, sender) in senders.iter().enumerate() {
            if let Some(track) = sender.track().await {
                info!("Sender {}: track_id={}, kind={}", i, track.id(), track.kind());
            } else {
                info!("Sender {}: no track", i);
            }
        }

        // Start RTCP reading tasks
        tokio::spawn(async move {
            let mut rtcp_buf = vec![0u8; 1500];
            while let Ok((packet, attributes)) = video_sender.read(&mut rtcp_buf).await {
                debug!("Video RTCP: {:?}, attributes: {:?}", packet, attributes);
            }
        });

        tokio::spawn(async move {
            let mut rtcp_buf = vec![0u8; 1500];
            while let Ok((packet, attributes)) = audio_sender.read(&mut rtcp_buf).await {
                debug!("Audio RTCP: {:?}, attributes: {:?}", packet, attributes);
            }
        });

        let mut session = Session::new(session_id);
        session.peer_connection = Some(Arc::clone(&peer_connection));
        session.video_track = Some(video_track);
        session.audio_track = Some(audio_track);

        let app_state_for_close = config.app_state.clone();

        // Set up connection state change handler
        let session_id_clone = session.id.clone();
        let video_track_clone = session.video_track.clone();
        let audio_track_clone = session.audio_track.clone();
        // Add isConnected tracking per session
        let is_connected = Arc::new(RwLock::new(false));
        let is_connected_clone = is_connected.clone();

        peer_connection.on_ice_connection_state_change(Box::new(
            move |connection_state: RTCIceConnectionState| {
                let session_id = session_id_clone.clone();
                let video_track = video_track_clone.clone();
                let audio_track = audio_track_clone.clone();
                let is_connected = is_connected_clone.clone();
                let app_state = app_state_for_close.clone();
                Box::pin(async move {
                    info!(
                        "ICE Connection State has changed: {} for session: {}",
                        connection_state, session_id
                    );

                    match connection_state {
                        RTCIceConnectionState::Connected => {
                            let mut connected = is_connected.write().await;
                            if !*connected {
                                *connected = true;
                                drop(connected);
                                info!("WebRTC session {} connected", session_id);
                                // actionSessions++ and session management
                                increment_session_counter().await;
                                // Bridge video -> track (GStreamer -> channel -> WebRTC)
                                if let Some(track) = video_track.clone() {
                                    video::attach_webrtc_sink(track).await;
                                    info!("Video track attached to WebRTC session {}", session_id);
                                }
                                // Bridge audio -> track (GStreamer -> channel -> WebRTC)
                                if let Some(track) = audio_track.clone() {
                                    video::attach_audio_sink(track).await;
                                    info!("Audio track attached to WebRTC session {}", session_id);
                                }
                                // Set as current session
                                set_current_session(Some(session_id)).await;
                            }
                        }
                        RTCIceConnectionState::Failed => {
                            warn!("ICE Connection State is failed, closing peerConnection for session {}", session_id);
                            // Note: Peer connection will be cleaned up when the session is dropped
                        }
                        RTCIceConnectionState::Closed => {
                            info!("ICE Connection State is closed for session {}", session_id);
                            // Clear current session if this was the current one
                            let current = get_current_session().await;
                            if current == Some(session_id.clone()) {
                                set_current_session(None).await;
                            }

                            // Clean up RPC channel for this session
                            remove_rpc_channel(&session_id).await;

                            // Handle virtual media unmounting if needed
                            // Virtual media will be automatically unmounted when session ends
                            // Only decrement if was connected
                            let mut connected = is_connected.write().await;
                            if *connected {
                                *connected = false;
                                drop(connected);
                                // Decrement session counter
                                decrement_session_counter().await;
                                // Only detach sink if there is no active current session
                                if get_current_session().await.is_none() {
                                    video::detach_webrtc_sink().await;
                                }
                                // Auto-unmount virtual media if last session closed and source is WebRTC
                                if get_current_session().await.is_none()
                                    && let Some(st) = storage_mod::get_virtual_media_state()
                                        && matches!(st.source, storage_mod::VirtualMediaSource::WebRTC) {
                                            tokio::spawn(async move {
                                                if let Err(e) = storage_mod::unmount_image().await {
                                                    warn!("failed to auto unmount WebRTC media: {}", e);
                                                } else {
                                                    info!("auto unmounted WebRTC virtual media on last session close");
                                                }
                                            });
                                        }
                            }

                            // Remove session from AppState to free resources
                            app_state.remove_session(&session_id).await;
                            info!("Removed session {} on ICE Closed", session_id);
                        }
                        _ => {}
                    }
                })
            },
        ));

        // Set up ICE candidate handler
        let session_id_clone = session.id.clone();
        let app_state_clone = config.app_state.clone();
        peer_connection.on_ice_candidate(Box::new(move |candidate: Option<RTCIceCandidate>| {
            let session_id = session_id_clone.clone();
            let app_state = app_state_clone.clone();
            Box::pin(async move {
                if let Some(candidate) = candidate {
                    info!("WebRTC peerConnection has a new ICE candidate: {:?}", candidate);

                    // Send ICE candidate through Socket.IO signaling channel
                    if let Err(e) = send_ice_candidate(&session_id, &candidate, app_state).await {
                        warn!("Failed to send ICE candidate: {}", e);
                    }
                } else {
                    debug!("ICE gathering completed (candidate is None)");
                }
            })
        }));

        // Set up data channel handler with proper routing
        let session_clone = session.clone();
        peer_connection.on_data_channel(Box::new(move |data_channel: Arc<RTCDataChannel>| {
            let session = session_clone.clone();
            let label = data_channel.label().to_string();
            let channel_id = data_channel.id();
            Box::pin(async move {
                info!(
                    "New DataChannel label='{}' id={} for session: {}",
                    label, channel_id, session.id
                );

                let manager = DataChannelManager::new();
                manager.route_data_channel(data_channel, &session).await;
            })
        }));

        Ok(session)
    }
}

/// Global WebRTC API instance
static WEBRTC_API: tokio::sync::OnceCell<WebRTCApi> = tokio::sync::OnceCell::const_new();

/// Initialize global WebRTC API
pub async fn init_webrtc_api() -> anyhow::Result<()> {
    let api = WebRTCApi::new()?;
    WEBRTC_API.set(api).map_err(|_| anyhow::anyhow!("WebRTC API already initialized"))?;
    info!("WebRTC API initialized");
    Ok(())
}

/// Get global WebRTC API instance
pub async fn get_webrtc_api() -> &'static WebRTCApi {
    WEBRTC_API.get().expect("WebRTC API not initialized")
}

/// Session counter for managing active sessions
static SESSION_COUNTER: RwLock<i32> = RwLock::const_new(0);

/// Current active session
static CURRENT_SESSION: RwLock<Option<String>> = RwLock::const_new(None);

/// Global RPC channel storage for sessions
use std::collections::HashMap;
static RPC_CHANNELS: tokio::sync::OnceCell<
    RwLock<HashMap<String, Arc<webrtc::data_channel::RTCDataChannel>>>,
> = tokio::sync::OnceCell::const_new();

/// Create JSON-RPC processor for sending events
fn create_rpc_processor() -> JsonRpcProcessor {
    let registry = Arc::new(create_default_registry());
    JsonRpcProcessor::new(registry)
}

/// Trigger OTA state update
pub async fn trigger_ota_state_update() {
    info!("Triggering OTA state update");

    if let Some(session_id) = get_current_session().await {
        // Create a session with RPC channel for sending events
        let mut session = Session::new(session_id.clone());
        if let Some(rpc_channel) = get_rpc_channel(&session_id).await {
            if rpc_channel.ready_state()
                != webrtc::data_channel::data_channel_state::RTCDataChannelState::Open
            {
                warn!("RPC channel not open yet for session: {}", session_id);
                return;
            }
            session.rpc_channel = Some(rpc_channel);
        }
        let processor = create_rpc_processor();

        // Send update status event
        let params = serde_json::json!({
            "updateAvailable": false,
            "currentVersion": "1.0.0",
            "error": null
        });

        if let Err(e) = processor.send_event("updateStatusChanged", Some(params), &session).await {
            warn!("Failed to send OTA update status: {}", e);
        }
    }
}

/// Trigger video state update
pub async fn trigger_video_state_update() {
    info!("Triggering video state update");
    // Call video module's internal update function
    video::trigger_video_state_update_rpc().await;
}

/// Trigger USB state update
pub async fn trigger_usb_state_update() {
    info!("Triggering USB state update");

    if let Some(session_id) = get_current_session().await {
        // Create a session with RPC channel for sending events
        let mut session = Session::new(session_id.clone());
        if let Some(rpc_channel) = get_rpc_channel(&session_id).await {
            if rpc_channel.ready_state()
                != webrtc::data_channel::data_channel_state::RTCDataChannelState::Open
            {
                warn!("RPC channel not open yet for session: {}", session_id);
                return;
            }
            session.rpc_channel = Some(rpc_channel);
        }
        let processor = create_rpc_processor();

        // Get USB device states using our handlers
        let usb_devices = match crate::jsonrpc::handlers::get_usb_devices() {
            Ok(devices) => devices,
            Err(_) => crate::jsonrpc::handlers::UsbDevicesResponse {
                absolute_mouse: true,
                relative_mouse: false,
                keyboard: true,
                mass_storage: true,
            },
        };
        let usb_emulation = crate::jsonrpc::handlers::get_usb_emulation_state().unwrap_or(false);

        let params = serde_json::json!({
            "devices": usb_devices,
            "emulationEnabled": usb_emulation
        });

        if let Err(e) = processor.send_event("usbStateChanged", Some(params), &session).await {
            warn!("Failed to send USB state update: {}", e);
        }
    }
}

/// Handle first session connected
pub async fn on_first_session_connected() {
    info!("First WebRTC session connected - starting video");
    if let Err(e) = video::write_ctrl_action("start_video").await {
        warn!("Failed to write start_video action: {}", e);
    }
}

/// Handle last session disconnected
pub async fn on_last_session_disconnected() {
    info!("Last WebRTC session disconnected - stopping video");
    if let Err(e) = video::write_ctrl_action("stop_video").await {
        warn!("Failed to write stop_video action: {}", e);
    }
}

/// Handle session count change
pub async fn on_active_sessions_changed() {
    let count = *SESSION_COUNTER.read().await;
    info!("Active sessions count changed to: {}", count);

    // Send session count update to current session if available
    if let Some(session_id) = get_current_session().await {
        let mut session = Session::new(session_id.clone());
        if let Some(rpc_channel) = get_rpc_channel(&session_id).await {
            if rpc_channel.ready_state()
                != webrtc::data_channel::data_channel_state::RTCDataChannelState::Open
            {
                debug!("RPC channel not open yet for session: {}", session_id);
                return;
            }
            session.rpc_channel = Some(rpc_channel);
        }
        let processor = create_rpc_processor();

        let params = serde_json::json!({
            "activeSessionCount": count,
            "hasActiveSessions": count > 0
        });

        if let Err(e) = processor.send_event("sessionCountChanged", Some(params), &session).await {
            warn!("Failed to send session count update: {}", e);
        }
    }
}

/// Increment session counter
pub async fn increment_session_counter() {
    let mut counter = SESSION_COUNTER.write().await;
    *counter += 1;
    let count = *counter;
    drop(counter);

    if count == 1 {
        on_first_session_connected().await;
    }
    on_active_sessions_changed().await;
}

/// Decrement session counter
pub async fn decrement_session_counter() {
    let mut counter = SESSION_COUNTER.write().await;
    *counter = (*counter - 1).max(0);
    let count = *counter;
    drop(counter);

    if count == 0 {
        on_last_session_disconnected().await;
    }
    on_active_sessions_changed().await;
}

/// Get current session ID
pub async fn get_current_session() -> Option<String> {
    CURRENT_SESSION.read().await.clone()
}

/// Set current session ID
pub async fn set_current_session(session_id: Option<String>) {
    *CURRENT_SESSION.write().await = session_id;
}

/// Store RPC channel for a session
pub async fn store_rpc_channel(
    session_id: String,
    channel: Arc<webrtc::data_channel::RTCDataChannel>,
) {
    let channels = RPC_CHANNELS.get_or_init(|| async { RwLock::new(HashMap::new()) }).await;
    channels.write().await.insert(session_id, channel);
}

/// Get RPC channel for a session
pub async fn get_rpc_channel(
    session_id: &str,
) -> Option<Arc<webrtc::data_channel::RTCDataChannel>> {
    let channels = RPC_CHANNELS.get_or_init(|| async { RwLock::new(HashMap::new()) }).await;
    channels.read().await.get(session_id).cloned()
}

/// Remove RPC channel for a session
pub async fn remove_rpc_channel(
    session_id: &str,
) -> Option<Arc<webrtc::data_channel::RTCDataChannel>> {
    let channels = RPC_CHANNELS.get_or_init(|| async { RwLock::new(HashMap::new()) }).await;
    channels.write().await.remove(session_id)
}

/// Send ICE candidate through Socket.IO signaling channel
async fn send_ice_candidate(
    session_id: &str,
    candidate: &RTCIceCandidate,
    app_state: Arc<AppState>,
) -> anyhow::Result<()> {
    use serde_json::json;

    let candidate_init = candidate.to_json()?;
    let message = json!({
        "type": "new-ice-candidate",
        "data": candidate_init
    });

    // Emit ICE candidate to the specific session via Socket.IO
    info!("Sending ICE candidate for session: {}", session_id);

    // // Get the socket from AppState and emit the message
    // if let Some(socket) = app_state.sockets.read().await.get(session_id) {
    //     if let Err(e) = socket.emit("ice-candidate", &message) {
    //         warn!("Socket.IO emit failed, queuing for WebSocket: {}", e);
    //         let message_str = message.to_string();
    //         app_state.queue_ice_candidate(session_id, message_str).await;
    //         info!("Queued ICE candidate for WebSocket session: {}", session_id);
    //     } else {
    //         info!("Sent ICE candidate via Socket.IO for session: {}", session_id);
    //     }
    // } else {
    //     let message_str = message.to_string();
    //     app_state.queue_ice_candidate(session_id, message_str).await;
    //     info!("Queued ICE candidate for WebSocket session: {}", session_id);
    // }

    // Ok(())

    // 1) Try cloud websocket first
    if let Err(e) = crate::cloud::websocket::cloud_ws_send_json(&message).await {
        debug!("cloud ws send failed: {}", e);

        // 2) Fallback to Socket.IO (browser/local signaling)
        if let Some(socket) = app_state.sockets.read().await.get(session_id) {
            if let Err(e) = socket.emit("ice-candidate", &message) {
                warn!("Socket.IO emit failed, queue for local WS: {}", e);
                app_state.queue_ice_candidate(session_id, message.to_string()).await;
            } else {
                info!("Sent ICE candidate via Socket.IO for session: {}", session_id);
                return Ok(());
            }
        } else {
            // 3) Final fallback: queue to local websocket
            app_state.queue_ice_candidate(session_id, message.to_string()).await;
        }
    }

    Ok(())
}

/// Handle session takeover: send otherSessionConnected to old session and close it after delay
pub async fn handle_session_takeover(
    app_state: std::sync::Arc<crate::state::AppState>,
    new_session_id: &str,
) {
    let maybe_old = app_state.get_current_session().await;

    if let Some(ref old_id) = maybe_old
        && *old_id != new_session_id
    {
        // Send otherSessionConnected event to old session
        if let Some(old_session) = app_state.get_session(old_id).await {
            if let Some(rpc_channel) = get_rpc_channel(old_id).await {
                let mut session_with_rpc = old_session.clone();
                session_with_rpc.rpc_channel = Some(rpc_channel);

                let processor = crate::jsonrpc::JsonRpcProcessor::new(std::sync::Arc::new(
                    crate::jsonrpc::create_default_registry(),
                ));

                if let Err(e) =
                    processor.send_event("otherSessionConnected", None, &session_with_rpc).await
                {
                    tracing::warn!(
                        "Failed to send otherSessionConnected to session {}: {}",
                        old_id,
                        e
                    );
                } else {
                    tracing::info!("Sent otherSessionConnected to old session {}", old_id);
                }
            } else {
                tracing::warn!("No RPC channel available for old session {}", old_id);
            }
        }

        // Close old session after 1 second delay
        let app_state_cl = app_state.clone();
        let old_id_cl = old_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            if let Some(old_sess) = app_state_cl.get_session(&old_id_cl).await {
                if let Some(pc) = old_sess.peer_connection {
                    let _ = pc.close().await;
                }
                app_state_cl.remove_session(&old_id_cl).await;
                tracing::info!("Closed previous session {}", old_id_cl);
            }
        });
    }
}
