use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail, ensure};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Notify;
use tracing::{error, info, warn};

pub const ASSET_NAME: &str = "rustkvm_app-aarch64-unknown-linux-gnu";
pub const MAX_BOOT_ATTEMPTS: u32 = 3;
const RELEASES_API: &str = "https://api.github.com/repos/Rust-KVM/rustkvm/releases?per_page=30";
pub const RELEASE_DOWNLOAD_PREFIX: &str = "https://github.com/Rust-KVM/rustkvm/releases/download/";
const MAX_BINARY_BYTES: usize = 128 * 1024 * 1024;
const CONFIRM_AFTER: Duration = Duration::from_secs(120);
const EM_AARCH64: u16 = 0xB7;

#[derive(Debug, Clone)]
pub struct InstallPaths {
    current: PathBuf,
}

impl InstallPaths {
    pub fn new(current: impl Into<PathBuf>) -> Self {
        Self { current: current.into() }
    }

    pub fn current(&self) -> &Path {
        &self.current
    }

    pub fn staged(&self) -> PathBuf {
        self.sibling("new")
    }

    pub fn previous(&self) -> PathBuf {
        self.sibling("prev")
    }

    pub fn marker(&self) -> PathBuf {
        self.sibling("update-pending.json")
    }

    fn sibling(&self, suffix: &str) -> PathBuf {
        let mut name = self.current.file_name().unwrap_or_default().to_os_string();
        name.push(".");
        name.push(suffix);
        self.current.with_file_name(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingUpdate {
    pub from_version: String,
    pub to_version: String,
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootDecision {
    Normal,
    Probation(PendingUpdate),
    RolledBack(PendingUpdate),
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseInfo {
    pub version: String,
    pub url: String,
    pub sha256_url: String,
    pub prerelease: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub state: String,
    pub target_version: Option<String>,
    pub error: Option<String>,
}

pub fn select_release(
    releases: &[GithubRelease],
    current: &str,
    include_pre_release: bool,
) -> Option<ReleaseInfo> {
    let current = semver::Version::parse(current.trim_start_matches('v')).ok()?;
    releases
        .iter()
        .filter(|r| !r.draft && (include_pre_release || !r.prerelease))
        .filter_map(|r| {
            let version = semver::Version::parse(r.tag_name.trim_start_matches('v')).ok()?;
            let asset = |name: &str| {
                r.assets.iter().find(|a| a.name == name).map(|a| a.browser_download_url.clone())
            };
            let url = asset(ASSET_NAME).filter(|u| is_official_download(u))?;
            let sha256_url =
                asset(&format!("{ASSET_NAME}.sha256")).filter(|u| is_official_download(u))?;
            Some((
                version,
                ReleaseInfo {
                    version: r.tag_name.trim_start_matches('v').to_string(),
                    url,
                    sha256_url,
                    prerelease: r.prerelease,
                },
            ))
        })
        .filter(|(version, _)| *version > current)
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, info)| info)
}

pub fn is_official_download(url: &str) -> bool {
    url.strip_prefix(RELEASE_DOWNLOAD_PREFIX).is_some_and(|rest| !rest.contains(".."))
}

pub fn parse_sha256(text: &str) -> Result<[u8; 32]> {
    let token = text.split_whitespace().next().ok_or_else(|| anyhow!("empty sha256 file"))?;
    let bytes = hex::decode(token).context("sha256 is not hex")?;
    bytes.try_into().map_err(|_| anyhow!("sha256 must be 32 bytes"))
}

pub fn verify_aarch64_elf(bytes: &[u8]) -> Result<()> {
    ensure!(bytes.len() >= 20 && bytes[..4] == *b"\x7fELF", "not an ELF binary");
    ensure!(bytes[4] == 2 && bytes[5] == 1, "not a 64-bit little-endian ELF");
    let machine = u16::from_le_bytes([bytes[18], bytes[19]]);
    ensure!(machine == EM_AARCH64, "ELF machine {machine:#x} is not aarch64");
    Ok(())
}

pub fn stage_binary(paths: &InstallPaths, bytes: &[u8], expected_sha256: &[u8; 32]) -> Result<()> {
    let actual = Sha256::digest(bytes);
    ensure!(actual.as_slice() == expected_sha256, "sha256 mismatch");
    verify_aarch64_elf(bytes)?;

    let staged = paths.staged();
    let mut file = fs::File::create(&staged).with_context(|| format!("create {staged:?}"))?;
    file.write_all(bytes)?;
    file.set_permissions(fs::Permissions::from_mode(0o755))?;
    file.sync_all()?;
    Ok(())
}

pub fn install_staged(paths: &InstallPaths, from_version: &str, to_version: &str) -> Result<()> {
    let staged = paths.staged();
    ensure!(staged.exists(), "no staged binary at {staged:?}");
    fs::rename(paths.current(), paths.previous()).context("back up current binary")?;
    if let Err(e) = fs::rename(&staged, paths.current()) {
        fs::rename(paths.previous(), paths.current()).context("restore after failed install")?;
        bail!("install staged binary: {e}");
    }
    write_marker(
        paths,
        &PendingUpdate {
            from_version: from_version.to_string(),
            to_version: to_version.to_string(),
            attempts: 0,
        },
    )?;
    sync_dir(paths.current())
}

/// Counts boots of a freshly installed binary and swaps the previous one back once
/// the new binary has failed to confirm itself `MAX_BOOT_ATTEMPTS` times; a crash
/// lets the hardware watchdog reboot the device, which lands back here.
pub fn on_boot(paths: &InstallPaths) -> Result<BootDecision> {
    let marker = paths.marker();
    let Ok(raw) = fs::read(&marker) else {
        return Ok(BootDecision::Normal);
    };
    let mut pending: PendingUpdate = serde_json::from_slice(&raw).context("parse update marker")?;
    pending.attempts += 1;

    if pending.attempts > MAX_BOOT_ATTEMPTS {
        if !paths.previous().exists() {
            fs::remove_file(&marker)?;
            bail!("update failed {} boots but no previous binary to restore", MAX_BOOT_ATTEMPTS);
        }
        fs::rename(paths.previous(), paths.current()).context("restore previous binary")?;
        fs::remove_file(&marker)?;
        sync_dir(paths.current())?;
        return Ok(BootDecision::RolledBack(pending));
    }
    write_marker(paths, &pending)?;
    Ok(BootDecision::Probation(pending))
}

pub fn confirm(paths: &InstallPaths) -> Result<()> {
    match fs::remove_file(paths.marker()) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

fn write_marker(paths: &InstallPaths, pending: &PendingUpdate) -> Result<()> {
    let marker = paths.marker();
    let tmp = marker.with_extension("json.tmp");
    let mut file = fs::File::create(&tmp)?;
    file.write_all(&serde_json::to_vec(pending)?)?;
    file.sync_all()?;
    fs::rename(&tmp, &marker)?;
    Ok(())
}

fn sync_dir(path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::File::open(dir)?.sync_all()?;
    }
    Ok(())
}

static PATHS: OnceLock<InstallPaths> = OnceLock::new();
static PROGRESS: RwLock<Option<UpdateProgress>> = RwLock::new(None);
static IN_PROGRESS: AtomicBool = AtomicBool::new(false);
static RESTART: OnceLock<Notify> = OnceLock::new();
static RESTART_REQUESTED: AtomicBool = AtomicBool::new(false);

fn restart_notify() -> &'static Notify {
    RESTART.get_or_init(Notify::new)
}

/// Must run before anything renames the running binary, because `/proc/self/exe`
/// follows the rename.
pub fn init_paths() -> Result<&'static InstallPaths> {
    if let Some(paths) = PATHS.get() {
        return Ok(paths);
    }
    let exe = std::env::current_exe().context("resolve current executable")?;
    Ok(PATHS.get_or_init(|| InstallPaths::new(exe)))
}

pub fn paths() -> Result<&'static InstallPaths> {
    PATHS.get().ok_or_else(|| anyhow!("update paths not initialized"))
}

pub fn progress() -> UpdateProgress {
    PROGRESS
        .read()
        .clone()
        .unwrap_or_else(|| UpdateProgress { state: "idle".into(), ..Default::default() })
}

fn set_progress(state: &str, target: Option<&str>, error: Option<String>) {
    *PROGRESS.write() = Some(UpdateProgress {
        state: state.to_string(),
        target_version: target.map(str::to_string),
        error,
    });
}

pub fn is_pending() -> bool {
    IN_PROGRESS.load(Ordering::Relaxed)
        || paths().is_ok_and(|p| p.marker().exists() || p.staged().exists())
}

pub async fn restart_requested() {
    loop {
        let notified = restart_notify().notified();
        if RESTART_REQUESTED.load(Ordering::Acquire) {
            return;
        }
        notified.await;
    }
}

pub fn should_restart() -> bool {
    RESTART_REQUESTED.load(Ordering::Acquire)
}

fn request_restart() {
    RESTART_REQUESTED.store(true, Ordering::Release);
    restart_notify().notify_waiters();
}

/// Replaces this process with the binary at the install path, keeping the CLI args.
pub fn exec_current() -> anyhow::Error {
    use std::os::unix::process::CommandExt as _;
    match paths() {
        Ok(paths) => {
            let err = std::process::Command::new(paths.current())
                .args(std::env::args_os().skip(1))
                .exec();
            anyhow!("exec {:?}: {err}", paths.current())
        }
        Err(e) => e,
    }
}

pub fn spawn_confirm_after_probation(pending: PendingUpdate) {
    tokio::spawn(async move {
        tokio::time::sleep(CONFIRM_AFTER).await;
        let result = paths().and_then(confirm);
        match result {
            Ok(()) => info!(version = %pending.to_version, "update confirmed after probation"),
            Err(e) => warn!(error = %e, "failed to confirm update"),
        }
    });
}

fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("rustkvm/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(600))
        .build()?)
}

pub async fn check_latest(include_pre_release: bool) -> Result<Option<ReleaseInfo>> {
    let releases: Vec<GithubRelease> = http_client()?
        .get(RELEASES_API)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .timeout(Duration::from_secs(8))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(select_release(&releases, crate::version::built_app_version(), include_pre_release))
}

async fn download(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>> {
    let mut response = client.get(url).send().await?.error_for_status()?;
    if let Some(len) = response.content_length() {
        ensure!(len as usize <= limit, "download is {len} bytes, limit {limit}");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(body.len() + chunk.len() <= limit, "download exceeds {limit} bytes");
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Starts a background download → verify → install → restart of an official release.
/// A failure at any step leaves the running binary untouched.
pub fn start_update(release: ReleaseInfo) -> Result<()> {
    paths()?;
    ensure!(
        is_official_download(&release.url) && is_official_download(&release.sha256_url),
        "update assets must come from {RELEASE_DOWNLOAD_PREFIX}"
    );
    ensure!(!IN_PROGRESS.swap(true, Ordering::AcqRel), "an update is already in progress");

    set_progress("downloading", Some(&release.version), None);
    tokio::spawn(async move {
        let outcome = run_update(&release).await;
        IN_PROGRESS.store(false, Ordering::Release);
        match outcome {
            Ok(()) => {
                set_progress("restarting", Some(&release.version), None);
                info!(version = %release.version, "update installed, restarting");
                request_restart();
            }
            Err(e) => {
                error!(error = %format!("{e:#}"), "update failed");
                if let Ok(paths) = paths() {
                    let _ = fs::remove_file(paths.staged());
                }
                set_progress("failed", Some(&release.version), Some(format!("{e:#}")));
            }
        }
    });
    Ok(())
}

async fn run_update(release: &ReleaseInfo) -> Result<()> {
    let client = http_client()?;
    let sha = download(&client, &release.sha256_url, 4096).await?;
    let expected = parse_sha256(std::str::from_utf8(&sha).context("sha256 file is not UTF-8")?)?;
    let bytes = download(&client, &release.url, MAX_BINARY_BYTES).await?;
    set_progress("installing", Some(&release.version), None);
    let paths = paths()?.clone();
    let from = crate::version::built_app_version().to_string();
    let target = release.version.clone();
    tokio::task::spawn_blocking(move || {
        stage_binary(&paths, &bytes, &expected)?;
        install_staged(&paths, &from, &target)
    })
    .await?
}
