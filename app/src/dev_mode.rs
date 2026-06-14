use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{Context, Result, bail};
use rustix::fs::{Mode, OFlags, open, stat, unlink};
use rustix::io::{read, write};
use serde::Serialize;
use tracing::debug;

const DEV_MODE_FILE: &str = "/userdata/rustkvm/devmode.enable";
const SSH_KEY_DIR: &str = "/userdata/dropbear/.ssh";
const SSH_KEY_FILE: &str = "/userdata/dropbear/.ssh/authorized_keys";

const VALID_KEY_PREFIXES: &[&str] = &[
    "ssh-rsa",
    "ssh-ed25519",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

#[derive(Debug, Serialize)]
pub struct DevModeState {
    pub enabled: bool,
}

pub fn get_dev_mode_state() -> Result<DevModeState> {
    let enabled = match stat(DEV_MODE_FILE) {
        Ok(_) => true,
        Err(rustix::io::Errno::NOENT) => false,
        Err(e) => return Err(e).context("error checking dev mode file"),
    };
    debug!("dev mode state: {}", enabled);
    Ok(DevModeState { enabled })
}

pub fn set_dev_mode_state(enabled: bool) -> Result<()> {
    if enabled {
        match stat(DEV_MODE_FILE) {
            Err(rustix::io::Errno::NOENT) => {
                let dir = Path::new(DEV_MODE_FILE)
                    .parent()
                    .expect("DEV_MODE_FILE has no parent directory");
                std::fs::create_dir_all(dir)
                    .context("failed to create directory for devmode file")?;
                let _fd = open(
                    DEV_MODE_FILE,
                    OFlags::WRONLY | OFlags::CREATE | OFlags::TRUNC,
                    Mode::from_bits(0o644).expect("invalid mode bits"),
                )
                .context("failed to create devmode file")?;
                debug!("dev mode enabled");
            }
            Ok(_) => {
                debug!("dev mode already enabled");
            }
            Err(e) => return Err(e).context("error checking dev mode file"),
        }
    } else {
        match stat(DEV_MODE_FILE) {
            Ok(_) => {
                unlink(DEV_MODE_FILE).context("failed to remove devmode file")?;
                debug!("dev mode disabled");
            }
            Err(rustix::io::Errno::NOENT) => {
                debug!("dev mode already disabled");
            }
            Err(e) => return Err(e).context("error checking dev mode file"),
        }
    }
    Ok(())
}

pub fn get_ssh_key_state() -> Result<String> {
    match stat(SSH_KEY_FILE) {
        Ok(_) => {
            let fd = open(SSH_KEY_FILE, OFlags::RDONLY, Mode::empty())
                .context("error opening SSH key file")?;
            let mut content = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                match read(&fd, &mut buffer) {
                    Ok(0) => break,
                    Ok(n) => content.extend_from_slice(&buffer[..n]),
                    Err(e) => return Err(e).context("error reading SSH key file"),
                }
            }
            let data = String::from_utf8(content).context("SSH key file is not valid UTF-8")?;
            debug!("read SSH key state ({} bytes)", data.len());
            Ok(data)
        }
        Err(rustix::io::Errno::NOENT) => Ok(String::new()),
        Err(e) => Err(e).context("error reading SSH key file"),
    }
}

pub fn set_ssh_key_state(ssh_key: &str) -> Result<()> {
    if ssh_key.is_empty() {
        match unlink(SSH_KEY_FILE) {
            Ok(_) => {
                debug!("SSH key file removed");
                return Ok(());
            }
            Err(rustix::io::Errno::NOENT) => {
                debug!("SSH key file already absent");
                return Ok(());
            }
            Err(e) => return Err(e).context("failed to remove SSH key file"),
        }
    }

    validate_ssh_key(ssh_key)?;

    let dir = Path::new(SSH_KEY_DIR);
    std::fs::create_dir_all(dir).context("failed to create SSH key directory")?;

    let dir_meta = std::fs::metadata(dir).context("failed to stat SSH key directory")?;
    let mut dir_perm = dir_meta.permissions();
    #[cfg(unix)]
    {
        dir_perm.set_mode(0o700);
    }
    std::fs::set_permissions(dir, dir_perm)
        .context("failed to set SSH key directory permissions")?;

    let fd = open(
        SSH_KEY_FILE,
        OFlags::WRONLY | OFlags::CREATE | OFlags::TRUNC,
        Mode::from_bits(0o600).expect("invalid mode bits"),
    )
    .context("failed to write SSH key")?;
    write(&fd, ssh_key.as_bytes()).context("failed to write SSH key")?;

    debug!("SSH key written ({} bytes)", ssh_key.len());
    Ok(())
}

pub fn validate_ssh_key(key: &str) -> Result<()> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let mut has_valid = false;
    let mut last_error: Option<String> = None;

    for line in key.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let mut parts = trimmed.split_whitespace();
        let key_type = match parts.next() {
            Some(t) => t,
            None => {
                last_error = Some("empty key field in line".into());
                continue;
            }
        };

        if !VALID_KEY_PREFIXES.contains(&key_type) {
            last_error = Some(format!("unsupported SSH key type: {key_type}"));
            continue;
        }

        let blob = match parts.next() {
            Some(b) => b,
            None => {
                last_error = Some("missing key blob".into());
                continue;
            }
        };
        if engine.decode(blob).is_err() {
            last_error = Some("malformed key blob (not base64)".into());
            continue;
        }

        has_valid = true;
    }

    if !has_valid {
        let msg = last_error.unwrap_or_else(|| "no valid SSH public key found".into());
        bail!("SSH key validation failed: {}", msg);
    }

    Ok(())
}
