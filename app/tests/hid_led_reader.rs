use std::ffi::CString;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::time::Duration;

use rustkvm::hardware::usb::hid::{Hid, KEYBOARD_LED_MASK_CAPS_LOCK};

// The LED callback in the app spawns a tokio task; the reader thread must run it
// inside the runtime or the process aborts on the first LED report.
#[tokio::test(flavor = "multi_thread")]
async fn led_reader_runs_state_callback_inside_the_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let fifo = dir.path().join("hidg0");
    let c_path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    // SAFETY: c_path is a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);

    let hid = Hid::new(
        fifo.to_string_lossy().into_owned(),
        dir.path().join("hidg1").to_string_lossy().into_owned(),
        dir.path().join("hidg2").to_string_lossy().into_owned(),
    );
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    hid.set_on_keyboard_state_change(move |state| {
        let tx = tx.clone();
        tokio::spawn(async move {
            let _ = tx.send(state);
        });
    });
    hid.open_keyboard_hid_file().unwrap();

    let mut host = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
    host.write_all(&[KEYBOARD_LED_MASK_CAPS_LOCK]).unwrap();

    let state = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.unwrap().unwrap();
    assert!(state.caps_lock);

    // Stopping must not hang on a reader that has nothing to read.
    tokio::task::spawn_blocking(move || hid.suspend_io()).await.unwrap();
}
