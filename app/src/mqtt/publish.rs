use std::sync::atomic::Ordering;
use std::time::Duration;

use serde_json as json;
use tracing::{error, warn};

use super::manager::MqttManager;
use super::stubs::{
    get_active_sessions, get_available_images, get_installed_version, get_network_state,
    get_ota_progress, get_system_state, get_update_status, get_usb_state, get_video_state,
    get_virtual_media_mount_info, is_cloud_connected, is_jiggler_enabled, is_ota_updating,
};
use super::types::{
    AtxState, DcPowerState, HaDiscoveryPayload, MqttCloudState, MqttJigglerState, MqttNetworkState,
    MqttSessionsState, MqttUpdateState, MqttUsbState, MqttVirtualMediaState, PUBLISH_TIMEOUT,
    UPDATE_CHECK_INTERVAL,
};

impl MqttManager {
    pub async fn publish_extended_states(&self) {
        self.publish_video_state().await;
        self.publish_usb_state().await;
        self.publish_cloud_state().await;
        self.publish_sessions_state().await;
        self.publish_jiggler_state().await;
        self.publish_network_state().await;
        self.publish_system_state().await;
        self.publish_virtual_media_state().await;
        self.publish_update_state().await;
    }

    pub async fn publish_video_state(&self) {
        if !self.is_connected() {
            return;
        }
        let state = get_video_state().await;
        self.publish_json(&self.topic(&["video", "state"]), &state, true).await;
    }

    pub async fn publish_usb_state(&self) {
        if !self.is_connected() {
            return;
        }
        let state = MqttUsbState { state: get_usb_state() };
        self.publish_json(&self.topic(&["usb", "state"]), &state, true).await;
    }

    pub async fn publish_cloud_state(&self) {
        if !self.is_connected() {
            return;
        }
        let state = MqttCloudState { connected: is_cloud_connected() };
        self.publish_json(&self.topic(&["cloud", "state"]), &state, true).await;
    }

    pub async fn publish_sessions_state(&self) {
        if !self.is_connected() {
            return;
        }
        let state = MqttSessionsState { active_sessions: get_active_sessions().await };
        self.publish_json(&self.topic(&["sessions", "state"]), &state, true).await;
    }

    pub async fn publish_jiggler_state(&self) {
        if !self.is_connected() {
            return;
        }
        let state = MqttJigglerState { enabled: is_jiggler_enabled().await };
        self.publish_json(&self.topic(&["jiggler", "state"]), &state, true).await;
    }

    pub async fn publish_network_state(&self) {
        if !self.is_connected() {
            return;
        }
        let net = get_network_state().await;
        let state = MqttNetworkState { ip_address: net.0, hostname: net.1 };
        self.publish_json(&self.topic(&["network", "state"]), &state, true).await;
    }

    pub async fn publish_system_state(&self) {
        if !self.is_connected() {
            return;
        }
        let state = get_system_state().await;
        self.publish_json(&self.topic(&["system", "state"]), &state, true).await;
    }

    pub async fn publish_virtual_media_state(&self) {
        if !self.is_connected() {
            return;
        }

        let (mounted_image, source) = get_virtual_media_mount_info();
        let state = MqttVirtualMediaState { mounted_image, source };

        self.publish_json(&self.topic(&["virtual_media", "state"]), &state, true).await;

        if self.enable_ha_discovery && self.enable_actions {
            let mut vm_options = get_available_images().await;
            if state.source == "url" {
                vm_options.push(state.mounted_image.clone());
            }

            let mut last_options = self.last_vm_options.lock().await;
            if *last_options != vm_options {
                *last_options = vm_options.clone();
                drop(last_options);

                let device = self.ha_device_info();
                self.publish_discovery(
                    "select",
                    "virtual_media",
                    HaDiscoveryPayload {
                        name: "Virtual Media".to_string(),
                        unique_id: format!("rustkvm_{}_virtual_media", self.device_id),
                        state_topic: Some(self.topic(&["virtual_media", "state"])),
                        command_topic: Some(self.topic(&["virtual_media", "set"])),
                        value_template: Some("{{ value_json.mounted_image }}".to_string()),
                        options: Some(vm_options),
                        icon: Some("mdi:disc".to_string()),
                        json_attributes_topic: Some(self.topic(&["virtual_media", "state"])),
                        json_attributes_template: Some(
                            "{{ {'source': value_json.source} | tojson }}".to_string(),
                        ),
                        availability_topic: self.topic(&["status"]),
                        availability_template: "{{ 'online' if value_json.online else 'offline' }}"
                            .to_string(),
                        device: Some(device),
                        ..Default::default()
                    },
                )
                .await;
            }
        }
    }

    pub async fn publish_update_state(&self) {
        if !self.is_connected() {
            return;
        }

        let ota_updating = is_ota_updating();
        let update_requested = self.update_requested.load(Ordering::Relaxed);

        if update_requested && !ota_updating {
            self.update_requested.store(false, Ordering::Relaxed);
        }

        let updating = ota_updating || self.update_requested.load(Ordering::Relaxed);

        let mut update_payload = MqttUpdateState {
            installed_version: get_installed_version(),
            latest_version: get_installed_version(),
            in_progress: false,
            update_percentage: None,
        };

        if updating {
            let latest = self.last_known_latest_version.lock().await;
            if !latest.is_empty() {
                update_payload.latest_version = latest.clone();
            }
            update_payload.in_progress = true;
            update_payload.update_percentage = Some(get_ota_progress());

            *self.last_update_check.lock().await = None;
        } else {
            let now = std::time::Instant::now();
            let mut last_check = self.last_update_check.lock().await;

            let cached = if let Some(ref last) = *last_check {
                if now.duration_since(*last) < UPDATE_CHECK_INTERVAL {
                    self.last_update_payload.lock().await.clone()
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(cached) = cached {
                update_payload = cached;
                update_payload.installed_version = get_installed_version();
            } else {
                if let Some(status) = get_update_status() {
                    update_payload.installed_version = status.installed_version;
                    if status.update_available {
                        update_payload.latest_version = status.latest_version.clone();
                        *self.last_known_latest_version.lock().await = status.latest_version;
                    }
                }
                update_payload.in_progress = false;
                update_payload.update_percentage = None;

                *self.last_update_payload.lock().await = Some(update_payload.clone());
                *last_check = Some(now);
            }
        }

        self.publish_json(&self.topic(&["update", "state"]), &update_payload, true).await;
    }

    pub async fn publish_dc_state(&self, state: &DcPowerState) {
        if !self.is_connected() {
            return;
        }
        self.publish_json(&self.topic(&["dc", "state"]), state, true).await;
    }

    pub async fn publish_atx_state(&self, state: &AtxState) {
        if !self.is_connected() {
            return;
        }

        if self.debounce_ms == 0 {
            self.publish_json(&self.topic(&["atx", "state"]), state, true).await;
            return;
        }

        let state = *state;
        let mut mu = self.atx_debounce_mu.lock().await;
        let last = *self.atx_last_published.lock().await;

        let Some(last) = last else {
            self.publish_atx_state_locked(&state).await;
            return;
        };

        if state.power != last.power {
            mu.timer_active = false;
            mu.pending_state = None;
            self.publish_atx_state_locked(&state).await;
            return;
        }

        if state.hdd {
            if mu.timer_active {
                mu.timer_active = false;
                mu.pending_state = None;
            }
            if !last.hdd {
                self.publish_atx_state_locked(&state).await;
            }
            return;
        }

        if !last.hdd {
            return;
        }

        if mu.timer_active {
            return;
        }

        mu.timer_active = true;
        mu.pending_state = Some(state);
        drop(mu);

        let debounce_ms = self.debounce_ms;

        let topic = self.topic(&["atx", "state"]);
        let client = self.client.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(debounce_ms)).await;
            let data = json::to_string(&state).unwrap_or_default();
            match tokio::time::timeout(
                PUBLISH_TIMEOUT,
                client.publish(&topic, rumqttc::QoS::AtLeastOnce, true, data.into_bytes()),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    error!("Failed to publish ATX state (debounced): {}", e);
                }
                Err(_) => {
                    warn!("MQTT publish timed out for ATX state (debounced)");
                }
            }
        });
    }

    async fn publish_atx_state_locked(&self, state: &AtxState) {
        *self.atx_last_published.lock().await = Some(*state);
        self.publish_json(&self.topic(&["atx", "state"]), state, true).await;
    }
}
