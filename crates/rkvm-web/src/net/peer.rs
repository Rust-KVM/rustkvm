use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
use gloo_net::websocket::Message as WsMessage;
use gloo_net::websocket::futures::WebSocket;
use gloo_timers::future::TimeoutFuture;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    HtmlVideoElement, MediaStream, RtcDataChannel, RtcDataChannelInit, RtcDataChannelState,
    RtcIceCandidateInit, RtcIceConnectionState, RtcPeerConnection, RtcPeerConnectionIceEvent,
    RtcRtpTransceiverDirection, RtcRtpTransceiverInit, RtcSdpType, RtcSessionDescriptionInit,
    RtcTrackEvent,
};

pub struct Peer {
    pub pc: RtcPeerConnection,
    pub rpc: RtcDataChannel,
    pub hidrpc: RtcDataChannel,
    pub hidrpc_unreliable_nonordered: RtcDataChannel,
    pub terminal: RtcDataChannel,
}

pub async fn connect(video: &HtmlVideoElement) -> Result<Peer, String> {
    let pc = RtcPeerConnection::new().map_err(js_err("RtcPeerConnection::new"))?;

    let tinit = RtcRtpTransceiverInit::new();
    tinit.set_direction(RtcRtpTransceiverDirection::Recvonly);
    let _ = pc.add_transceiver_with_str_and_init("video", &tinit);
    let _ = pc.add_transceiver_with_str_and_init("audio", &tinit);

    {
        let video = video.clone();
        let on_track = Closure::<dyn FnMut(RtcTrackEvent)>::new(move |ev: RtcTrackEvent| {
            if let Some(stream) = ev.streams().get(0).dyn_ref::<MediaStream>() {
                video.set_src_object(Some(stream));
            }
        });
        pc.set_ontrack(Some(on_track.as_ref().unchecked_ref()));
        on_track.forget();
    }

    let rpc = pc.create_data_channel("rpc");
    let hidrpc = pc.create_data_channel("hidrpc");
    let no_init = RtcDataChannelInit::new();
    no_init.set_ordered(false);
    no_init.set_max_retransmits(0);
    let hidrpc_unreliable_nonordered =
        pc.create_data_channel_with_data_channel_dict("hidrpc-unreliable-nonordered", &no_init);
    let terminal = pc.create_data_channel("terminal");

    let (out_tx, mut out_rx) = mpsc::unbounded::<String>();

    {
        let tx = out_tx.clone();
        let on_cand = Closure::<dyn FnMut(RtcPeerConnectionIceEvent)>::new(
            move |ev: RtcPeerConnectionIceEvent| {
                if let Some(c) = ev.candidate() {
                    let msg = json!({
                        "type": "new-ice-candidate",
                        "data": {
                            "candidate": c.candidate(),
                            "sdpMid": c.sdp_mid(),
                            "sdpMLineIndex": c.sdp_m_line_index(),
                        }
                    });
                    let _ = tx.unbounded_send(msg.to_string());
                }
            },
        );
        pc.set_onicecandidate(Some(on_cand.as_ref().unchecked_ref()));
        on_cand.forget();
    }

    let offer = JsFuture::from(pc.create_offer())
        .await
        .map_err(js_err("createOffer"))?
        .unchecked_into::<RtcSessionDescriptionInit>();
    JsFuture::from(pc.set_local_description(&offer))
        .await
        .map_err(js_err("setLocalDescription"))?;
    let local = pc.local_description().ok_or("no local description")?;
    let sd = STANDARD.encode(
        serde_json::to_vec(&json!({ "type": "offer", "sdp": local.sdp() }))
            .map_err(|e| format!("encode offer: {e}"))?,
    );

    let ws = WebSocket::open(&signaling_url()?).map_err(|e| format!("open signaling ws: {e}"))?;
    let (mut sink, mut stream) = ws.split();

    sink.send(WsMessage::Text(json!({ "type": "offer", "data": { "sd": sd } }).to_string()))
        .await
        .map_err(|e| format!("send offer: {e}"))?;

    spawn_local(async move {
        while let Some(text) = out_rx.next().await {
            if sink.send(WsMessage::Text(text)).await.is_err() {
                break;
            }
        }
    });

    {
        let tx = out_tx.clone();
        spawn_local(async move {
            loop {
                TimeoutFuture::new(750).await;
                if tx.unbounded_send("ping".to_string()).is_err() {
                    break;
                }
            }
        });
    }

    {
        let pc = pc.clone();
        spawn_local(async move {
            while let Some(Ok(msg)) = stream.next().await {
                let WsMessage::Text(text) = msg else { continue };
                if text == "pong" {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
                match value.get("type").and_then(Value::as_str) {
                    Some("answer") => {
                        if let Some(sdp) = decode_sdp(value.get("data")) {
                            let answer = RtcSessionDescriptionInit::new(RtcSdpType::Answer);
                            answer.set_sdp(&sdp);
                            let _ = JsFuture::from(pc.set_remote_description(&answer)).await;
                        }
                    }
                    Some("new-ice-candidate") => {
                        if let Some(init) = candidate_init(value.get("data")) {
                            let _ = JsFuture::from(
                                pc.add_ice_candidate_with_opt_rtc_ice_candidate_init(Some(&init)),
                            )
                            .await;
                        }
                    }
                    _ => {}
                }
            }
        });
    }

    for _ in 0..240 {
        match pc.ice_connection_state() {
            RtcIceConnectionState::Failed | RtcIceConnectionState::Closed => {
                return Err("ICE connection failed".to_string());
            }
            _ => {}
        }
        if rpc.ready_state() == RtcDataChannelState::Open {
            return Ok(Peer { pc, rpc, hidrpc, hidrpc_unreliable_nonordered, terminal });
        }
        TimeoutFuture::new(100).await;
    }
    Err("timed out waiting for the data channel to open".to_string())
}

fn signaling_url() -> Result<String, String> {
    let location = web_sys::window().ok_or("no window")?.location();
    let proto = location.protocol().map_err(|_| "no protocol")?;
    let host = location.host().map_err(|_| "no host")?;
    let scheme = if proto == "https:" { "wss" } else { "ws" };
    Ok(format!("{scheme}://{host}/webrtc/signaling/client"))
}

fn decode_sdp(data: Option<&Value>) -> Option<String> {
    let b64 = data?.as_str()?;
    let bytes = STANDARD.decode(b64).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    value.get("sdp").and_then(Value::as_str).map(str::to_string)
}

fn candidate_init(data: Option<&Value>) -> Option<RtcIceCandidateInit> {
    let data = data?;
    let candidate = data.get("candidate").and_then(Value::as_str).unwrap_or_default();
    let init = RtcIceCandidateInit::new(candidate);
    init.set_sdp_mid(data.get("sdpMid").and_then(Value::as_str));
    if let Some(idx) = data.get("sdpMLineIndex").and_then(Value::as_u64) {
        init.set_sdp_m_line_index(Some(idx as u16));
    }
    Some(init)
}

fn js_err(ctx: &'static str) -> impl Fn(wasm_bindgen::JsValue) -> String {
    move |e| format!("{ctx}: {e:?}")
}
