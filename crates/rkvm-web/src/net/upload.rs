use std::rc::Rc;

use gloo_timers::future::TimeoutFuture;
use js_sys::{ArrayBuffer, Uint8Array};
use serde_json::{Value, json};
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::{File, RtcDataChannel, RtcDataChannelState, RtcDataChannelType};

use crate::net::peer::Peer;
use crate::rpc::RpcClient;

const CHUNK: u32 = 16 * 1024;
const BACKPRESSURE_HIGH: u32 = 1_000_000;

pub async fn upload_and_mount(
    peer: Rc<Peer>,
    rpc: Rc<RpcClient>,
    file: File,
    mode: String,
    progress: impl Fn(u32),
) -> Result<(), String> {
    let filename = file.name();
    let size = file.size() as i64;

    let start = rpc
        .call("startStorageFileUpload", json!({ "filename": filename, "size": size }))
        .await
        .map_err(|e| e.to_string())?;
    let label = start
        .get("dataChannel")
        .and_then(Value::as_str)
        .ok_or("startStorageFileUpload missing dataChannel")?
        .to_string();

    let channel = peer.pc.create_data_channel(&label);
    channel.set_binary_type(RtcDataChannelType::Arraybuffer);
    wait_open(&channel).await?;

    let buffer = JsFuture::from(file.array_buffer())
        .await
        .map_err(|e| format!("read file: {e:?}"))?
        .dyn_into::<ArrayBuffer>()
        .map_err(|_| "file did not yield an ArrayBuffer".to_string())?;
    let bytes = Uint8Array::new(&buffer);
    let total = bytes.length();

    let mut offset = 0;
    let mut scratch = vec![0u8; CHUNK as usize];
    while offset < total {
        while channel.buffered_amount() > BACKPRESSURE_HIGH {
            TimeoutFuture::new(10).await;
        }
        let end = (offset + CHUNK).min(total);
        let len = (end - offset) as usize;
        let view = bytes.subarray(offset, end);
        view.copy_to(&mut scratch[..len]);
        channel.send_with_u8_array(&scratch[..len]).map_err(|e| format!("send chunk: {e:?}"))?;
        offset = end;
        progress(offset.saturating_mul(100).checked_div(total).unwrap_or(100));
    }

    while channel.buffered_amount() > 0 {
        TimeoutFuture::new(20).await;
    }

    rpc.call("mountWithStorage", json!({ "filename": filename, "mode": mode }))
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn wait_open(channel: &RtcDataChannel) -> Result<(), String> {
    for _ in 0..200 {
        match channel.ready_state() {
            RtcDataChannelState::Open => return Ok(()),
            RtcDataChannelState::Closing | RtcDataChannelState::Closed => {
                return Err("upload channel closed before opening".to_string());
            }
            _ => TimeoutFuture::new(25).await,
        }
    }
    Err("upload channel did not open".to_string())
}
