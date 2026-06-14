use std::collections::HashMap;
use std::ffi::CString;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use rustix::fd::OwnedFd;
use rustix::io::dup;
use rustix::net::{
    AddressFamily, RecvFlags, SendFlags, SocketAddrUnix, SocketFlags, SocketType, accept_with,
    bind, listen, recv, send, socket,
};
use serde_json::{Map, Value};
use tokio::runtime::Handle as TokioHandle;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use super::jsonrpc::{CtrlAction, CtrlResponse};
use crate::video::{VideoInputState, handle_video_state_message};

#[derive(Default)]
struct CtrlShared {
    conn: Option<OwnedFd>,
    next_seq: i32,
    pending: HashMap<i32, oneshot::Sender<CtrlResponse>>,
}

pub struct CtrlSocket {
    inner: Arc<Mutex<CtrlShared>>,
}

impl Default for CtrlSocket {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(CtrlShared {
                conn: None,
                next_seq: 1,
                pending: HashMap::new(),
            })),
        }
    }
}

impl CtrlSocket {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start_server(&self, socket_path: &str, is_ctrl: bool) -> Result<()> {
        let path = Path::new(socket_path);
        if path.exists() {
            std::fs::remove_file(path).context("failed to remove existing socket file")?;
        }
        let sock = socket(AddressFamily::UNIX, SocketType::SEQPACKET, None)
            .context("failed to create seqpacket socket")?;
        let c_path = CString::new(socket_path).context("invalid socket path")?;
        let addr = SocketAddrUnix::new(&c_path).context("failed to build unix addr")?;
        bind(&sock, &addr).context("failed to bind unix seqpacket socket")?;
        listen(&sock, 128).context("failed to listen on unix seqpacket socket")?;

        let shared = self.inner.clone();
        let rt_handle = TokioHandle::try_current().ok();
        std::thread::Builder::new()
            .name("ctrl-sock-acceptor".to_string())
            .spawn(move || {
                loop {
                    match accept_with(&sock, SocketFlags::CLOEXEC) {
                        Ok(conn) => {
                            {
                                let mut guard = shared.lock();
                                if guard.conn.is_some() {
                                    debug!("closing existing ctrl conn");
                                }
                                match dup(&conn) {
                                    Ok(fd) => guard.conn = Some(fd),
                                    Err(e) => {
                                        warn!(
                                            "failed to dup ctrl connection fd, dropping client: {}",
                                            e
                                        );
                                        continue;
                                    }
                                }
                            }
                            if is_ctrl && let Some(rt) = &rt_handle {
                                static CTRL_FIRST_CONNECTED: once_cell::sync::OnceCell<()> =
                                    once_cell::sync::OnceCell::new();
                                if CTRL_FIRST_CONNECTED.set(()).is_ok() {
                                    rt.spawn(async move {
                                        let cfg = crate::config::get_config_manager().get().await;
                                        if let Some(edid) = cfg.edid_string.clone()
                                            && !edid.is_empty()
                                        {
                                            let mut params = serde_json::Map::new();
                                            params.insert(
                                                "edid".to_string(),
                                                serde_json::Value::String(edid),
                                            );
                                            if let Err(e) = super::socket::call_ctrl_action(
                                                "set_edid",
                                                Some(params),
                                            )
                                            .await
                                            {
                                                warn!("failed to restore HDMI EDID: {}", e);
                                            } else {
                                                info!("HDMI EDID restored via ctrl action");
                                            }
                                        }
                                    });
                                }
                            }
                            let reader_fd = conn;
                            let shared_reader = shared.clone();
                            let rt_handle_clone = rt_handle.clone();
                            if let Err(e) = std::thread::Builder::new()
                                .name("ctrl-sock-reader".to_string())
                                .spawn(move || {
                                    if is_ctrl {
                                        debug!("ctrl client connected");
                                    }
                                    if let Err(e) =
                                        read_loop(reader_fd, shared_reader, rt_handle_clone)
                                    {
                                        warn!("ctrl read loop error: {}", e);
                                    }
                                })
                            {
                                warn!("failed to spawn ctrl reader thread: {}", e);
                            }
                        }
                        Err(e) => {
                            warn!("accept error: {}", e);
                            std::thread::sleep(Duration::from_millis(200));
                        }
                    }
                }
            })
            .context("failed to spawn ctrl socket acceptor thread")?;

        info!("server listening: {}", socket_path);
        Ok(())
    }

    pub async fn call_action(
        &self,
        action: &str,
        params: Option<serde_json::Map<String, Value>>,
    ) -> Result<CtrlResponse> {
        let (seq, rx) = {
            let mut guard = self.inner.lock();
            let seq = guard.next_seq;
            guard.next_seq += 1;
            let (tx, rx) = oneshot::channel();
            guard.pending.insert(seq, tx);
            (seq, rx)
        };

        let ctrl = CtrlAction { action: action.to_string(), seq: Some(seq), params };
        let payload = serde_json::to_vec(&ctrl)?;
        self.write_message(&payload).await?;

        match timeout(Duration::from_secs(5), rx).await {
            Ok(Ok(resp)) => {
                if !resp.error.is_empty() {
                    return Err(anyhow!(resp.error));
                }
                Ok(resp)
            }
            Ok(Err(_)) => Err(anyhow!("response channel closed")),
            Err(_) => {
                let mut guard = self.inner.lock();
                guard.pending.remove(&seq);
                Err(anyhow!("timeout waiting for response"))
            }
        }
    }

    pub async fn write_message(&self, data: &[u8]) -> Result<()> {
        let dup_fd = {
            let guard = self.inner.lock();
            let fd_ref = guard.conn.as_ref().ok_or_else(|| anyhow!("ctrl socket not connected"))?;
            dup(fd_ref).context("failed to dup ctrl fd for send")?
        };
        let buf = data.to_vec();
        tokio::task::spawn_blocking(move || {
            send(&dup_fd, &buf, SendFlags::empty()).context("send ctrl message failed")?;
            Ok::<(), anyhow::Error>(())
        })
        .await
        .map_err(|e| anyhow!("join error: {}", e))??;
        Ok(())
    }
}

fn read_loop(conn: OwnedFd, shared: Arc<Mutex<CtrlShared>>, rt: Option<TokioHandle>) -> Result<()> {
    loop {
        let mut buf = vec![0u8; 64 * 1024];
        let (_, n) = recv(&conn, &mut buf[..], RecvFlags::empty())?;
        if n == 0 {
            return Ok(());
        }
        buf.truncate(n);

        let resp: CtrlResponse = match serde_json::from_slice(&buf) {
            Ok(v) => v,
            Err(e) => {
                warn!("invalid ctrl json: {}", e);
                continue;
            }
        };

        debug!("ctrl sock msg: {} bytes", n);

        if resp.seq != 0 {
            let tx = {
                let mut guard = shared.lock();
                guard.pending.remove(&resp.seq)
            };
            if let Some(tx) = tx {
                let _ = tx.send(resp);
                continue;
            }
        }

        if !resp.event.is_empty()
            && resp.event.as_str() == "video_input_state"
            && let Some(data) = resp.data
        {
            match serde_json::from_value::<VideoInputState>(data) {
                Ok(state) => {
                    if let Some(rt) = &rt {
                        rt.spawn(async move {
                            handle_video_state_message(state).await;
                        });
                    } else {
                        warn!("no tokio runtime handle available to deliver video state");
                    }
                }
                Err(e) => warn!("failed to parse VideoInputState: {}", e),
            }
        }
    }
}

static CTRL: OnceCell<CtrlSocket> = OnceCell::new();

pub fn init_ctrl_socket() -> Result<()> {
    let sock = CtrlSocket::new();
    let _ = CTRL.set(sock);
    let ctrl = CTRL.get().expect("ctrl OnceCell");
    ctrl.start_server("/var/run/rustkvm_ctrl.sock", true)?;
    Ok(())
}

pub async fn call_ctrl_action(
    action: &str,
    params: Option<Map<String, Value>>,
) -> Result<CtrlResponse> {
    let ctrl = CTRL.get().ok_or_else(|| anyhow!("ctrl socket not initialized"))?;
    ctrl.call_action(action, params).await
}
