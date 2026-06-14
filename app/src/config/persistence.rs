use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, trace};

use crate::config::types::Config;

const DEFAULT_CONFIG_PATH: &str = "/userdata/rustkvm/config.toml";

pub(crate) async fn write_atomic_secret(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .with_context(|| format!("Failed to create directory: {parent:?}"))?;
    }

    let temp_path = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp_path)
        .await
        .with_context(|| format!("Failed to create temp file: {temp_path:?}"))?;
    file.write_all(contents)
        .await
        .with_context(|| format!("Failed to write temp file: {temp_path:?}"))?;
    file.sync_all().await.with_context(|| format!("Failed to fsync temp file: {temp_path:?}"))?;
    drop(file);

    fs::rename(&temp_path, path)
        .await
        .with_context(|| format!("Failed to rename {temp_path:?} -> {path:?}"))?;

    if let Some(parent) = path.parent()
        && let Ok(dir) = fs::File::open(parent).await
    {
        let _ = dir.sync_all().await;
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct ConfigPersistence {
    config_path: PathBuf,
}

impl ConfigPersistence {
    pub fn new() -> Self {
        Self { config_path: PathBuf::from(DEFAULT_CONFIG_PATH) }
    }

    pub fn with_path<P: AsRef<Path>>(path: P) -> Self {
        Self { config_path: path.as_ref().to_path_buf() }
    }

    pub async fn load(&self) -> Result<Config> {
        trace!("Loading configuration from {:?}", self.config_path);

        let contents = match fs::read_to_string(&self.config_path).await {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!("Configuration file doesn't exist, using defaults");
                return Ok(Config::default());
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    format!("Failed to read config file: {:?}", self.config_path)
                });
            }
        };

        if contents.trim().is_empty() {
            debug!("Configuration file is empty, using defaults");
            return Ok(Config::default());
        }

        let mut loaded = toml::from_str::<Config>(&contents)
            .with_context(|| "Failed to parse TOML configuration")?;

        if let Err(errors) = loaded.validate() {
            bail!("Configuration validation failed for {:?}: {errors:?}", self.config_path);
        }

        info!("Configuration loaded from {:?}", self.config_path);
        Ok(loaded)
    }

    pub async fn save(&self, config: &Config) -> Result<()> {
        trace!("Saving configuration to {:?}", self.config_path);

        let toml_content = toml::to_string_pretty(config)
            .with_context(|| "Failed to serialize configuration to TOML")?;

        write_atomic_secret(&self.config_path, toml_content.as_bytes()).await?;

        info!("Configuration saved to {:?}", self.config_path);
        Ok(())
    }

    pub fn exists(&self) -> bool {
        self.config_path.exists()
    }

    pub fn path(&self) -> &Path {
        &self.config_path
    }
}

impl Default for ConfigPersistence {
    fn default() -> Self {
        Self::new()
    }
}
