use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
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
    /// The macro being typed and whether it came from the web UI's data channel.
    running_macro: Option<(CancellationToken, bool)>,
}

static DISPATCHER: Lazy<Mutex<DispatcherState>> =
    Lazy::new(|| Mutex::new(DispatcherState::default()));

/// Macros (typeText, pressCombo, keyboard macros, web UI paste) type one after
/// another in arrival order; a new one never cuts off the one being typed.
static MACRO_LANE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// Bumped by an explicit cancel: drops the running macro and every queued one.
static CANCEL_ALL_EPOCH: AtomicU64 = AtomicU64::new(0);
/// Bumped when the web UI's data channel closes: drops only macros it started.
static CHANNEL_EPOCH: AtomicU64 = AtomicU64::new(0);

pub fn install_reliable_channel(ch: Arc<RTCDataChannel>) {
    DISPATCHER.lock().reliable_channel = Some(ch);
    info!("hidrpc reliable channel installed");
}

pub fn clear_reliable_channel() {
    let mut s = DISPATCHER.lock();
    s.reliable_channel = None;
    CHANNEL_EPOCH.fetch_add(1, Ordering::SeqCst);
    if let Some((token, true)) = &s.running_macro {
        token.cancel();
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
            Ok(r) => {
                // The UI learns the outcome from the macro state messages.
                drop(start_macro(r.is_paste, r.steps, true));
            }
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

fn start_macro(
    is_paste: bool,
    steps: Vec<KeyboardMacroStep>,
    from_channel: bool,
) -> tokio::sync::oneshot::Receiver<()> {
    let epochs = (CANCEL_ALL_EPOCH.load(Ordering::SeqCst), CHANNEL_EPOCH.load(Ordering::SeqCst));
    let cancelled = move || {
        CANCEL_ALL_EPOCH.load(Ordering::SeqCst) != epochs.0
            || (from_channel && CHANNEL_EPOCH.load(Ordering::SeqCst) != epochs.1)
    };
    let (done_tx, done_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        let _lane = MACRO_LANE.lock().await;
        if cancelled() {
            return;
        }
        let token = CancellationToken::new();
        DISPATCHER.lock().running_macro = Some((token.clone(), from_channel));
        // A cancel between the check above and publishing the token would miss it.
        if cancelled() {
            DISPATCHER.lock().running_macro = None;
            return;
        }

        let begin = new_keyboard_macro_state_message(true, is_paste).marshal();
        send_reliable(begin).await;
        let completed = run_macro(token, steps).await;
        let end = new_keyboard_macro_state_message(false, is_paste).marshal();
        send_reliable(end).await;
        DISPATCHER.lock().running_macro = None;
        if completed {
            let _ = done_tx.send(());
        }
    });

    done_rx
}

pub async fn play_macro(is_paste: bool, steps: Vec<KeyboardMacroStep>) -> anyhow::Result<()> {
    if usb_hid().is_none() {
        anyhow::bail!("USB HID not initialized");
    }
    start_macro(is_paste, steps, false)
        .await
        .map_err(|_| anyhow::anyhow!("keyboard macro was cancelled or failed"))
}

pub async fn cancel_running_macro() {
    cancel_macro().await;
}

async fn run_macro(cancel: CancellationToken, steps: Vec<KeyboardMacroStep>) -> bool {
    let Some(hid) = usb_hid() else { return false };

    for step in steps {
        if cancel.is_cancelled() {
            return false;
        }
        if let Err(e) = hid.keyboard_report(step.modifier, &step.keys) {
            warn!("hidrpc macro step keyboard_report: {e}");
            return false;
        }
        let delay = tokio::time::Duration::from_millis(step.delay as u64);
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = cancel.cancelled() => {
                let _ = hid.keyboard_report(0, &[0u8; HID_KEY_BUFFER_SIZE]);
                return false;
            }
        }
    }
    true
}

async fn cancel_macro() {
    CANCEL_ALL_EPOCH.fetch_add(1, Ordering::SeqCst);
    if let Some((token, _)) = &DISPATCHER.lock().running_macro {
        token.cancel();
    }
    // Returns once the running macro has released its keys and queued ones have
    // dropped out (the lane is FIFO, so this waits behind them).
    drop(MACRO_LANE.lock().await);
}
