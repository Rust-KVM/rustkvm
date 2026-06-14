use std::io::ErrorKind;
use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::{Result, anyhow};
use reqwest::Client;
use tracing::{info, warn};

use super::oidc::OidcAuthenticator;
use super::types::{
    CloudConnectionState, CloudRegisterRequest, CloudState, TokenExchangeRequest,
    TokenExchangeResponse,
};
use super::websocket::CloudWebSocketClient;
use crate::config::get_config_manager;

static CLOUD_MANAGER: once_cell::sync::OnceCell<CloudManager> = once_cell::sync::OnceCell::new();

pub fn get_cloud_manager() -> &'static CloudManager {
    CLOUD_MANAGER.get_or_init(CloudManager::new)
}

pub struct CloudManager {
    state: AtomicU8,
}

impl Default for CloudManager {
    fn default() -> Self {
        Self::new()
    }
}

impl CloudManager {
    pub fn new() -> Self {
        Self { state: AtomicU8::new(CloudConnectionState::NotConfigured as u8) }
    }

    pub fn get_state(&self) -> CloudConnectionState {
        CloudConnectionState::from(self.state.load(Ordering::Relaxed))
    }

    pub fn set_state(&self, state: CloudConnectionState) {
        self.state.store(state as u8, Ordering::Relaxed);
    }

    pub async fn get_cloud_state(&self) -> CloudState {
        let config_manager = get_config_manager();
        let config = config_manager.get().await;

        CloudState {
            connected: config.cloud_token.is_some() && !config.cloud_url.is_empty(),
            url: Some(config.cloud_url),
            app_url: Some(config.cloud_app_url),
        }
    }

    pub async fn register_device(&self, req: CloudRegisterRequest) -> Result<()> {
        info!("Starting cloud device registration");

        let config_manager = get_config_manager();
        let cfg = config_manager.get().await;

        let cloud_api = if !cfg.cloud_url.is_empty() {
            cfg.cloud_url.clone()
        } else if !req.cloud_api.is_empty() {
            req.cloud_api.clone()
        } else {
            anyhow::bail!("Cloud URL is not configured");
        };

        let token_resp = self.exchange_temp_token(&req.token, &cloud_api).await?;
        info!("Token exchange successful");

        let oidc_auth = OidcAuthenticator::new().await?;
        let google_identity =
            oidc_auth.verify_token_with_client_id(&req.oidc_google, &req.client_id).await?;
        info!("OIDC token verification successful");

        if cfg.cloud_url.is_empty() {
            config_manager.set_cloud_config(Some(cloud_api), Some(token_resp.secret_token)).await?;
        } else {
            config_manager
                .set_cloud_config(Some(cfg.cloud_url), Some(token_resp.secret_token))
                .await?;
        }

        config_manager
            .update(|config| {
                config.google_identity = Some(google_identity);
            })
            .await?;

        info!("Cloud device registration completed successfully");

        self.set_state(CloudConnectionState::Disconnected);

        Ok(())
    }

    pub async fn deregister_device(&self) -> Result<()> {
        let config_manager = get_config_manager();
        let config = config_manager.get().await;

        if config.cloud_token.is_none() || config.cloud_url.is_empty() {
            return Err(anyhow!("Cloud token or URL is not set"));
        }

        let client = Client::new();
        let token = config.cloud_token.as_ref().ok_or_else(|| anyhow!("Cloud token is not set"))?;
        let response = client
            .delete(format!("{}/devices/{}", config.cloud_url, config.device_id))
            .header("Authorization", format!("Bearer {}", token))
            .timeout(super::CLOUD_API_REQUEST_TIMEOUT)
            .send()
            .await?;

        if response.status().is_success() || response.status().as_u16() == 404 {
            config_manager.set_cloud_config(None, None).await?;
            config_manager
                .update(|config| {
                    config.google_identity = None;
                })
                .await?;

            info!("Device deregistered, disconnecting from cloud");
            self.set_state(CloudConnectionState::NotConfigured);
            Ok(())
        } else {
            Err(anyhow!("Deregister request failed with status: {}", response.status()))
        }
    }

    pub async fn set_cloud_url(&self, api_url: &str, app_url: &str) -> Result<()> {
        let config_manager = get_config_manager();
        let current_config = config_manager.get().await;

        if current_config.cloud_url != api_url {
            info!("Cloud URL changed from {} to {}", current_config.cloud_url, api_url);
            self.set_state(CloudConnectionState::Disconnected);
        }

        config_manager
            .update(|config| {
                config.cloud_url = api_url.to_string();
                config.cloud_app_url = app_url.to_string();
            })
            .await?;

        info!("Cloud URL configuration updated: API={}, App={}", api_url, app_url);
        Ok(())
    }

    pub async fn start_connection_loop(&self) -> Result<()> {
        info!("Starting cloud connection loop");

        loop {
            match self.get_state() {
                CloudConnectionState::NotConfigured => {
                    let config_manager = get_config_manager();
                    let config = config_manager.get().await;

                    if config.cloud_token.is_some() && !config.cloud_url.is_empty() {
                        self.set_state(CloudConnectionState::Disconnected);
                    } else {
                        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    }
                }
                CloudConnectionState::Disconnected => {
                    self.set_state(CloudConnectionState::Connecting);
                    match self.connect_to_cloud().await {
                        Ok(_) => {}
                        Err(e) => {
                            let is_unexpected_eof =
                                if let Some(io_err) = e.downcast_ref::<std::io::Error>() {
                                    io_err.kind() == ErrorKind::UnexpectedEof
                                } else {
                                    false
                                };

                            if is_unexpected_eof {
                                info!(
                                    "Cloud connection closed by peer (UnexpectedEof), treating as normal disconnect"
                                );
                            } else {
                                warn!("Cloud connection failed: {}", e);
                            }

                            self.set_state(CloudConnectionState::Disconnected);

                            let retry_delay = if is_unexpected_eof {
                                tokio::time::Duration::from_secs(1)
                            } else {
                                tokio::time::Duration::from_secs(5)
                            };

                            tokio::time::sleep(retry_delay).await;
                        }
                    }
                }
                CloudConnectionState::Connecting | CloudConnectionState::Connected => {
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn exchange_temp_token(
        &self,
        temp_token: &str,
        cloud_api: &str,
    ) -> Result<TokenExchangeResponse> {
        let client = Client::new();
        let payload = TokenExchangeRequest { temp_token: temp_token.to_string() };

        let response = client
            .post(format!("{}/devices/token", cloud_api))
            .json(&payload)
            .timeout(super::CLOUD_API_REQUEST_TIMEOUT)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!("Token exchange failed: {}", response.status()));
        }

        let token_resp: TokenExchangeResponse = response.json().await?;
        Ok(token_resp)
    }

    async fn connect_to_cloud(&self) -> Result<()> {
        let config_manager = get_config_manager();
        let config = config_manager.get().await;

        let token = config.cloud_token.ok_or_else(|| anyhow!("No cloud token available"))?;
        let device_id = config.device_id.clone();

        info!("Connecting to cloud WebSocket: {}", config.cloud_url);
        let mut client = CloudWebSocketClient::new(config.cloud_url, token, device_id);

        self.set_state(CloudConnectionState::Connected);

        let result = match client.connect().await {
            Ok(_) => {
                info!("Cloud WebSocket loop returned (clean disconnect)");
                Ok(())
            }
            Err(e) => {
                if let Some(io_err) = e.downcast_ref::<std::io::Error>()
                    && io_err.kind() == ErrorKind::UnexpectedEof
                {
                    info!(
                        "WebSocket connection closed by peer without close_notify (UnexpectedEof)"
                    );
                    Ok(())
                } else {
                    Err(e)
                }
            }
        };
        self.set_state(CloudConnectionState::Disconnected);
        result
    }
}
