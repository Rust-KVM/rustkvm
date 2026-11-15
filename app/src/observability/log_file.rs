use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use tracing_subscriber::fmt::MakeWriter;

/// Size at which the log file is rotated. With one rotated file kept, the log
/// never takes more than twice this on the data partition.
pub const DEFAULT_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// An append-only log file that renames itself to `<name>.1` (replacing the
/// previous one) once it reaches `max_bytes`, so it cannot fill the eMMC.
pub struct RotatingLog {
    path: PathBuf,
    max_bytes: u64,
    state: Mutex<State>,
}

struct State {
    file: File,
    written: u64,
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

impl RotatingLog {
    pub fn open(path: impl Into<PathBuf>, max_bytes: u64) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = open_append(&path)?;
        let written = file.metadata()?.len();
        Ok(Self { path, max_bytes, state: Mutex::new(State { file, written }) })
    }

    pub fn rotated_path(&self) -> PathBuf {
        let mut name = self.path.clone().into_os_string();
        name.push(".1");
        PathBuf::from(name)
    }

    fn rotate(&self, state: &mut State) -> io::Result<()> {
        std::fs::rename(&self.path, self.rotated_path())?;
        state.file = open_append(&self.path)?;
        state.written = 0;
        Ok(())
    }
}

impl Write for &RotatingLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.state.lock();
        // A failed rotation must not lose the event: keep appending to the
        // current file and try again on the next write.
        if state.written > 0 && state.written + buf.len() as u64 > self.max_bytes {
            let _ = self.rotate(&mut state);
        }
        let n = state.file.write(buf)?;
        state.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.state.lock().file.flush()
    }
}

impl<'a> MakeWriter<'a> for RotatingLog {
    type Writer = &'a RotatingLog;

    fn make_writer(&'a self) -> Self::Writer {
        self
    }
}
