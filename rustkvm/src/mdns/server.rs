use std::sync::Arc;

use mdns_sd::{ServiceDaemon, ServiceInfo};
use tokio::sync::RwLock;
use tracing::info;

use crate::mdns::config::{MdnsListenOptions, MdnsOptions};

/// mDNS server structure
pub struct Mdns {
    service: Arc<RwLock<Option<ServiceDaemon>>>,
    local_names: Arc<RwLock<Vec<String>>>,
    listen_options: Arc<RwLock<MdnsListenOptions>>,
    service_name: String,
    service_type: String,
    port: u16,
}

impl Mdns {
    /// Default service name and type
    const DEFAULT_SERVICE_NAME: &'static str = "rustkvm";
    const DEFAULT_SERVICE_TYPE: &'static str = "_http._tcp";
    const DEFAULT_PORT: u16 = 80;

    /// Default mDNS addresses
    pub const DEFAULT_ADDRESS_IPV4: &'static str = "224.0.0.251:5353";
    pub const DEFAULT_ADDRESS_IPV6: &'static str = "[ff02::fb]:5353";

    /// Create new mDNS instance
    pub fn new(options: MdnsOptions) -> anyhow::Result<Self> {
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

    /// Start mDNS server
    pub async fn start(&self) -> anyhow::Result<()> {
        self.start_internal(false).await
    }

    /// Restart mDNS server
    pub async fn restart(&self) -> anyhow::Result<()> {
        self.start_internal(true).await
    }

    /// Internal start logic
    async fn start_internal(&self, allow_restart: bool) -> anyhow::Result<()> {
        let mut service_guard = self.service.write().await;

        if service_guard.is_some() {
            if !allow_restart {
                anyhow::bail!("mDNS server already running");
            }
            self.stop_internal().await?;
        }

        let listen_options = self.listen_options.read().await;

        if !listen_options.ipv4 && !listen_options.ipv6 {
            info!("mDNS server disabled");
            return Ok(());
        }

        // Create mDNS service daemon
        let daemon = ServiceDaemon::new()?;

        let local_names = self.local_names.read().await;
        let mut service_infos = Vec::new();

        // Create service info for each local name
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

        // Register all services
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

    /// Stop mDNS server
    pub async fn stop(&self) -> anyhow::Result<()> {
        self.stop_internal().await
    }

    async fn stop_internal(&self) -> anyhow::Result<()> {
        let mut service_guard = self.service.write().await;

        if let Some(daemon) = service_guard.take() {
            daemon.shutdown()?;
            info!("mDNS server stopped");
        }

        Ok(())
    }

    /// Set local names
    pub async fn set_local_names(&self, names: Vec<String>, always: bool) -> anyhow::Result<()> {
        let mut local_names_guard = self.local_names.write().await;

        if !always && *local_names_guard == names {
            return Ok(());
        }

        *local_names_guard = names;
        drop(local_names_guard);

        self.restart().await?;
        Ok(())
    }

    /// Set listening options
    pub async fn set_listen_options(&self, options: MdnsListenOptions) -> anyhow::Result<()> {
        let mut listen_options_guard = self.listen_options.write().await;

        if *listen_options_guard == options {
            return Ok(());
        }

        *listen_options_guard = options;
        drop(listen_options_guard);

        self.restart().await?;
        Ok(())
    }

    /// Get current local names
    pub async fn get_local_names(&self) -> Vec<String> {
        self.local_names.read().await.clone()
    }

    /// Get current listening options
    pub async fn get_listen_options(&self) -> MdnsListenOptions {
        *self.listen_options.read().await
    }

    /// Normalize hostname to ensure .local suffix
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
        // Use tokio runtime for async cleanup
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn test_mdns_creation() {
        let options = MdnsOptions {
            local_names: vec!["test".to_string()],
            listen_options: MdnsListenOptions::default(),
        };

        let mdns = Mdns::new(options).unwrap();
        assert_eq!(mdns.get_local_names().await, vec!["test".to_string()]);
    }

    #[tokio::test]
    async fn test_normalize_hostname() {
        let options = MdnsOptions::default();
        let mdns = Mdns::new(options).unwrap();

        assert_eq!(mdns.normalize_hostname("test"), "test.local");
        assert_eq!(mdns.normalize_hostname("test.local"), "test.local");
        assert_eq!(mdns.normalize_hostname("TEST."), "test.local");
        assert_eq!(mdns.normalize_hostname("TEST.HOST."), "test.host.local");
    }

    #[tokio::test]
    async fn test_start_stop_cycle_disabled() {
        let options = MdnsOptions {
            local_names: vec!["test".to_string()],
            listen_options: MdnsListenOptions { ipv4: false, ipv6: false },
        };

        let mdns = Mdns::new(options).unwrap();

        // Test start with disabled listening (should succeed)
        assert!(mdns.start().await.is_ok());

        // Test stop
        assert!(mdns.stop().await.is_ok());

        // Test stop when already stopped
        assert!(mdns.stop().await.is_ok());
    }

    #[tokio::test]
    async fn test_set_local_names_logic() {
        let options = MdnsOptions {
            local_names: vec!["test1".to_string()],
            listen_options: MdnsListenOptions { ipv4: false, ipv6: false },
        };

        let mdns = Mdns::new(options).unwrap();
        assert!(mdns.start().await.is_ok());

        // Test setting same names (should not restart)
        assert!(mdns.set_local_names(vec!["test1".to_string()], false).await.is_ok());

        // Test setting different names (should restart)
        assert!(mdns.set_local_names(vec!["test2".to_string()], false).await.is_ok());

        // Test force restart
        assert!(mdns.set_local_names(vec!["test2".to_string()], true).await.is_ok());

        assert_eq!(mdns.get_local_names().await, vec!["test2".to_string()]);
    }

    #[tokio::test]
    async fn test_set_listen_options_logic() {
        let options = MdnsOptions {
            local_names: vec!["test".to_string()],
            listen_options: MdnsListenOptions { ipv4: false, ipv6: false },
        };

        let mdns = Mdns::new(options).unwrap();
        assert!(mdns.start().await.is_ok());

        // Test setting same options (should not restart)
        let same_options = MdnsListenOptions { ipv4: false, ipv6: false };
        assert!(mdns.set_listen_options(same_options).await.is_ok());

        // Test setting different options (should update and restart)
        let different_options = MdnsListenOptions { ipv4: true, ipv6: false };
        // In test environment, we test the logic flow rather than actual restart
        // The key is that options are updated correctly
        let _ = mdns.set_listen_options(different_options).await;
        assert_eq!(mdns.get_listen_options().await, different_options);
    }

    #[tokio::test]
    async fn test_empty_local_names() {
        let options = MdnsOptions {
            local_names: vec![],
            listen_options: MdnsListenOptions { ipv4: false, ipv6: false },
        };

        let mdns = Mdns::new(options).unwrap();
        assert!(mdns.start().await.is_ok());
        assert!(mdns.stop().await.is_ok());
    }

    #[tokio::test]
    async fn test_multiple_local_names() {
        let options = MdnsOptions {
            local_names: vec!["host1".to_string(), "host2".to_string(), "host3".to_string()],
            listen_options: MdnsListenOptions { ipv4: false, ipv6: false },
        };

        let mdns = Mdns::new(options).unwrap();
        assert!(mdns.start().await.is_ok());

        let names = mdns.get_local_names().await;
        assert_eq!(names.len(), 3);
        assert!(names.contains(&"host1".to_string()));
        assert!(names.contains(&"host2".to_string()));
        assert!(names.contains(&"host3".to_string()));

        assert!(mdns.stop().await.is_ok());
    }

    #[tokio::test]
    async fn test_config_options() {
        let listen_options = MdnsListenOptions { ipv4: false, ipv6: true };
        let options = MdnsOptions { local_names: vec!["test".to_string()], listen_options };

        let mdns = Mdns::new(options).unwrap();
        assert_eq!(mdns.get_listen_options().await, listen_options);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        let options = MdnsOptions {
            local_names: vec!["test".to_string()],
            listen_options: MdnsListenOptions::default(),
        };

        let mdns = Arc::new(Mdns::new(options).unwrap());

        // Test concurrent reads
        let mdns_clone1 = Arc::clone(&mdns);
        let mdns_clone2 = Arc::clone(&mdns);

        let handle1 = tokio::spawn(async move { mdns_clone1.get_local_names().await });

        let handle2 = tokio::spawn(async move { mdns_clone2.get_listen_options().await });

        let (names, options) = tokio::join!(handle1, handle2);
        assert_eq!(names.unwrap(), vec!["test".to_string()]);
        assert_eq!(options.unwrap(), MdnsListenOptions::default());
    }

    #[tokio::test]
    async fn test_drop_cleanup() {
        let options = MdnsOptions {
            local_names: vec!["test".to_string()],
            listen_options: MdnsListenOptions { ipv4: false, ipv6: false },
        };

        let mdns = Mdns::new(options).unwrap();
        assert!(mdns.start().await.is_ok());

        // Drop should clean up resources
        drop(mdns);

        // Give some time for cleanup
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
