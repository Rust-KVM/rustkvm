use std::sync::Arc;

use anyhow::{Result, bail};
use bcrypt::{DEFAULT_COST, hash, verify};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::config::persistence::ConfigPersistence;
use crate::config::types::Config;

/// Global configuration manager with thread-safe access
#[derive(Debug)]
pub struct ConfigManager {
    config: Arc<RwLock<Config>>,
    persistence: ConfigPersistence,
}

impl ConfigManager {
    /// Create new configuration manager with default persistence
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(Config::default())),
            persistence: ConfigPersistence::new(),
        }
    }

    /// Create new configuration manager with custom persistence
    pub fn with_persistence(persistence: ConfigPersistence) -> Self {
        Self { config: Arc::new(RwLock::new(Config::default())), persistence }
    }

    /// Load configuration from persistent storage
    pub async fn load(&self) -> Result<()> {
        let loaded_config = self.persistence.load().await?;
        *self.config.write().await = loaded_config;
        info!("Configuration loaded successfully");
        Ok(())
    }

    /// Save current configuration to persistent storage
    pub async fn save(&self) -> Result<()> {
        let config = self.config.read().await;
        self.persistence.save(&config).await?;
        info!("Configuration saved successfully");
        Ok(())
    }

    /// Get a copy of the current configuration
    pub async fn get(&self) -> Config {
        self.config.read().await.clone()
    }

    /// Update configuration with validation and persistence
    pub async fn update<F>(&self, updater: F) -> Result<()>
    where
        F: FnOnce(&mut Config),
    {
        {
            let mut config = self.config.write().await;
            updater(&mut config);

            // Validate the updated configuration
            if let Err(errors) = config.validate() {
                warn!("Configuration validation failed: {:?}", errors);
                bail!("Configuration validation failed: {:?}", errors);
            }
        }

        self.save().await?;
        Ok(())
    }

    /// Set authentication mode
    pub async fn set_auth_mode(&self, mode: String) -> Result<()> {
        self.update(|config| {
            config.local_auth_mode = mode;
        })
        .await
    }

    /// Set hashed password
    pub async fn set_hashed_password(&self, hashed_password: Option<String>) -> Result<()> {
        self.update(|config| {
            config.hashed_password = hashed_password;
        })
        .await
    }

    /// Set authentication token
    pub async fn set_auth_token(&self, token: Option<String>) -> Result<()> {
        self.update(|config| {
            config.local_auth_token = token;
        })
        .await
    }

    /// Set cloud configuration
    pub async fn set_cloud_config(
        &self,
        cloud_url: Option<String>,
        cloud_token: Option<String>,
    ) -> Result<()> {
        self.update(|config| {
            if let Some(url) = cloud_url {
                config.cloud_url = url;
            }
            config.cloud_token = cloud_token;
        })
        .await
    }

    /// Add or update a keyboard macro
    pub async fn set_keyboard_macro(
        &self,
        mut macro_item: crate::config::types::KeyboardMacro,
    ) -> Result<()> {
        // Validate the macro before adding
        macro_item.validate()?;

        self.update(|config| {
            // Check if macro exists (by ID)
            if let Some(existing) =
                config.keyboard_macros.iter_mut().find(|m| m.id == macro_item.id)
            {
                *existing = macro_item.clone();
            } else {
                config.keyboard_macros.push(macro_item);
            }
        })
        .await
    }

    /// Remove a keyboard macro by ID
    pub async fn remove_keyboard_macro(&self, macro_id: &str) -> Result<()> {
        self.update(|config| {
            config.keyboard_macros.retain(|m| m.id != macro_id);
        })
        .await
    }

    /// Add or update a wake-on-LAN device
    pub async fn set_wake_on_lan_device(
        &self,
        device: crate::config::types::WakeOnLanDevice,
    ) -> Result<()> {
        self.update(|config| {
            // Check if device exists (by name)
            if let Some(existing) =
                config.wake_on_lan_devices.iter_mut().find(|d| d.name == device.name)
            {
                *existing = device.clone();
            } else {
                config.wake_on_lan_devices.push(device);
            }
        })
        .await
    }

    /// Remove a wake-on-LAN device by name
    pub async fn remove_wake_on_lan_device(&self, device_name: &str) -> Result<()> {
        self.update(|config| {
            config.wake_on_lan_devices.retain(|d| d.name != device_name);
        })
        .await
    }

    /// Set display configuration
    pub async fn set_display_config(
        &self,
        rotation: Option<String>,
        max_brightness: Option<u32>,
        dim_after_sec: Option<u32>,
        off_after_sec: Option<u32>,
    ) -> Result<()> {
        self.update(|config| {
            if let Some(r) = rotation {
                config.display_rotation = r;
            }
            if let Some(b) = max_brightness {
                config.display_max_brightness = b;
            }
            if let Some(d) = dim_after_sec {
                config.display_dim_after_sec = d;
            }
            if let Some(o) = off_after_sec {
                config.display_off_after_sec = o;
            }
        })
        .await
    }

    /// Check if device is set up (has auth mode configured)
    pub async fn is_setup(&self) -> bool {
        let config = self.get().await;
        !config.is_setup_required()
    }

    /// Validate authentication token
    pub async fn validate_auth_token(&self, token: &str) -> bool {
        let config = self.get().await;

        // If noPassword mode, no validation needed
        if config.local_auth_mode == "noPassword" {
            return true;
        }

        // Check if token matches
        if let Some(ref local_token) = config.local_auth_token {
            !token.is_empty() && token == local_token
        } else {
            false
        }
    }

    /// Validate password using bcrypt
    pub async fn validate_password(&self, password: &str) -> bool {
        let config = self.get().await;

        if let Some(ref hashed_password) = config.hashed_password {
            let password = password.to_string();
            let hashed_password = hashed_password.clone();

            // Use spawn_blocking for CPU-intensive bcrypt verification
            match tokio::task::spawn_blocking(move || verify(password, &hashed_password)).await {
                Ok(Ok(is_valid)) => is_valid,
                Ok(Err(e)) => {
                    warn!("Password verification failed: {}", e);
                    false
                }
                Err(e) => {
                    warn!("Task join error during password verification: {}", e);
                    false
                }
            }
        } else {
            false
        }
    }

    /// Hash password using bcrypt
    pub async fn hash_password(&self, password: &str) -> Result<String> {
        let password = password.to_string();

        // Use spawn_blocking for CPU-intensive bcrypt hashing
        let hashed = tokio::task::spawn_blocking(move || hash(password, DEFAULT_COST)).await??;

        Ok(hashed)
    }

    /// Get configuration file path
    pub fn config_path(&self) -> &std::path::Path {
        self.persistence.path()
    }

    /// Check if configuration file exists
    pub fn config_exists(&self) -> bool {
        self.persistence.exists()
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::config::types::{KeyboardMacro, KeyboardMacroStep, WakeOnLanDevice};

    #[tokio::test]
    async fn test_config_manager_lifecycle() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.json");
        let persistence = ConfigPersistence::with_path(&config_path);
        let manager = ConfigManager::with_persistence(persistence);

        // Load initial configuration
        manager.load().await.unwrap();
        let initial_config = manager.get().await;
        assert_eq!(initial_config.local_auth_mode, "noPassword");

        // Update configuration
        manager.set_auth_mode("password".to_string()).await.unwrap();

        // Verify update
        let updated_config = manager.get().await;
        assert_eq!(updated_config.local_auth_mode, "password");

        // Verify persistence
        assert!(manager.config_exists());
    }

    #[tokio::test]
    async fn test_keyboard_macro_management() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.json");
        let persistence = ConfigPersistence::with_path(&config_path);
        let manager = ConfigManager::with_persistence(persistence);

        // Create a test macro
        let mut macro_item = KeyboardMacro {
            id: "test_macro".to_string(),
            name: "Test Macro".to_string(),
            steps: vec![KeyboardMacroStep {
                keys: vec!["a".to_string()],
                modifiers: vec![],
                delay: 100,
            }],
            sort_order: Some(1),
        };

        // Add macro
        manager.set_keyboard_macro(macro_item.clone()).await.unwrap();

        // Verify macro was added
        let config = manager.get().await;
        assert_eq!(config.keyboard_macros.len(), 1);
        assert_eq!(config.keyboard_macros[0].id, "test_macro");

        // Update macro
        macro_item.name = "Updated Macro".to_string();
        manager.set_keyboard_macro(macro_item).await.unwrap();

        // Verify macro was updated
        let config = manager.get().await;
        assert_eq!(config.keyboard_macros.len(), 1);
        assert_eq!(config.keyboard_macros[0].name, "Updated Macro");

        // Remove macro
        manager.remove_keyboard_macro("test_macro").await.unwrap();

        // Verify macro was removed
        let config = manager.get().await;
        assert_eq!(config.keyboard_macros.len(), 0);
    }

    #[tokio::test]
    async fn test_wake_on_lan_device_management() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.json");
        let persistence = ConfigPersistence::with_path(&config_path);
        let manager = ConfigManager::with_persistence(persistence);

        // Create a test device
        let device = WakeOnLanDevice {
            name: "Test Device".to_string(),
            mac_address: "00:11:22:33:44:55".to_string(),
        };

        // Add device
        manager.set_wake_on_lan_device(device.clone()).await.unwrap();

        // Verify device was added
        let config = manager.get().await;
        assert_eq!(config.wake_on_lan_devices.len(), 1);
        assert_eq!(config.wake_on_lan_devices[0].name, "Test Device");

        // Remove device
        manager.remove_wake_on_lan_device("Test Device").await.unwrap();

        // Verify device was removed
        let config = manager.get().await;
        assert_eq!(config.wake_on_lan_devices.len(), 0);
    }

    #[tokio::test]
    async fn test_password_hashing_and_validation() {
        let temp_dir = tempdir().unwrap();
        let config_path = temp_dir.path().join("test_config.json");
        let persistence = ConfigPersistence::with_path(&config_path);
        let manager = ConfigManager::with_persistence(persistence);
        let password = "test_password";

        // Hash password
        let hashed = manager.hash_password(password).await.unwrap();
        assert_ne!(hashed, password);

        // Set hashed password
        manager.set_hashed_password(Some(hashed)).await.unwrap();

        // Validate correct password
        assert!(manager.validate_password(password).await);

        // Validate incorrect password
        assert!(!manager.validate_password("wrong_password").await);
    }
}
