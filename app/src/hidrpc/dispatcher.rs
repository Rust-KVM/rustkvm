use std::sync::Arc;

use bytes::Bytes;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_message::DataChannelMessage;

use super::codec::{
    HID_KEY_BUFFER_SIZE, KeyboardMacroStep, Message, MessageType, new_handshake_message,
    new_keyboard_macro_state_message,
};

#[derive(Default)]
struct DispatcherState {
    reliable_channel: Option<Arc<RTCDataChannel>>,
    macro_task: Option<(JoinHandle<()>, CancellationToken)>,
}

static DISPATCHER: Lazy<Mutex<DispatcherState>> =
    Lazy::new(|| Mutex::new(DispatcherState::default()));

pub fn install_reliable_channel(ch: Arc<RTCDataChannel>) {
    DISPATCHER.lock().reliable_channel = Some(ch);
    info!("hidrpc reliable channel installed");
}

pub fn clear_reliable_channel() {
    let prior_task = {
        let mut s = DISPATCHER.lock();
        s.reliable_channel = None;
        s.macro_task.take()
    };
    if let Some((handle, token)) = prior_task {
        token.cancel();
        handle.abort();
    }
}

fn reliable_channel() -> Option<Arc<RTCDataChannel>> {
    DISPATCHER.lock().reliable_channel.clone()
}

async fn send_reliable(payload: Vec<u8>) {
    if let Some(ch) = reliable_channel() {
        if let Err(e) = ch.send(&Bytes::from(payload)).await {
            warn!("hidrpc reliable send failed: {e}");
        }
    } else {
        debug!("hidrpc reliable channel unbound; dropping outbound frame");
    }
}

pub async fn dispatch(msg: DataChannelMessage) {
    if msg.is_string {
        debug!("hidrpc received string frame on binary channel; ignoring");
        return;
    }

    let message = match Message::unmarshal(&msg.data) {
        Ok(m) => m,
        Err(e) => {
            warn!("hidrpc unmarshal: {e}");
            return;
        }
    };

    match message.r#type {
        MessageType::Handshake => {
            let resp = new_handshake_message().marshal();
            send_reliable(resp).await;
        }
        MessageType::KeyboardReport => match message.keyboard_report() {
            Ok(r) => {
                if let Some(hid) = usb_hid() {
                    if let Err(e) = hid.keyboard_report(r.modifier, r.keys) {
                        warn!("hidrpc keyboard_report: {e}");
                    }
                }
            }
            Err(e) => warn!("hidrpc keyboard_report parse: {e}"),
        },
        MessageType::KeypressReport => match message.keypress_report() {
            Ok(r) => {
                if let Some(hid) = usb_hid() {
                    if let Err(e) = hid.keypress_report(r.key, r.press) {
                        warn!("hidrpc keypress_report: {e}");
                    }
                }
            }
            Err(e) => warn!("hidrpc keypress_report parse: {e}"),
        },
        MessageType::PointerReport => match message.pointer_report() {
            Ok(r) => {
                if let Some(hid) = usb_hid() {
                    if let Err(e) = hid.abs_mouse_report(r.x, r.y, r.button) {
                        warn!("hidrpc pointer_report: {e}");
                    }
                }
            }
            Err(e) => warn!("hidrpc pointer_report parse: {e}"),
        },
        MessageType::MouseReport => match message.mouse_report() {
            Ok(r) => {
                if let Some(hid) = usb_hid() {
                    if let Err(e) = hid.rel_mouse_report(r.dx, r.dy, r.button) {
                        warn!("hidrpc mouse_report: {e}");
                    }
                }
            }
            Err(e) => warn!("hidrpc mouse_report parse: {e}"),
        },
        MessageType::KeyboardMacroReport => match message.keyboard_macro_report() {
            Ok(r) => start_macro(r.is_paste, r.steps).await,
            Err(e) => warn!("hidrpc keyboard_macro_report parse: {e}"),
        },
        MessageType::CancelKeyboardMacroReport => cancel_macro().await,
        MessageType::KeypressKeepAliveReport => {}
        MessageType::WheelReport
        | MessageType::KeyboardLedState
        | MessageType::KeydownState
        | MessageType::KeyboardMacroState => {
            debug!("hidrpc dispatcher: unexpected inbound type {:?}", message.r#type);
        }
    }
}

fn usb_hid() -> Option<Arc<crate::hardware::usb::hid::Hid>> {
    crate::hardware::usb::get_usb_manager().map(|m| m.read().hid())
}

async fn start_macro(is_paste: bool, steps: Vec<KeyboardMacroStep>) {
    cancel_macro().await;

    let begin = new_keyboard_macro_state_message(true, is_paste).marshal();
    send_reliable(begin).await;

    let token = CancellationToken::new();
    let run_token = token.clone();

    let handle = tokio::spawn(async move {
        run_macro(run_token, steps).await;
        let end = new_keyboard_macro_state_message(false, is_paste).marshal();
        send_reliable(end).await;
    });

    DISPATCHER.lock().macro_task = Some((handle, token));
}

async fn run_macro(cancel: CancellationToken, steps: Vec<KeyboardMacroStep>) {
    let Some(hid) = usb_hid() else { return };

    for step in steps {
        if cancel.is_cancelled() {
            break;
        }
        if let Err(e) = hid.keyboard_report(step.modifier, &step.keys) {
            warn!("hidrpc macro step keyboard_report: {e}");
            return;
        }
        let delay = tokio::time::Duration::from_millis(step.delay as u64);
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = cancel.cancelled() => {
                let _ = hid.keyboard_report(0, &[0u8; HID_KEY_BUFFER_SIZE]);
                return;
            }
        }
    }
}

async fn cancel_macro() {
    let task = DISPATCHER.lock().macro_task.take();
    if let Some((handle, token)) = task {
        token.cancel();
        let _ = handle.await;
    }
}
