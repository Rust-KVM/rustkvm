use std::io;
use std::os::fd::OwnedFd;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use bytes::Bytes;
use parking_lot::Mutex;
use rustix::fs::{Mode, OFlags, fcntl_getfl, fcntl_setfl, open};
use rustix::io::{read, write};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};
use serde::{Deserialize, Serialize};
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};
use webrtc::data_channel::RTCDataChannel;
use webrtc::data_channel::data_channel_message::DataChannelMessage;

/// A shell that does not read its input for this long has the write dropped,
/// so one stuck program cannot hold the data channel's message loop forever.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSize {
    pub rows: u16,
    pub cols: u16,
}

/// A window-size message from the web terminal: a JSON object and nothing else.
pub fn parse_resize(msg: &[u8]) -> Option<TerminalSize> {
    let text = std::str::from_utf8(msg).ok()?.trim();
    (text.starts_with('{') && text.ends_with('}')).then(|| serde_json::from_str(text).ok())?
}

/// A shell on a pseudo-terminal, alive while its data channel is open.
struct Shell {
    master: Arc<AsyncFd<OwnedFd>>,
    child: Child,
    output: JoinHandle<()>,
}

impl Shell {
    fn spawn(channel: Arc<RTCDataChannel>) -> Result<Self> {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).context("open PTY master")?;
        grantpt(&master).context("grant PTY access")?;
        unlockpt(&master).context("unlock PTY")?;
        let slave_name = ptsname(&master, Vec::new()).context("get PTY slave name")?;
        let slave = std::fs::File::from(
            open(slave_name.as_c_str(), OFlags::RDWR, Mode::empty()).context("open PTY slave")?,
        );

        let child = Command::new("/bin/sh")
            .stdin(Stdio::from(slave.try_clone().context("clone PTY slave")?))
            .stdout(Stdio::from(slave.try_clone().context("clone PTY slave")?))
            .stderr(Stdio::from(slave))
            .spawn()
            .context("start shell")?;

        let flags = fcntl_getfl(&master).context("read PTY flags")?;
        fcntl_setfl(&master, flags | OFlags::NONBLOCK).context("set PTY non-blocking")?;
        // SAFETY: `master` is an `OwnedFd`, so it is a valid open descriptor that
        // nothing else closes or replaces while the `AsyncFd` owns it.
        let master = unsafe { AsyncFd::register(master) }
            .map_err(io::Error::from)
            .context("register PTY with the runtime")?;
        let master = Arc::new(master);

        let output = tokio::spawn(forward_output(master.clone(), channel));
        Ok(Self { master, child, output })
    }

    /// Stops the shell and reaps it, so closed terminals leave no zombie.
    fn stop(mut self) {
        self.output.abort();
        if let Err(e) = self.child.kill() {
            debug!("shell already gone: {e}");
        }
        tokio::task::spawn_blocking(move || {
            if let Err(e) = self.child.wait() {
                warn!("failed to reap shell: {e}");
            }
        });
    }
}

/// Sends shell output to the browser from one task, so chunks arrive in the
/// order they were read.
async fn forward_output(master: Arc<AsyncFd<OwnedFd>>, channel: Arc<RTCDataChannel>) {
    let mut buf = [0u8; 4096];
    loop {
        let read = master
            .async_io(Interest::READABLE, |fd| read(fd, &mut buf[..]).map_err(io::Error::from))
            .await;
        match read {
            // The master reads EIO once the shell has exited.
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if let Err(e) = channel.send(&Bytes::copy_from_slice(&buf[..n])).await {
                    debug!("terminal channel gone: {e}");
                    break;
                }
            }
        }
    }
    debug!("terminal output ended");
}

async fn write_all(master: &AsyncFd<OwnedFd>, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        let n = master
            .async_io(Interest::WRITABLE, |fd| write(fd, data).map_err(io::Error::from))
            .await?;
        data = &data[n..];
    }
    Ok(())
}

fn resize(master: &AsyncFd<OwnedFd>, size: &TerminalSize) -> Result<()> {
    let winsize = Winsize { ws_row: size.rows, ws_col: size.cols, ws_xpixel: 0, ws_ypixel: 0 };
    tcsetwinsize(master.get_ref(), winsize).context("set terminal window size")
}

/// Runs a shell for a `terminal` data channel: started when the channel opens,
/// stopped when it closes.
pub fn attach(channel: &Arc<RTCDataChannel>) {
    let shell: Arc<Mutex<Option<Shell>>> = Arc::default();

    let slot = shell.clone();
    // Weak: the channel owns this handler, so a strong ref would keep it alive.
    let weak = Arc::downgrade(channel);
    channel.on_open(Box::new(move || {
        Box::pin(async move {
            let Some(channel) = weak.upgrade() else { return };
            match Shell::spawn(channel.clone()) {
                Ok(new) => {
                    info!(pid = new.child.id(), "terminal shell started");
                    if let Some(old) = slot.lock().replace(new) {
                        old.stop();
                    }
                }
                Err(e) => {
                    warn!(error = %format!("{e:#}"), "cannot start terminal shell");
                    let _ = channel.close().await;
                }
            }
        })
    }));

    let slot = shell.clone();
    channel.on_message(Box::new(move |msg: DataChannelMessage| {
        let master = slot.lock().as_ref().map(|s| s.master.clone());
        Box::pin(async move {
            let Some(master) = master else { return };
            if msg.is_string
                && let Some(size) = parse_resize(&msg.data)
            {
                match resize(&master, &size) {
                    Ok(()) => debug!(cols = size.cols, rows = size.rows, "terminal resized"),
                    Err(e) => warn!(error = %format!("{e:#}"), "terminal resize failed"),
                }
                return;
            }
            match tokio::time::timeout(WRITE_TIMEOUT, write_all(&master, &msg.data)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => debug!("terminal input dropped: {e}"),
                Err(_) => warn!("terminal input dropped: shell is not reading"),
            }
        })
    }));

    channel.on_close(Box::new(move || {
        if let Some(shell) = shell.lock().take() {
            shell.stop();
            info!("terminal shell stopped");
        }
        Box::pin(async {})
    }));
}
