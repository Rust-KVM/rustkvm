use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use futures::channel::oneshot;
use futures::future::{Either, select};
use gloo_timers::future::TimeoutFuture;
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{MessageEvent, RtcDataChannel};

#[derive(Debug, Clone)]
pub struct RpcError {
    pub code: Option<i32>,
    pub message: String,
}

impl RpcError {
    fn other(msg: impl Into<String>) -> Self {
        Self { code: None, message: msg.into() }
    }
    fn timeout(ms: u32) -> Self {
        Self { code: None, message: format!("timed out after {ms}ms") }
    }
    fn failsafe(reason: &str, method: &str) -> Self {
        Self { code: Some(-32000), message: format!("{method} unavailable in failsafe ({reason})") }
    }
}

impl fmt::Display for RpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.code {
            Some(c) => write!(f, "[{c}] {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

type Pending = Rc<RefCell<HashMap<u64, oneshot::Sender<Value>>>>;
type EventHandler = Rc<RefCell<Option<Box<dyn Fn(String, Option<Value>)>>>>;

pub struct RpcClient {
    channel: RtcDataChannel,
    pending: Pending,
    events: EventHandler,
    next_id: Cell<u64>,
    failsafe_reason: Rc<RefCell<Option<String>>>,
}

impl RpcClient {
    pub fn new(channel: RtcDataChannel) -> Rc<Self> {
        let pending: Pending = Rc::new(RefCell::new(HashMap::new()));
        let events: EventHandler = Rc::new(RefCell::new(None));
        let failsafe_reason: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        let on_msg = {
            let pending = pending.clone();
            let events = events.clone();
            let failsafe_reason = failsafe_reason.clone();
            Closure::<dyn FnMut(MessageEvent)>::new(move |ev: MessageEvent| {
                let Some(text) = ev.data().as_string() else { return };
                let Ok(value) = serde_json::from_str::<Value>(&text) else { return };

                if let Some(method) = value.get("method").and_then(Value::as_str) {
                    let method = method.to_string();
                    let params = value.get("params").cloned();
                    if method == "failsafeMode" {
                        *failsafe_reason.borrow_mut() = params
                            .as_ref()
                            .and_then(|p| p.get("reason"))
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                    if let Some(handler) = events.borrow().as_ref() {
                        handler(method, params);
                    }
                    return;
                }

                if let Some(id) = value.get("id").and_then(Value::as_u64)
                    && let Some(tx) = pending.borrow_mut().remove(&id)
                {
                    let _ = tx.send(value);
                }
            })
        };
        channel.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
        on_msg.forget();

        Rc::new(Self { channel, pending, events, next_id: Cell::new(0), failsafe_reason })
    }

    pub fn on_event(&self, handler: impl Fn(String, Option<Value>) + 'static) {
        *self.events.borrow_mut() = Some(Box::new(handler));
    }

    fn next_id(&self) -> u64 {
        let id = self.next_id.get().wrapping_add(1);
        self.next_id.set(id);
        id
    }

    pub fn notify(&self, method: &str, params: Value) {
        let payload =
            json!({ "jsonrpc": "2.0", "method": method, "params": params, "id": self.next_id() });
        let _ = self.channel.send_with_str(&payload.to_string());
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.call_with(method, params, 5_000, 1).await
    }

    pub async fn call_with(
        &self,
        method: &str,
        params: Value,
        timeout_ms: u32,
        max_attempts: u32,
    ) -> Result<Value, RpcError> {
        if let Some(reason) = self.failsafe_reason.borrow().as_deref()
            && is_blocked(reason, method)
        {
            return Err(RpcError::failsafe(reason, method));
        }

        for _ in 0..100 {
            if self.channel.ready_state() == web_sys::RtcDataChannelState::Open {
                break;
            }
            TimeoutFuture::new(50).await;
        }

        let attempts = max_attempts.max(1);
        let mut last = RpcError::other("no attempt made");
        for attempt in 0..attempts {
            let id = self.next_id();
            let (tx, rx) = oneshot::channel::<Value>();
            self.pending.borrow_mut().insert(id, tx);

            let payload = json!({
                "jsonrpc": "2.0", "method": method, "params": params, "id": id,
            });
            if self.channel.send_with_str(&payload.to_string()).is_err() {
                self.pending.borrow_mut().remove(&id);
                last = RpcError::other("data channel send failed");
            } else {
                match select(rx, TimeoutFuture::new(timeout_ms)).await {
                    Either::Left((Ok(value), _)) => return parse_response(value),
                    Either::Left((Err(_canceled), _)) => last = RpcError::other("call canceled"),
                    Either::Right(((), _)) => {
                        self.pending.borrow_mut().remove(&id);
                        last = RpcError::timeout(timeout_ms);
                    }
                }
            }

            if attempt + 1 < attempts {
                let backoff = (500u32 << attempt).min(10_000);
                TimeoutFuture::new(backoff).await;
            }
        }
        Err(last)
    }
}

fn parse_response(value: Value) -> Result<Value, RpcError> {
    if let Some(err) = value.get("error") {
        return Err(RpcError {
            code: err.get("code").and_then(Value::as_i64).map(|c| c as i32),
            message: err.get("message").and_then(Value::as_str).unwrap_or("RPC error").to_string(),
        });
    }
    Ok(value.get("result").cloned().unwrap_or(Value::Null))
}

fn is_blocked(reason: &str, method: &str) -> bool {
    match reason {
        "video" => matches!(
            method,
            "setStreamQualityFactor"
                | "setVideoCodecPreference"
                | "getEDID"
                | "setEDID"
                | "getHostDisplayIdleMode"
                | "setHostDisplayIdleMode"
                | "getVideoLogStatus"
                | "setDisplayRotation"
                | "getVideoSleepMode"
                | "setVideoSleepMode"
                | "getVideoState"
        ),
        _ => false,
    }
}
