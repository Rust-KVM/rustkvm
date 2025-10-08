use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json;
use tokio::fs;
use tracing::{debug, info, trace, warn};

use crate::config::types::Config;
use crate::hardware::hw::get_device_id;

/// Default configuration file path
const DEFAULT_CONFIG_PATH: &str = "/userdata/kvm_config.json";

/// Configuration persistence layer
#[derive(Debug, Clone)]
pub struct ConfigPersistence {
    config_path: PathBuf,
}

impl ConfigPersistence {
    /// Create new persistence layer with default path
    pub fn new() -> Self {
        Self { config_path: PathBuf::from(DEFAULT_CONFIG_PATH) }
    }

    /// Create new persistence layer with custom path
    pub fn with_path<P: AsRef<Path>>(path: P) -> Self {
        Self { config_path: path.as_ref().to_path_buf() }
    }

    /// Load configuration from file system
    pub async fn load(&self) -> Result<Config> {
        trace!("Loading configuration from {:?}", self.config_path);

        // Check if config file exists
        if !self.config_path.exists() {
            debug!("Configuration file doesn't exist, using defaults");
            return Ok(Config::default());
        }

        // Read file contents
        let contents = fs::read_to_string(&self.config_path)
            .await
            .with_context(|| format!("Failed to read config file: {:?}", self.config_path))?;

        if contents.trim().is_empty() {
            debug!("Configuration file is empty, using defaults");
            return Ok(Config::default());
        }

        // Parse JSON and merge with defaults
        let mut loaded_config = self.parse_and_merge_config(&contents)?;

        // Validate the loaded configuration
        if let Err(errors) = loaded_config.validate() {
            warn!("Configuration validation failed: {:?}", errors);
            // Return default config if validation fails
            return Ok(Config::default());
        }

        info!("Configuration loaded successfully from {:?}", self.config_path);
        Ok(loaded_config)
    }

    /// Save configuration to file system
    pub async fn save(&self, config: &Config) -> Result<()> {
        trace!("Saving configuration to {:?}", self.config_path);

        // Ensure parent directory exists
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("Failed to create config directory: {:?}", parent))?;
        }

        // Serialize configuration to JSON
        let json_content = serde_json::to_string_pretty(config)
            .with_context(|| "Failed to serialize configuration to JSON")?;

        // Write to file atomically
        let temp_path = self.config_path.with_extension("tmp");
        fs::write(&temp_path, json_content)
            .await
            .with_context(|| format!("Failed to write temporary config file: {:?}", temp_path))?;

        // Rename to final path (atomic operation on most filesystems)
        fs::rename(&temp_path, &self.config_path).await.with_context(|| {
            format!("Failed to rename config file: {:?} -> {:?}", temp_path, self.config_path)
        })?;

        info!("Configuration saved successfully to {:?}", self.config_path);
        Ok(())
    }

    /// Check if configuration file exists
    pub fn exists(&self) -> bool {
        self.config_path.exists()
    }

    /// Get the configuration file path
    pub fn path(&self) -> &Path {
        &self.config_path
    }

    /// Parse JSON and merge with default configuration
    fn parse_and_merge_config(&self, contents: &str) -> Result<Config> {
        // Parse the loaded configuration
        let mut loaded_config: Config =
            serde_json::from_str(contents).with_context(|| "Failed to parse configuration JSON")?;

        // Get default configuration for fallback values
        let default_config = Config::default();

        // Merge critical fields that might be missing in loaded config
        if loaded_config.device_id.is_empty() {
            loaded_config.device_id = get_device_id();
        }

        if loaded_config.cloud_url.is_empty() {
            loaded_config.cloud_url = default_config.cloud_url;
        }

        if loaded_config.cloud_app_url.is_empty() {
            loaded_config.cloud_app_url = default_config.cloud_app_url;
        }

        if loaded_config.default_log_level.is_empty() {
            loaded_config.default_log_level = default_config.default_log_level;
        }

        Ok(loaded_config)
    }
}

impl Default for ConfigPersistence {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn test_save_and_load() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.json");
        let persistence = ConfigPersistence::with_path(&config_path);

        // Create test configuration
        let mut config = Config::default();
        config.local_auth_mode = "password".to_string();
        config.display_max_brightness = 100;

        // Save configuration
        persistence.save(&config).await.unwrap();

        // Load configuration
        let loaded_config = persistence.load().await.unwrap();

        // Verify loaded configuration matches saved configuration
        assert_eq!(loaded_config.local_auth_mode, "password");
        assert_eq!(loaded_config.display_max_brightness, 100);
        assert_eq!(loaded_config.cloud_url, config.cloud_url); // Should maintain defaults
    }

    #[tokio::test]
    async fn test_load_nonexistent_file() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("nonexistent.json");
        let persistence = ConfigPersistence::with_path(&config_path);

        // Loading non-existent file should return default config
        let config = persistence.load().await.unwrap();
        assert_eq!(config.local_auth_mode, "noPassword");
        assert_eq!(config.cloud_url, "https://api.rustkvm.com");
    }

    #[tokio::test]
    async fn test_load_invalid_json() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("invalid.json");
        let persistence = ConfigPersistence::with_path(&config_path);

        // Write invalid JSON
        fs::write(&config_path, "{ invalid json }").await.unwrap();

        // Loading invalid JSON should return error
        assert!(persistence.load().await.is_err());
    }

    #[tokio::test]
    async fn test_exists() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test.json");
        let persistence = ConfigPersistence::with_path(&config_path);

        // File doesn't exist initially
        assert!(!persistence.exists());

        // Save configuration
        let config = Config::default();
        persistence.save(&config).await.unwrap();

        // File should exist now
        assert!(persistence.exists());
    }
}
