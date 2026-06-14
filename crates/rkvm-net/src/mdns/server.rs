use std::sync::Arc;

use mdns_sd::{ServiceDaemon, ServiceInfo};
use tokio::sync::RwLock;
use tracing::info;

use crate::error::{Error, Result};
use crate::mdns::config::{MdnsListenOptions, MdnsOptions};

pub struct Mdns {
    service: Arc<RwLock<Option<ServiceDaemon>>>,
    local_names: Arc<RwLock<Vec<String>>>,
    listen_options: Arc<RwLock<MdnsListenOptions>>,
    service_name: String,
    service_type: String,
    port: u16,
}

impl Mdns {
    const DEFAULT_SERVICE_NAME: &'static str = "rustkvm";
    const DEFAULT_SERVICE_TYPE: &'static str = "_http._tcp";
    const DEFAULT_PORT: u16 = 80;

    pub const DEFAULT_ADDRESS_IPV4: &'static str = "224.0.0.251:5353";
    pub const DEFAULT_ADDRESS_IPV6: &'static str = "[ff02::fb]:5353";

    pub fn new(options: MdnsOptions) -> Result<Self> {
        let service_name = Self::DEFAULT_SERVICE_NAME.to_string();
        let service_type = Self::DEFAULT_SERVICE_TYPE.to_string();
        let port = Self::DEFAULT_PORT;

        Ok(Self {
            service: Arc::new(RwLock::new(None)),
            local_names: Arc::new(RwLock::new(options.local_names)),
            listen_options: Arc::new(RwLock::new(options.listen_options)),
            service_name,
            service_type,
            port,
        })
    }

    pub async fn start(&self) -> Result<()> {
        self.start_internal(false).await
    }

    async fn start_internal(&self, allow_restart: bool) -> Result<()> {
        let mut service_guard = self.service.write().await;

        if service_guard.is_some() {
            if !allow_restart {
                return Err(Error::MdnsAlreadyRunning);
            }
            self.stop_internal().await?;
        }

        let listen_options = self.listen_options.read().await;

        if !listen_options.ipv4 && !listen_options.ipv6 {
            info!("mDNS server disabled");
            return Ok(());
        }

        let daemon = ServiceDaemon::new()?;

        let local_names = self.local_names.read().await;
        let mut service_infos = Vec::new();

        for name in local_names.iter() {
            let hostname = self.normalize_hostname(name);
            let properties: std::collections::HashMap<String, String> =
                [("hostname", hostname.as_str())]
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect();

            let service_info = ServiceInfo::new(
                &self.service_type,
                &self.service_name,
                &hostname,
                "",
                self.port,
                properties,
            )?;

            service_infos.push(service_info);
        }

        for service_info in service_infos {
            daemon.register(service_info)?;
        }

        *service_guard = Some(daemon);

        info!(
            local_names = ?*local_names,
            ipv4 = listen_options.ipv4,
            ipv6 = listen_options.ipv6,
            "mDNS server started"
        );

        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        self.stop_internal().await
    }

    async fn stop_internal(&self) -> Result<()> {
        let mut service_guard = self.service.write().await;

        if let Some(daemon) = service_guard.take() {
            daemon.shutdown()?;
            info!("mDNS server stopped");
        }

        Ok(())
    }

    fn normalize_hostname(&self, name: &str) -> String {
        let mut hostname = name.trim_end_matches('.').to_lowercase();
        if !hostname.ends_with(".local") {
            hostname.push_str(".local");
        }
        hostname
    }
}

impl Drop for Mdns {
    fn drop(&mut self) {
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            let service = self.service.clone();
            rt.spawn(async move {
                let mut service_guard = service.write().await;
                if let Some(daemon) = service_guard.take() {
                    let _ = daemon.shutdown();
                }
            });
        }
    }
}
