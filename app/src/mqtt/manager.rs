use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json as json;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use super::MQTT_MANAGER;
use super::types::{
    AtxState, MqttConfig, MqttStatusPayload, MqttUpdateState, PUBLISH_TIMEOUT, build_topic,
};

pub(crate) struct MqttManager {
    pub(super) client: rumqttc::AsyncClient,
    pub(super) device_id: String,
    pub(super) base_topic: String,
    pub(super) connected: AtomicBool,
    pub(super) last_error: Mutex<String>,
    pub(super) update_requested: AtomicBool,
    pub(super) debounce_ms: u64,
    pub(super) enable_ha_discovery: bool,
    pub(super) enable_actions: bool,

    pub(super) shutdown: Arc<tokio::sync::Notify>,

    pub(super) atx_debounce_mu: Mutex<AtxDebounceState>,
    pub(super) atx_last_published: Mutex<Option<AtxState>>,

    pub(super) last_vm_options: Mutex<Vec<String>>,

    pub(super) last_update_check: Mutex<Option<std::time::Instant>>,
    pub(super) last_update_payload: Mutex<Option<MqttUpdateState>>,
    pub(super) last_known_latest_version: Mutex<String>,
}

#[derive(Default)]
pub(super) struct AtxDebounceState {
    pub(super) timer_active: bool,
    pub(super) pending_state: Option<AtxState>,
}

impl MqttManager {
    pub async fn new(cfg: &MqttConfig, device_id: &str) -> Result<Self> {
        if !cfg.enabled {
            return Err(anyhow!("MQTT is not enabled"));
        }

        let mut base_topic =
            if cfg.base_topic.is_empty() { "rustkvm".to_string() } else { cfg.base_topic.clone() };

        if base_topic.contains('+') || base_topic.contains('#') {
            return Err(anyhow!("base topic must not contain MQTT wildcards (+ or #)"));
        }
        if base_topic.contains(' ') {
            return Err(anyhow!("base topic must not contain spaces"));
        }

        if !base_topic.contains(device_id) {
            use std::fmt::Write;
            write!(&mut base_topic, "/{}", device_id).ok();
        }

        let port = if cfg.port == 0 { 1883 } else { cfg.port };
        let client_id = format!("rustkvm-{}", device_id);

        let mut mqtt_options = rumqttc::MqttOptions::new(&client_id, &cfg.broker, port);
        mqtt_options.set_keep_alive(Duration::from_secs(30));
        mqtt_options.set_clean_session(false);
        mqtt_options.set_max_packet_size(256 * 1024, 256 * 1024);

        let will_topic = build_topic(&base_topic, &["status"]);
        let will_payload = json::to_string(&MqttStatusPayload { online: false })?;
        mqtt_options.set_last_will(rumqttc::LastWill::new(
            &will_topic,
            will_payload.into_bytes(),
            rumqttc::QoS::AtLeastOnce,
            true,
        ));

        if !cfg.username.is_empty() {
            mqtt_options.set_credentials(&cfg.username, &cfg.password);
        }

        let (client, eventloop) = rumqttc::AsyncClient::new(mqtt_options, 100);

        let enable_ha_discovery = cfg.enable_ha_discovery;
        let enable_actions = cfg.enable_actions;
        let debounce_ms = cfg.debounce_ms;

        let manager = Self {
            client,
            device_id: device_id.to_string(),
            base_topic,
            connected: AtomicBool::new(false),
            last_error: Mutex::new(String::new()),
            update_requested: AtomicBool::new(false),
            debounce_ms,
            enable_ha_discovery,
            enable_actions,
            shutdown: Arc::new(tokio::sync::Notify::new()),
            atx_debounce_mu: Mutex::new(AtxDebounceState::default()),
            atx_last_published: Mutex::new(None),
            last_vm_options: Mutex::new(Vec::new()),
            last_update_check: Mutex::new(None),
            last_update_payload: Mutex::new(None),
            last_known_latest_version: Mutex::new(String::new()),
        };

        let shutdown = Arc::clone(&manager.shutdown);
        tokio::spawn(async move {
            run_eventloop(eventloop, shutdown).await;
        });

        Ok(manager)
    }

    pub(super) fn topic(&self, parts: &[&str]) -> String {
        build_topic(&self.base_topic, parts)
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    pub async fn last_error(&self) -> String {
        self.last_error.lock().await.clone()
    }

    pub(super) fn actions_allowed(&self) -> bool {
        self.enable_actions
    }

    pub(super) async fn publish_json<T: Serialize>(
        &self,
        topic: &str,
        payload: &T,
        retained: bool,
    ) {
        match json::to_string(payload) {
            Ok(data) => {
                self.publish_raw(topic, retained, data.into_bytes()).await;
            }
            Err(e) => {
                error!("Failed to marshal MQTT payload for topic {}: {}", topic, e);
            }
        }
    }

    pub(super) async fn publish_raw(&self, topic: &str, retained: bool, payload: Vec<u8>) {
        match tokio::time::timeout(
            PUBLISH_TIMEOUT,
            self.client.publish(topic, rumqttc::QoS::AtLeastOnce, retained, payload),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                error!("Failed to publish MQTT message to {}: {}", topic, e);
            }
            Err(_elapsed) => {
                warn!("MQTT publish timed out for topic {}", topic);
            }
        }
    }

    pub(super) async fn publish_string(&self, topic: &str, retained: bool, payload: &str) {
        self.publish_raw(topic, retained, payload.as_bytes().to_vec()).await;
    }

    pub(super) async fn on_connect(&self) {
        info!(
            device_id = %self.device_id,
            "connected to MQTT broker"
        );
        self.connected.store(true, Ordering::Relaxed);
        *self.last_error.lock().await = String::new();

        self.publish_json(&self.topic(&["status"]), &MqttStatusPayload { online: true }, true)
            .await;

        if self.enable_ha_discovery {
            self.publish_ha_discovery().await;
        }

        self.subscribe_commands().await;

        self.publish_extended_states().await;
    }

    pub(super) async fn on_connection_lost(&self, err: &str) {
        warn!("MQTT connection lost: {}", err);
        self.connected.store(false, Ordering::Relaxed);
        *self.last_error.lock().await = err.to_string();
    }

    pub async fn close(&self) {
        self.shutdown.notify_one();

        let mut last_error = self.last_error.lock().await;
        *last_error = String::new();
        drop(last_error);

        self.publish_json(&self.topic(&["status"]), &MqttStatusPayload { online: false }, true)
            .await;

        let _ = self.client.disconnect().await;

        self.connected.store(false, Ordering::Relaxed);
    }
}

pub(super) async fn run_eventloop(
    mut eventloop: rumqttc::EventLoop,
    shutdown: Arc<tokio::sync::Notify>,
) {
    loop {
        tokio::select! {
            _ = shutdown.notified() => {
                info!("MQTT event loop shutting down");
                break;
            }
            event = eventloop.poll() => {
                match event {
                    Ok(rumqttc::Event::Incoming(rumqttc::Incoming::ConnAck(_))) => {
                        if let Some(guard) = MQTT_MANAGER.get() {
                            let mgr = guard.lock().await;
                            if let Some(mgr) = mgr.as_ref() {
                                mgr.on_connect().await;
                            }
                        }
                    }
                    Ok(rumqttc::Event::Incoming(rumqttc::Incoming::Publish(publish))) => {
                        let topic = publish.topic.clone();
                        let payload = String::from_utf8_lossy(&publish.payload).to_string();

                        if let Some(guard) = MQTT_MANAGER.get() {
                            let mgr = guard.lock().await;
                            if let Some(mgr) = mgr.as_ref() {
                                mgr.handle_command(&topic, &payload).await;
                            }
                        }
                    }
                    Ok(rumqttc::Event::Incoming(rumqttc::Incoming::Disconnect)) => {
                        if let Some(guard) = MQTT_MANAGER.get() {
                            let mgr = guard.lock().await;
                            if let Some(mgr) = mgr.as_ref() {
                                mgr.connected.store(false, Ordering::Relaxed);
                            }
                        }
                        warn!("MQTT broker disconnected");
                    }
                    Ok(_) => {
                    }
                    Err(e) => {
                        error!("MQTT event loop error: {}", e);
                        if let Some(guard) = MQTT_MANAGER.get() {
                            let mgr = guard.lock().await;
                            if let Some(mgr) = mgr.as_ref() {
                                mgr.on_connection_lost(&e.to_string()).await;
                            }
                        }
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
        }
    }
}
