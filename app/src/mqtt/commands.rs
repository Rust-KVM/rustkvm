use std::sync::atomic::Ordering;
use std::time::Duration;

use tracing::{debug, error, info, warn};

use super::manager::MqttManager;
use super::stubs::{
    get_installed_version, is_virtual_media_mounted, mount_with_storage, press_atx_power_button,
    press_atx_reset_button, reboot_device, set_dc_power_state, set_dc_restore_state,
    set_jiggler_state, trigger_ota_update, unmount_image,
};
use super::types::MqttUpdateState;

impl MqttManager {
    pub(super) async fn subscribe_commands(&self) {
        let commands: &[(&str, rumqttc::QoS)] = &[
            (&self.topic(&["dc_power", "set"]), rumqttc::QoS::AtLeastOnce),
            (&self.topic(&["dc_restore", "set"]), rumqttc::QoS::AtLeastOnce),
            (&self.topic(&["atx_power_short", "set"]), rumqttc::QoS::AtLeastOnce),
            (&self.topic(&["atx_power_long", "set"]), rumqttc::QoS::AtLeastOnce),
            (&self.topic(&["atx_reset", "set"]), rumqttc::QoS::AtLeastOnce),
            (&self.topic(&["jiggler", "set"]), rumqttc::QoS::AtLeastOnce),
            (&self.topic(&["reboot", "set"]), rumqttc::QoS::AtLeastOnce),
            (&self.topic(&["update", "install"]), rumqttc::QoS::AtLeastOnce),
            (&self.topic(&["virtual_media", "set"]), rumqttc::QoS::AtLeastOnce),
        ];

        for (topic, _qos) in commands {
            match self.client.subscribe(&**topic, rumqttc::QoS::AtLeastOnce).await {
                Ok(()) => {}
                Err(e) => {
                    error!("Failed to subscribe to topic {}: {}", topic, e);
                }
            }
        }

        info!("subscribed to command topics");
    }

    #[tracing::instrument(skip(self, payload), fields(topic))]
    pub(super) async fn handle_command(&self, topic: &str, payload: &str) {
        let payload = payload.trim();
        let base = self.base_topic.as_str();

        let cmd = topic.strip_prefix(base).and_then(|s| s.strip_prefix('/')).unwrap_or(topic);

        match cmd {
            "dc_power/set" => self.handle_dc_power_command(payload).await,
            "dc_restore/set" => self.handle_dc_restore_command(payload).await,
            "atx_power_short/set" => self.handle_atx_power_short_command().await,
            "atx_power_long/set" => self.handle_atx_power_long_command().await,
            "atx_reset/set" => self.handle_atx_reset_command().await,
            "jiggler/set" => self.handle_jiggler_command(payload).await,
            "reboot/set" => self.handle_reboot_command().await,
            "update/install" => self.handle_update_install_command().await,
            "virtual_media/set" => self.handle_virtual_media_command(payload).await,
            _ => {
                debug!("Unknown MQTT command topic: {}", topic);
            }
        }
    }

    async fn handle_dc_power_command(&self, payload: &str) {
        if !self.actions_allowed() {
            warn!("DC power command rejected: actions are disabled");
            return;
        }
        info!(payload, "received DC power command");

        match payload.to_uppercase().as_str() {
            "ON" => {
                if let Err(e) = set_dc_power_state(true).await {
                    error!("failed to set DC power on: {}", e);
                }
            }
            "OFF" => {
                if let Err(e) = set_dc_power_state(false).await {
                    error!("failed to set DC power off: {}", e);
                }
            }
            _ => {
                warn!(payload, "unknown DC power command");
            }
        }
    }

    async fn handle_dc_restore_command(&self, payload: &str) {
        if !self.actions_allowed() {
            warn!("DC restore command rejected: actions are disabled");
            return;
        }
        info!(payload, "received DC restore command");

        let state = match payload.to_lowercase().as_str() {
            "off" => 0,
            "on" => 1,
            "last_state" => 2,
            _ => {
                warn!(payload, "unknown DC restore command");
                return;
            }
        };

        if let Err(e) = set_dc_restore_state(state).await {
            error!("failed to set DC restore state: {}", e);
        }
    }

    async fn handle_atx_power_short_command(&self) {
        if !self.actions_allowed() {
            warn!("ATX power short command rejected: actions are disabled");
            return;
        }
        info!("received ATX power short press command");
        if let Err(e) = press_atx_power_button(Duration::from_millis(200)).await {
            error!("failed to press ATX power button (short): {}", e);
        }
    }

    async fn handle_atx_power_long_command(&self) {
        if !self.actions_allowed() {
            warn!("ATX power long command rejected: actions are disabled");
            return;
        }
        info!("received ATX power long press command");
        if let Err(e) = press_atx_power_button(Duration::from_secs(5)).await {
            error!("failed to press ATX power button (long): {}", e);
        }
    }

    async fn handle_atx_reset_command(&self) {
        if !self.actions_allowed() {
            warn!("ATX reset command rejected: actions are disabled");
            return;
        }
        info!("received ATX reset command");
        if let Err(e) = press_atx_reset_button(Duration::from_millis(500)).await {
            error!("failed to press ATX reset button: {}", e);
        }
    }

    async fn handle_jiggler_command(&self, payload: &str) {
        if !self.actions_allowed() {
            warn!("jiggler command rejected: actions are disabled");
            return;
        }
        info!(payload, "received jiggler command");

        match payload.to_uppercase().as_str() {
            "ON" => {
                if let Err(e) = set_jiggler_state(true) {
                    error!("failed to enable jiggler: {}", e);
                }
            }
            "OFF" => {
                if let Err(e) = set_jiggler_state(false) {
                    error!("failed to disable jiggler: {}", e);
                }
            }
            _ => {
                warn!(payload, "unknown jiggler command");
            }
        }

        self.publish_jiggler_state().await;
    }

    async fn handle_reboot_command(&self) {
        if !self.actions_allowed() {
            warn!("reboot command rejected: actions are disabled");
            return;
        }
        info!("received reboot command via MQTT");
        if let Err(e) = reboot_device() {
            error!("failed to reboot: {}", e);
        }
    }

    async fn handle_update_install_command(&self) {
        if !self.actions_allowed() {
            warn!("update install command rejected: actions are disabled");
            return;
        }
        info!("received update install command via MQTT");

        self.update_requested.store(true, Ordering::Relaxed);

        let latest_ver = {
            let guard = self.last_known_latest_version.lock().await;
            let lv = guard.clone();
            if lv.is_empty() { get_installed_version() } else { lv }
        };

        self.publish_json(
            &self.topic(&["update", "state"]),
            &MqttUpdateState {
                installed_version: get_installed_version(),
                latest_version: latest_ver,
                in_progress: true,
                update_percentage: Some(0.0),
            },
            true,
        )
        .await;

        if let Err(e) = trigger_ota_update() {
            error!("failed to start update: {}", e);
            self.update_requested.store(false, Ordering::Relaxed);
            self.publish_update_state().await;
        }
    }

    async fn handle_virtual_media_command(&self, payload: &str) {
        if !self.actions_allowed() {
            warn!("virtual media command rejected: actions are disabled");
            return;
        }
        info!(payload, "received virtual media command");

        if payload == "-- no media --" {
            if let Err(e) = unmount_image() {
                error!("failed to unmount image: {}", e);
            }
        } else {
            if payload.contains("..") || payload.contains('/') || payload.contains('\\') {
                warn!(payload, "rejected invalid filename");
                return;
            }

            if is_virtual_media_mounted()
                && let Err(e) = unmount_image()
            {
                error!("failed to unmount current image before mounting new one: {}", e);
                return;
            }

            if let Err(e) = mount_with_storage(payload) {
                error!("failed to mount image: {}", e);
            }
        }

        self.publish_virtual_media_state().await;
    }
}
