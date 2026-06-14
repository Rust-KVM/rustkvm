use std::fmt;
use std::sync::Arc;

use tracing::info;
use webrtc::data_channel::RTCDataChannel;
use webrtc::peer_connection::RTCPeerConnection;
use webrtc::track::track_local::track_local_static_sample::TrackLocalStaticSample;

pub struct Session {
    pub id: Arc<str>,
    pub peer_connection: Option<Arc<RTCPeerConnection>>,
    pub video_track: Option<Arc<TrackLocalStaticSample>>,
    pub audio_track: Option<Arc<TrackLocalStaticSample>>,
    pub control_channel: Option<Arc<RTCDataChannel>>,
    pub rpc_channel: Option<Arc<RTCDataChannel>>,
    pub hid_channel: Option<Arc<RTCDataChannel>>,
    pub disk_channel: Option<Arc<RTCDataChannel>>,
    pub should_unmount_virtual_media: bool,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("has_peer_connection", &self.peer_connection.is_some())
            .field("has_video_track", &self.video_track.is_some())
            .field("has_audio_track", &self.audio_track.is_some())
            .field("has_control_channel", &self.control_channel.is_some())
            .field("has_rpc_channel", &self.rpc_channel.is_some())
            .field("has_hid_channel", &self.hid_channel.is_some())
            .field("has_disk_channel", &self.disk_channel.is_some())
            .field("should_unmount_virtual_media", &self.should_unmount_virtual_media)
            .finish()
    }
}

impl Clone for Session {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            peer_connection: self.peer_connection.clone(),
            video_track: self.video_track.clone(),
            audio_track: self.audio_track.clone(),
            control_channel: self.control_channel.clone(),
            rpc_channel: self.rpc_channel.clone(),
            hid_channel: self.hid_channel.clone(),
            disk_channel: self.disk_channel.clone(),
            should_unmount_virtual_media: self.should_unmount_virtual_media,
        }
    }
}

impl fmt::Display for Session {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "id={}", self.id,)
    }
}

impl Session {
    pub fn new(id: impl Into<Arc<str>>) -> Self {
        Self {
            id: id.into(),
            peer_connection: None,
            video_track: None,
            audio_track: None,
            control_channel: None,
            rpc_channel: None,
            hid_channel: None,
            disk_channel: None,
            should_unmount_virtual_media: false,
        }
    }

    #[tracing::instrument(skip_all, fields(session = %self.id))]
    pub async fn exchange_offer(&self, offer_str: &str) -> anyhow::Result<String> {
        use base64::Engine as _;
        use base64::engine::general_purpose;

        let offer_bytes = general_purpose::STANDARD.decode(offer_str)?;
        let offer: webrtc::peer_connection::sdp::session_description::RTCSessionDescription =
            serde_json::from_slice(&offer_bytes)?;

        if let Some(peer_conn) = &self.peer_connection {
            peer_conn.set_remote_description(offer).await?;

            let answer = peer_conn.create_answer(None).await?;

            peer_conn.set_local_description(answer).await?;

            if let Some(local_desc) = peer_conn.local_description().await {
                let local_desc_bytes = serde_json::to_vec(&local_desc)?;
                let answer_str = general_purpose::STANDARD.encode(local_desc_bytes);
                Ok(answer_str)
            } else {
                anyhow::bail!("Failed to get local description")
            }
        } else {
            anyhow::bail!("No peer connection available")
        }
    }

    pub async fn add_ice_candidate(&self, candidate_str: &str) -> anyhow::Result<()> {
        let candidate: webrtc::ice_transport::ice_candidate::RTCIceCandidateInit =
            serde_json::from_str(candidate_str)?;

        if let Some(peer_conn) = &self.peer_connection {
            peer_conn.add_ice_candidate(candidate).await?;
            info!("Added ICE candidate to session: {}", self.id);
            Ok(())
        } else {
            anyhow::bail!("No peer connection available")
        }
    }
}
