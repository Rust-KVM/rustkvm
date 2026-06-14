use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::assets::BuiltinImages;
use crate::hardware::block_device::{
    NbdDevice, RemoteImageReader, set_current_remote_image_reader,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VirtualMediaSource {
    WebRTC,
    HTTP,
    Storage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VirtualMediaMode {
    CDROM,
    Disk,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualMediaState {
    pub source: VirtualMediaSource,
    pub mode: VirtualMediaMode,
    pub filename: Option<String>,
    pub url: Option<String>,
    pub size: i64,
}

static CURRENT_VIRTUAL_MEDIA_STATE: RwLock<Option<VirtualMediaState>> = RwLock::new(None);
static NBD_DEVICE: RwLock<Option<NbdDevice>> = RwLock::new(None);

const IMAGES_FOLDER: &str = "/userdata/rustkvm/images";

pub fn get_virtual_media_state() -> Option<VirtualMediaState> {
    CURRENT_VIRTUAL_MEDIA_STATE.read().clone()
}

pub async fn mount_with_http(url: &str, mode: VirtualMediaMode) -> Result<()> {
    ensure_images_folder().await?;
    set_mass_storage_mode(mode == VirtualMediaMode::CDROM)
        .await
        .context("failed to set mass storage mode")?;

    let reader = HttpRangeReader::new(url).context("failed to init HTTP reader")?;
    let size = reader.size().context("failed to get HTTP size")?;
    info!(url, size, "using remote HTTP url");

    {
        let mut st = CURRENT_VIRTUAL_MEDIA_STATE.write();
        if st.is_some() {
            return Err(anyhow!("another virtual media is already mounted"));
        }
        *st = Some(VirtualMediaState {
            source: VirtualMediaSource::HTTP,
            mode,
            filename: None,
            url: Some(url.to_string()),
            size,
        });
    }

    set_current_remote_image_reader(Some(Arc::new(reader)));
    start_nbd()?;

    sleep(Duration::from_secs(1)).await;
    set_mass_storage_image("/dev/nbd0")
        .await
        .context("failed to set mass storage image to nbd0")?;
    info!("usb mass storage mounted");
    Ok(())
}

pub async fn mount_with_webrtc(filename: &str, size: i64, mode: VirtualMediaMode) -> Result<()> {
    set_mass_storage_mode(mode == VirtualMediaMode::CDROM)
        .await
        .context("failed to set mass storage mode")?;

    {
        let mut st = CURRENT_VIRTUAL_MEDIA_STATE.write();
        if st.is_some() {
            return Err(anyhow!("another virtual media is already mounted"));
        }
        *st = Some(VirtualMediaState {
            source: VirtualMediaSource::WebRTC,
            mode,
            filename: Some(filename.to_string()),
            url: None,
            size,
        });
    }

    let reader = WebRtcDiskReader::new(size);
    set_current_remote_image_reader(Some(Arc::new(reader)));

    start_nbd()?;
    sleep(Duration::from_secs(1)).await;
    set_mass_storage_image("/dev/nbd0")
        .await
        .context("failed to set mass storage image to nbd0")?;
    info!("usb mass storage mounted");
    Ok(())
}

pub async fn mount_with_storage(filename: &str, mode: VirtualMediaMode) -> Result<()> {
    let filename = sanitize_filename(filename)?;
    ensure_images_folder().await?;
    let full_path = Path::new(IMAGES_FOLDER).join(&filename);
    let meta = fs::metadata(&full_path)
        .await
        .with_context(|| format!("failed to stat file: {}", filename))?;

    set_mass_storage_mode(mode == VirtualMediaMode::CDROM)
        .await
        .context("failed to set mass storage mode")?;
    set_mass_storage_image(full_path.to_str().ok_or_else(|| anyhow!("invalid path"))?)
        .await
        .context("failed to set mass storage image")?;

    *CURRENT_VIRTUAL_MEDIA_STATE.write() = Some(VirtualMediaState {
        source: VirtualMediaSource::Storage,
        mode,
        filename: Some(filename),
        url: None,
        size: meta.len() as i64,
    });
    Ok(())
}

pub async fn unmount_image() -> Result<()> {
    set_mass_storage_image("\n")
        .await
        .unwrap_or_else(|e| warn!("failed to clear mass storage image: {}", e));
    sleep(Duration::from_millis(500)).await;
    stop_nbd();
    *CURRENT_VIRTUAL_MEDIA_STATE.write() = None;
    set_current_remote_image_reader(None);
    Ok(())
}

pub async fn ensure_images_folder() -> Result<()> {
    fs::create_dir_all(IMAGES_FOLDER).await.context("failed to create images folder")
}

fn sanitize_filename(filename: &str) -> Result<String> {
    use std::path::Component;
    let p = Path::new(filename);
    if p.is_absolute() {
        return Err(anyhow!("invalid filename"));
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(anyhow!("invalid filename"));
    }
    let base = p.file_name().and_then(|s| s.to_str()).ok_or_else(|| anyhow!("invalid filename"))?;
    if base.is_empty() || base == "." {
        return Err(anyhow!("invalid filename"));
    }
    Ok(base.to_string())
}

async fn resolve_mass_storage_lun0() -> Result<PathBuf> {
    let gadgets_root = Path::new("/sys/kernel/config/usb_gadget");
    let mut entries = fs::read_dir(gadgets_root).await.context("failed to read usb_gadget root")?;
    while let Some(entry) = entries.next_entry().await? {
        let p = entry.path().join("functions").join("mass_storage.usb0").join("lun.0");
        if p.exists() {
            return Ok(p);
        }
    }
    Err(anyhow!("mass_storage.usb0/lun.0 not found under configfs"))
}

async fn read_trimmed(path: &Path) -> Result<String> {
    let s = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(s.trim().to_string())
}

pub async fn set_mass_storage_image(image_path: &str) -> Result<()> {
    let lun = resolve_mass_storage_lun0().await?;
    let file_path = lun.join("file");
    write_file(&file_path, "\n").await.ok();
    write_file(&file_path, image_path).await?;
    write_file(&file_path, image_path).await?;
    Ok(())
}

pub async fn set_mass_storage_mode(cdrom: bool) -> Result<()> {
    let lun = resolve_mass_storage_lun0().await?;
    let cdrom_path = lun.join("cdrom");
    write_file(&cdrom_path, if cdrom { "1" } else { "0" }).await?;
    Ok(())
}

pub async fn get_mass_storage_cdrom_enabled() -> Result<bool> {
    let lun = resolve_mass_storage_lun0().await?;
    let cdrom_path = lun.join("cdrom");
    let s = read_trimmed(&cdrom_path).await?;
    Ok(s == "1")
}

pub async fn get_mass_storage_image() -> Result<String> {
    let lun = resolve_mass_storage_lun0().await?;
    read_trimmed(&lun.join("file")).await
}

pub async fn set_initial_virtual_media_state() -> Result<()> {
    let cdrom_enabled = get_mass_storage_cdrom_enabled().await.unwrap_or(false);
    let disk_path = match get_mass_storage_image().await {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };

    if disk_path.is_empty() {
        return Ok(());
    }

    let mode = if cdrom_enabled { VirtualMediaMode::CDROM } else { VirtualMediaMode::Disk };

    let state = if disk_path == "/dev/nbd0" {
        VirtualMediaState {
            source: VirtualMediaSource::HTTP,
            mode,
            filename: None,
            url: Some("/".to_string()),
            size: 1,
        }
    } else {
        let filename =
            Path::new(&disk_path).file_name().and_then(|s| s.to_str()).map(str::to_string);
        let size = fs::metadata(&disk_path).await.map(|m| m.len() as i64).unwrap_or(0);
        VirtualMediaState { source: VirtualMediaSource::Storage, mode, filename, url: None, size }
    };

    info!(?state, "initial virtual media state set");
    *CURRENT_VIRTUAL_MEDIA_STATE.write() = Some(state);
    Ok(())
}

async fn write_file(path: &Path, data: &str) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .open(path)
        .await
        .with_context(|| format!("failed to open {}", path.display()))?;
    let mut writer = BufWriter::new(&mut f);
    writer
        .write_all(data.as_bytes())
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;
    writer.flush().await.ok();
    Ok(())
}

fn start_nbd() -> Result<()> {
    let mut guard = NBD_DEVICE.write();
    if guard.is_none() {
        *guard = Some(NbdDevice::new());
    }
    if let Some(dev) = guard.as_mut() {
        dev.start().context("failed to start nbd device")?;
        debug!("nbd device started");
        return Ok(());
    }
    Err(anyhow!("nbd device not available"))
}

fn stop_nbd() {
    let mut guard = NBD_DEVICE.write();
    if let Some(dev) = guard.as_mut() {
        dev.close();
    }
    *guard = None;
}

struct HttpRangeReader {
    client: reqwest::blocking::Client,
    url: String,
    size: i64,
}

impl HttpRangeReader {
    fn new(url: &str) -> Result<Self> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(anyhow!("unsupported url scheme"));
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .context("failed to build http client")?;

        let mut size: Option<i64> = None;
        if let Ok(resp) = client.head(url).send()
            && resp.status().is_success()
            && let Some(len) = resp.headers().get(reqwest::header::CONTENT_LENGTH)
            && let Ok(s) = len.to_str()
            && let Ok(n) = s.parse::<i64>()
        {
            size = Some(n);
        }
        if size.is_none() {
            let resp = client
                .get(url)
                .header(reqwest::header::RANGE, "bytes=0-0")
                .send()
                .context("range request failed")?;
            if !resp.status().is_success() && resp.status() != reqwest::StatusCode::PARTIAL_CONTENT
            {
                return Err(anyhow!("range probe failed with status {}", resp.status()));
            }
            if let Some(cr) = resp.headers().get(reqwest::header::CONTENT_RANGE)
                && let Ok(s) = cr.to_str()
                && let Some((_, total)) = s.rsplit_once('/')
                && let Ok(n) = total.trim().parse::<i64>()
            {
                size = Some(n);
            }
            if size.is_none()
                && let Some(len) = resp.headers().get(reqwest::header::CONTENT_LENGTH)
                && let Ok(s) = len.to_str()
                && let Ok(n) = s.parse::<i64>()
            {
                size = Some(n);
            }
        }

        let size = size.ok_or_else(|| anyhow!("failed to determine remote size"))?;
        Ok(Self { client, url: url.to_string(), size })
    }
}

impl RemoteImageReader for HttpRangeReader {
    fn read_at(&self, off: i64, len: i64) -> Result<Vec<u8>> {
        if off < 0 || len < 0 {
            return Err(anyhow!("invalid range"));
        }
        let end = (off + len - 1).max(off);
        let resp = self
            .client
            .get(&self.url)
            .header(reqwest::header::RANGE, format!("bytes={}-{}", off, end))
            .send()
            .context("range request failed")?;
        if !(resp.status().is_success() || resp.status() == reqwest::StatusCode::PARTIAL_CONTENT) {
            return Err(anyhow!("GET range request failed with status {}", resp.status()));
        }
        let bytes = resp.bytes().context("failed to read response body")?;
        Ok(bytes.to_vec())
    }
    fn size(&self) -> Result<i64> {
        Ok(self.size)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UrlCheckResult {
    #[serde(rename = "Usable")]
    pub usable: bool,
    #[serde(rename = "Reason", skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(rename = "Size")]
    pub size: i64,
}

pub fn check_mount_url(url: &str) -> Result<UrlCheckResult> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Ok(UrlCheckResult {
            usable: false,
            reason: Some("unsupported scheme".into()),
            size: 0,
        });
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("build http client")?;

    if let Ok(resp) = client.head(url).send()
        && resp.status().is_success()
        && let Some(len) = resp.headers().get(reqwest::header::CONTENT_LENGTH)
        && let Ok(s) = len.to_str()
        && let Ok(n) = s.parse::<i64>()
    {
        return Ok(UrlCheckResult { usable: true, reason: None, size: n });
    }

    match client.get(url).header(reqwest::header::RANGE, "bytes=0-0").send() {
        Ok(resp) => {
            if !(resp.status().is_success()
                || resp.status() == reqwest::StatusCode::PARTIAL_CONTENT)
            {
                return Ok(UrlCheckResult {
                    usable: false,
                    reason: Some(format!("status {}", resp.status())),
                    size: 0,
                });
            }
            if let Some(cr) = resp.headers().get(reqwest::header::CONTENT_RANGE)
                && let Ok(s) = cr.to_str()
                && let Some((_, total)) = s.rsplit_once('/')
                && let Ok(n) = total.trim().parse::<i64>()
            {
                return Ok(UrlCheckResult { usable: true, reason: None, size: n });
            }
            if let Some(len) = resp.headers().get(reqwest::header::CONTENT_LENGTH)
                && let Ok(s) = len.to_str()
                && let Ok(n) = s.parse::<i64>()
            {
                return Ok(UrlCheckResult { usable: true, reason: None, size: n });
            }
            Ok(UrlCheckResult { usable: true, reason: None, size: 0 })
        }
        Err(e) => Ok(UrlCheckResult { usable: false, reason: Some(e.to_string()), size: 0 }),
    }
}

pub trait WebRtcReadHandler: Send + Sync {
    fn read(&self, offset: i64, size: i64) -> Result<Vec<u8>>;
}

static WEBRTC_READ_HANDLER: RwLock<Option<Arc<dyn WebRtcReadHandler>>> = RwLock::new(None);

pub fn set_webrtc_read_handler(handler: Option<Arc<dyn WebRtcReadHandler>>) {
    *WEBRTC_READ_HANDLER.write() = handler;
}

struct WebRtcDiskReader {
    size: i64,
}

impl WebRtcDiskReader {
    fn new(size: i64) -> Self {
        Self { size }
    }
}

impl RemoteImageReader for WebRtcDiskReader {
    fn read_at(&self, off: i64, len: i64) -> Result<Vec<u8>> {
        let handler =
            WEBRTC_READ_HANDLER.read().clone().ok_or_else(|| anyhow!("not active session"))?;
        if off < 0 || len < 0 {
            return Err(anyhow!("invalid range"));
        }
        let end = (off + len).min(self.size);
        let req_len = (end - off).max(0);
        if req_len == 0 {
            return Ok(Vec::new());
        }
        handler.read(off, req_len)
    }
    fn size(&self) -> Result<i64> {
        Ok(self.size)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageFileUpload {
    #[serde(rename = "alreadyUploadedBytes")]
    pub already_uploaded_bytes: i64,
    #[serde(rename = "dataChannel")]
    pub data_channel: String,
}

struct PendingUpload {
    file: File,
    size: i64,
    already_uploaded_bytes: i64,
    upload_path: PathBuf,
    final_path: PathBuf,
}

static PENDING_UPLOADS: Lazy<Mutex<HashMap<String, PendingUpload>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub async fn start_storage_file_upload(filename: &str, size: i64) -> Result<StorageFileUpload> {
    let filename = sanitize_filename(filename)?;
    ensure_images_folder().await?;

    let final_path = Path::new(IMAGES_FOLDER).join(&filename);
    let mut upload_path = final_path.clone();
    let base = final_path.file_name().ok_or_else(|| anyhow!("invalid filename"))?;
    let mut appended = std::ffi::OsString::from(base);
    appended.push(".incomplete");
    upload_path.set_file_name(appended);

    if final_path.exists() {
        return Err(anyhow!("file already exists: {}", filename));
    }

    let mut already_uploaded_bytes: i64 = 0;
    if let Ok(meta) = fs::metadata(&upload_path).await {
        already_uploaded_bytes = meta.len() as i64;
    }

    let file =
        OpenOptions::new().append(true).create(true).open(&upload_path).await.with_context(
            || format!("failed to open file for upload: {}", upload_path.display()),
        )?;

    let upload_id = format!("upload_{}", Uuid::new_v4());
    {
        let mut map = PENDING_UPLOADS.lock().await;
        map.insert(
            upload_id.clone(),
            PendingUpload { file, size, already_uploaded_bytes, upload_path, final_path },
        );
    }

    Ok(StorageFileUpload { already_uploaded_bytes, data_channel: upload_id })
}

pub async fn append_upload_data(upload_id: &str, data: &[u8]) -> Result<i64> {
    let mut map = PENDING_UPLOADS.lock().await;
    let Some(entry) = map.get_mut(upload_id) else {
        return Err(anyhow!("upload not found"));
    };
    entry
        .file
        .write_all(data)
        .await
        .with_context(|| format!("failed to write upload: {}", upload_id))?;
    entry.already_uploaded_bytes += data.len() as i64;
    Ok(entry.already_uploaded_bytes)
}

pub async fn complete_upload(upload_id: &str) -> Result<()> {
    let entry = {
        let mut map = PENDING_UPLOADS.lock().await;
        map.remove(upload_id)
    };
    let Some(mut entry) = entry else {
        return Err(anyhow!("upload not found"));
    };
    entry.file.flush().await.ok();
    entry.file.sync_data().await.ok();
    drop(entry.file);
    if entry.already_uploaded_bytes == entry.size {
        fs::rename(&entry.upload_path, &entry.final_path).await.with_context(|| {
            format!(
                "failed to rename uploaded file: {} -> {}",
                entry.upload_path.display(),
                entry.final_path.display()
            )
        })?;
    }
    Ok(())
}

pub async fn get_upload_progress(upload_id: &str) -> Result<(i64, i64)> {
    let map = PENDING_UPLOADS.lock().await;
    let entry = map.get(upload_id).ok_or_else(|| anyhow!("upload not found"))?;
    Ok((entry.size, entry.already_uploaded_bytes))
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageFileEntry {
    pub filename: String,
    pub size: i64,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "totalBytes")]
    pub total_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageFilesList {
    pub files: Vec<StorageFileEntry>,
}

pub async fn list_storage_files() -> Result<StorageFilesList> {
    ensure_images_folder().await?;

    let mut pending_totals: HashMap<String, i64> = HashMap::new();
    {
        let map = PENDING_UPLOADS.lock().await;
        for pu in map.values() {
            if let Some(name) = pu.upload_path.file_name().and_then(|s| s.to_str()) {
                pending_totals.insert(name.to_string(), pu.size);
            }
        }
    }

    let mut out = Vec::new();
    let mut entries = fs::read_dir(IMAGES_FOLDER).await.context("failed to read images folder")?;
    while let Some(entry) = entries.next_entry().await? {
        let meta = entry.metadata().await?;
        if meta.is_file() {
            let name = entry.file_name();
            let filename = name.to_string_lossy().to_string();
            let size = meta.len() as i64;
            let created_at = meta
                .modified()
                .ok()
                .map(|t| {
                    chrono::DateTime::<chrono::Utc>::from(t)
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                })
                .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string());
            let total_bytes = if filename.ends_with(".incomplete") {
                pending_totals.get(&filename).copied().unwrap_or(size)
            } else {
                size
            };
            out.push(StorageFileEntry { filename, size, created_at, total_bytes });
        }
    }
    Ok(StorageFilesList { files: out })
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageSpace {
    #[serde(rename = "bytesUsed")]
    pub bytes_used: i64,
    #[serde(rename = "bytesFree")]
    pub bytes_free: i64,
}

pub async fn get_storage_space() -> Result<StorageSpace> {
    let path = Path::new(IMAGES_FOLDER);
    tokio::task::spawn_blocking(move || {
        use rustix::fs::statfs;
        let st = statfs(path).context("failed to statfs images folder")?;
        let total = st.f_blocks as u128 * st.f_bsize as u128;
        let free = st.f_bfree as u128 * st.f_bsize as u128;
        let used = total.saturating_sub(free);
        Ok(StorageSpace { bytes_used: used as i64, bytes_free: free as i64 })
    })
    .await
    .context("spawn_blocking failed")?
}

pub async fn delete_storage_file(filename: &str) -> Result<()> {
    let filename = sanitize_filename(filename)?;
    let full_path = Path::new(IMAGES_FOLDER).join(&filename);
    if !full_path.exists() {
        return Err(anyhow!("file does not exist: {}", filename));
    }
    fs::remove_file(&full_path)
        .await
        .with_context(|| format!("failed to delete file: {}", filename))?;
    Ok(())
}

pub async fn mount_built_in_image(filename: &str) -> Result<()> {
    let name = sanitize_filename(filename)?;
    ensure_images_folder().await?;
    let image_path = Path::new(IMAGES_FOLDER).join(&name);
    if image_path.exists() {
        return mount_with_storage(&name, VirtualMediaMode::Disk).await;
    }

    if let Some(data) = BuiltinImages::get(&name) {
        let bytes = data.data.as_ref();
        let mut file = fs::File::create(&image_path)
            .await
            .with_context(|| format!("failed to create file: {}", image_path.display()))?;

        let mut writer = BufWriter::new(&mut file);
        writer
            .write_all(bytes)
            .await
            .with_context(|| format!("failed to write embedded image: {}", image_path.display()))?;
        writer.flush().await.ok();
        drop(writer);

        return mount_with_storage(&name, VirtualMediaMode::Disk).await;
    }

    Err(anyhow!("image not found in built-in resources: {}", name))
}
