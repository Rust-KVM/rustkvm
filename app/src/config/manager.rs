use std::sync::Arc;

use anyhow::{Result, bail};
use bcrypt::{DEFAULT_COST, hash, verify};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::config::persistence::ConfigPersistence;
use crate::config::types::Config;

#[derive(Debug)]
pub struct ConfigManager {
    config: Arc<RwLock<Config>>,
    persistence: ConfigPersistence,
}

impl ConfigManager {
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(Config::default())),
            persistence: ConfigPersistence::new(),
        }
    }

    pub async fn load(&self) -> Result<()> {
        let loaded_config = self.persistence.load().await?;
        *self.config.write().await = loaded_config;
        info!("Configuration loaded successfully");
        Ok(())
    }

    pub async fn save(&self) -> Result<()> {
        let config = self.config.read().await;
        self.persistence.save(&config).await?;
        info!("Configuration saved successfully");
        Ok(())
    }

    pub async fn get(&self) -> Config {
        self.config.read().await.clone()
    }

    pub async fn update<F>(&self, updater: F) -> Result<()>
    where
        F: FnOnce(&mut Config),
    {
        {
            let mut config = self.config.write().await;
            updater(&mut config);

            if let Err(errors) = config.validate() {
                warn!("Configuration validation failed: {:?}", errors);
                bail!("Configuration validation failed: {:?}", errors);
            }
        }

        self.save().await?;
        Ok(())
    }

    pub async fn set_auth_mode(&self, mode: String) -> Result<()> {
        self.update(|config| {
            config.local_auth_mode = mode;
        })
        .await
    }

    pub async fn set_hashed_password(&self, hashed_password: Option<String>) -> Result<()> {
        self.update(|config| {
            config.hashed_password = hashed_password;
        })
        .await
    }

    pub async fn set_auth_token(&self, token: Option<String>) -> Result<()> {
        self.update(|config| {
            config.local_auth_token = token;
        })
        .await
    }

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

    pub async fn is_setup(&self) -> bool {
        let config = self.get().await;
        !config.is_setup_required()
    }

    pub async fn validate_auth_token(&self, token: &str) -> bool {
        let config = self.get().await;

        if config.local_auth_mode == "noPassword" {
            return true;
        }

        if let Some(ref local_token) = config.local_auth_token {
            !token.is_empty() && token == local_token
        } else {
            false
        }
    }

    pub async fn validate_password(&self, password: &str) -> bool {
        let config = self.get().await;

        if let Some(ref hashed_password) = config.hashed_password {
            let password = password.to_string();
            let hashed_password = hashed_password.clone();

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

    pub async fn hash_password(&self, password: &str) -> Result<String> {
        let password = password.to_string();

        let hashed = tokio::task::spawn_blocking(move || hash(password, DEFAULT_COST)).await??;

        Ok(hashed)
    }

    pub fn config_path(&self) -> &std::path::Path {
        self.persistence.path()
    }

    pub fn config_exists(&self) -> bool {
        self.persistence.exists()
    }
}

impl Default for ConfigManager {
    fn default() -> Self {
        Self::new()
    }
}
