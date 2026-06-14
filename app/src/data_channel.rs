use std::sync::Arc;

use anyhow::anyhow;
use parking_lot::Mutex;
use tracing::{debug, info, warn};
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_message::DataChannelMessage;

use crate::api::{JsonRpcProcessor, default_registry};
use crate::hardware::usb::storage::{
    append_upload_data, complete_upload, get_upload_progress, set_webrtc_read_handler,
};
use crate::remote_mount;
use crate::session::Session;
use crate::terminal::setup_terminal_channel;

#[tracing::instrument(skip_all, fields(session = %session.id))]
pub async fn handle_rpc_message(msg: DataChannelMessage, session: &Session) {
    debug!("Received RPC message: {} bytes", msg.data.len());

    let processor = JsonRpcProcessor::new(default_registry());
    processor.handle_message(msg, session).await;
}

pub async fn handle_terminal_channel(channel: Arc<RTCDataChannel>) {
    let label = channel.label().to_string();
    let channel_id = channel.id();
    info!("Terminal data channel '{}' (ID: {:?}) established", label, channel_id);

    match setup_terminal_channel(channel).await {
        Ok(_handler) => {
            info!("Terminal handler successfully initialized");
        }
        Err(e) => {
            warn!("Failed to setup terminal channel: {}", e);
        }
    }
}

pub async fn handle_serial_channel(channel: Arc<RTCDataChannel>) {
    let label = channel.label().to_string();
    info!("Serial data channel '{}' established", label);

    channel.on_message(Box::new(move |msg: DataChannelMessage| {
        Box::pin(async move {
            handle_serial_message(msg).await;
        })
    }));

    channel.on_open(Box::new(move || {
        Box::pin(async move {
            info!("Serial channel opened");
        })
    }));

    channel.on_close(Box::new(move || {
        Box::pin(async move {
            info!("Serial channel closed");
        })
    }));
}

async fn handle_serial_message(msg: DataChannelMessage) {
    debug!("Received serial data: {} bytes", msg.data.len());
}

const CDC_ACM_DEVICE_PATH: &str = "/dev/ttyGS0";

pub async fn handle_cdcacm_channel(channel: Arc<RTCDataChannel>) {
    use bytes::Bytes;
    use tokio::fs::OpenOptions;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let label = channel.label().to_string();
    info!("CDC-ACM data channel '{}' established", label);

    let device = match OpenOptions::new().read(true).write(true).open(CDC_ACM_DEVICE_PATH).await {
        Ok(f) => f,
        Err(e) => {
            warn!(path = CDC_ACM_DEVICE_PATH, "Failed to open CDC-ACM device: {e}");
            let _ = channel.close().await;
            return;
        }
    };

    let (mut reader, writer) = tokio::io::split(device);
    let writer = Arc::new(tokio::sync::Mutex::new(writer));

    let channel_send = channel.clone();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 1024];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    if let Err(e) = channel_send.send(&Bytes::copy_from_slice(&buf[..n])).await {
                        warn!("Failed to send CDC-ACM output: {e}");
                        break;
                    }
                }
                Err(e) => {
                    warn!("Failed to read from CDC-ACM device: {e}");
                    break;
                }
            }
        }
    });

    channel.on_message(Box::new(move |msg: DataChannelMessage| {
        let writer = writer.clone();
        Box::pin(async move {
            let mut w = writer.lock().await;
            if let Err(e) = w.write_all(&msg.data).await {
                warn!("Failed to write to CDC-ACM device: {e}");
            }
        })
    }));

    channel.on_open(Box::new(move || {
        Box::pin(async move {
            info!("CDC-ACM channel opened");
        })
    }));

    channel.on_close(Box::new(move || {
        Box::pin(async move {
            info!("CDC-ACM channel closed");
        })
    }));
}

pub async fn handle_upload_channel(channel: Arc<RTCDataChannel>) {
    let label = channel.label().to_string();
    info!("Upload data channel '{}' established", label);

    if !label.starts_with("upload_") {
        warn!("Invalid upload channel label: {}", label);
        return;
    }

    let upload_id = label;
    info!("Starting file upload with ID: {}", upload_id);

    let last_progress = Arc::new(Mutex::new(std::time::Instant::now()));

    let ch_for_msg = channel.clone();
    let upload_id_for_msg = upload_id.clone();
    let last_for_msg = last_progress;
    channel.on_message(Box::new(move |msg: DataChannelMessage| {
        let ch = ch_for_msg.clone();
        let upload_id = upload_id_for_msg.clone();
        let last = last_for_msg.clone();
        Box::pin(async move {
            if let Err(e) = append_upload_data(&upload_id, &msg.data).await {
                warn!("failed to write upload chunk {}: {}", upload_id, e);
                return;
            }

            let mut should_send = false;
            {
                let mut t = last.lock();
                if t.elapsed() >= std::time::Duration::from_millis(200) {
                    *t = std::time::Instant::now();
                    should_send = true;
                }
            }

            match get_upload_progress(&upload_id).await {
                Ok((size, already)) => {
                    if already >= size {
                        let progress = serde_json::json!({
                            "Size": size,
                            "AlreadyUploadedBytes": already,
                        });
                        if let Err(e) = ch.send_text(progress.to_string()).await {
                            warn!("failed to send final upload progress {}: {}", upload_id, e);
                        }
                        if let Err(e) = ch.close().await {
                            warn!("failed to close upload channel {}: {}", upload_id, e);
                        }
                        return;
                    }

                    if should_send {
                        let progress = serde_json::json!({
                            "Size": size,
                            "AlreadyUploadedBytes": already,
                        });
                        if let Err(e) = ch.send_text(progress.to_string()).await {
                            warn!("failed to send upload progress {}: {}", upload_id, e);
                        }
                    }
                }
                Err(e) => {
                    warn!("failed to get upload progress {}: {}", upload_id, e);
                }
            }
        })
    }));

    let upload_id_for_close = upload_id;
    channel.on_close(Box::new(move || {
        let upload_id = upload_id_for_close.clone();
        Box::pin(async move {
            match get_upload_progress(&upload_id).await {
                Ok((size, already)) => {
                    if already >= size {
                        info!("Upload {} completed (on close): {}/{}", upload_id, already, size);
                    } else {
                        warn!("Upload {} channel closed early: {}/{}", upload_id, already, size);
                    }
                }
                Err(e) => {
                    debug!("Upload {} close: progress not available: {}", upload_id, e);
                }
            }
            if let Err(e) = complete_upload(&upload_id).await {
                debug!("complete_upload on close ({}): {}", upload_id, e);
            }
        })
    }));
}

pub struct DataChannelManager {
    upload_prefix: String,
}

impl DataChannelManager {
    pub fn new() -> Self {
        Self { upload_prefix: "upload_".to_string() }
    }

    pub async fn route_data_channel(&self, channel: Arc<RTCDataChannel>, session: &Session) {
        let label = channel.label().to_string();

        match label.as_str() {
            "rpc" => {
                info!("Setting up RPC data channel for session: {}", session.id);
                self.setup_rpc_channel(channel, session).await;
            }
            "disk" => {
                info!("Setting up disk data channel for session: {}", session.id);
                self.setup_disk_channel(channel, session).await;
            }
            "terminal" => {
                info!("Setting up terminal data channel");
                handle_terminal_channel(channel).await;
            }
            "serial" => {
                info!("Setting up serial data channel");
                handle_serial_channel(channel).await;
            }
            "cdcacm" => {
                info!("Setting up CDC-ACM data channel");
                handle_cdcacm_channel(channel).await;
            }
            "hidrpc" => {
                info!("Setting up hidrpc data channel (reliable)");
                setup_hidrpc_channel(channel, true).await;
            }
            "hidrpc-unreliable-ordered" | "hidrpc-unreliable-nonordered" => {
                info!("Setting up {} data channel", label);
                setup_hidrpc_channel(channel, false).await;
            }
            _ if label.starts_with(&self.upload_prefix) => {
                info!("Setting up upload data channel");
                tokio::spawn(handle_upload_channel(channel));
            }
            _ => {
                warn!("Unknown data channel type: {}", label);
            }
        }
    }

    async fn setup_rpc_channel(&self, channel: Arc<RTCDataChannel>, session: &Session) {
        let session_id = session.id.clone();
        let rpc_channel = channel.clone();

        crate::webrtc::store_rpc_channel(session_id.clone(), rpc_channel.clone()).await;

        let session_id_for_msg = session_id.clone();
        channel.on_message(Box::new(move |msg: DataChannelMessage| {
            let session_id = session_id_for_msg.clone();
            let rpc_channel = rpc_channel.clone();
            Box::pin(async move {
                let mut session_with_rpc = Session::new(session_id);
                session_with_rpc.rpc_channel = Some(rpc_channel);
                handle_rpc_message(msg, &session_with_rpc).await;
            })
        }));

        channel.on_open(Box::new(move || {
            Box::pin(async move {
                info!("RPC channel opened - triggering state updates");
                crate::webrtc::trigger_ota_state_update().await;
                crate::webrtc::trigger_video_state_update().await;
                crate::webrtc::trigger_usb_state_update().await;
                crate::api::broadcast_failsafe_mode().await;

                let state = crate::hardware::usb::get_current_usb_state();
                crate::api::broadcast_usb_state(state).await;

                crate::webrtc::on_active_sessions_changed().await;
            })
        }));
    }

    async fn setup_disk_channel(&self, channel: Arc<RTCDataChannel>, session: &Session) {
        let session_id = session.id.clone();

        let ch_for_open = channel.clone();
        channel.on_open(Box::new(move || {
            let ch = ch_for_open;
            Box::pin(async move {
                let rt = tokio::runtime::Handle::current();
                remote_mount::webrtc_disk_set_sender(Arc::new(move |text: &str| {
                    let ch = ch.clone();
                    rt.block_on(async move {
                        ch.send_text(text.to_string()).await.map_err(|e| anyhow!(e.to_string()))
                    })?;
                    Ok(())
                }));
                remote_mount::install_webrtc_disk_bridge();
                info!("Disk data channel opened and bridge installed");
            })
        }));

        channel.on_message(Box::new(move |msg: DataChannelMessage| {
            let session_id = session_id.clone();
            Box::pin(async move {
                debug!("disk msg {} bytes (session={})", msg.data.len(), session_id);
                remote_mount::webrtc_disk_on_message(&msg.data);
            })
        }));

        channel.on_close(Box::new(move || {
            Box::pin(async move {
                info!("Disk data channel closed");
                remote_mount::webrtc_disk_clear_sender();
                set_webrtc_read_handler(None);
            })
        }));
    }
}

impl Default for DataChannelManager {
    fn default() -> Self {
        Self::new()
    }
}

async fn setup_hidrpc_channel(channel: Arc<RTCDataChannel>, reliable: bool) {
    if reliable {
        crate::hidrpc::install_reliable_channel(channel.clone());
    }

    let label = channel.label().to_string();
    channel.on_message(Box::new(move |msg: DataChannelMessage| {
        Box::pin(async move {
            crate::hidrpc::dispatch(msg).await;
        })
    }));

    if reliable {
        let label_close = label.clone();
        channel.on_close(Box::new(move || {
            let label = label_close.clone();
            Box::pin(async move {
                crate::hidrpc::clear_reliable_channel();
                info!("hidrpc channel '{}' closed", label);
            })
        }));
    }

    channel.on_open(Box::new(move || {
        Box::pin(async move {
            info!("hidrpc channel '{}' opened", label);
        })
    }));
}
