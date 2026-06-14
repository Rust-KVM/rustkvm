use tracing::info;

use super::manager::MqttManager;
use super::stubs::{
    get_active_extension, get_available_images, get_dc_state, get_network_config_url,
    get_sw_version_string,
};
use super::types::{HaDevice, HaDiscoveryPayload};

impl MqttManager {
    pub(super) fn ha_device_info(&self) -> HaDevice {
        let device_id = &self.device_id;

        let config_url = get_network_config_url();

        let sw_version = get_sw_version_string();

        HaDevice {
            identifiers: vec![device_id.clone()],
            name: format!("RustKVM {}", device_id),
            manufacturer: "RustKVM".to_string(),
            model: "RustKVM".to_string(),
            sw_version: if sw_version.is_empty() { None } else { Some(sw_version) },
            serial_number: Some(device_id.clone()),
            configuration_url: if config_url.is_empty() { None } else { Some(config_url) },
        }
    }

    pub(super) async fn publish_discovery(
        &self,
        component: &str,
        object_id: &str,
        mut payload: HaDiscoveryPayload,
    ) {
        payload.default_entity_id =
            Some(format!("{}.rustkvm_{}_{}", component, self.device_id, object_id));
        let discovery_topic =
            format!("homeassistant/{}/rustkvm_{}/{}/config", component, self.device_id, object_id);
        self.publish_json(&discovery_topic, &payload, true).await;
    }

    pub(super) async fn remove_discovery(&self, component: &str, object_id: &str) {
        let discovery_topic =
            format!("homeassistant/{}/rustkvm_{}/{}/config", component, self.device_id, object_id);
        self.publish_string(&discovery_topic, true, "").await;
    }

    async fn remove_atx_discovery(&self) {
        self.remove_discovery("binary_sensor", "power_led").await;
        self.remove_discovery("binary_sensor", "hdd_led").await;
        self.remove_discovery("button", "atx_power_short").await;
        self.remove_discovery("button", "atx_power_long").await;
        self.remove_discovery("button", "atx_reset").await;
    }

    async fn remove_dc_discovery(&self) {
        self.remove_discovery("sensor", "voltage").await;
        self.remove_discovery("sensor", "current").await;
        self.remove_discovery("sensor", "power").await;
        self.remove_discovery("switch", "dc_power").await;
        self.remove_discovery("binary_sensor", "dc_power").await;
        self.remove_discovery("select", "dc_restore").await;
        self.remove_discovery("sensor", "dc_restore").await;
    }

    pub async fn remove_all_discovery(&self) {
        self.remove_discovery("binary_sensor", "online").await;
        self.remove_discovery("binary_sensor", "video_signal").await;
        self.remove_discovery("sensor", "video_resolution").await;
        self.remove_discovery("sensor", "video_fps").await;
        self.remove_discovery("binary_sensor", "cloud_connected").await;
        self.remove_discovery("sensor", "active_sessions").await;
        self.remove_discovery("binary_sensor", "usb_state").await;
        self.remove_discovery("sensor", "ip_address").await;
        self.remove_discovery("sensor", "hostname").await;
        self.remove_discovery("sensor", "cpu_load").await;
        self.remove_discovery("sensor", "temperature").await;
        self.remove_discovery("sensor", "memory_used").await;
        self.remove_discovery("sensor", "storage_used").await;
        self.remove_discovery("sensor", "storage_free").await;
        self.remove_discovery("select", "virtual_media").await;
        self.remove_discovery("sensor", "virtual_media").await;
        self.remove_discovery("switch", "jiggler").await;
        self.remove_discovery("binary_sensor", "jiggler").await;
        self.remove_discovery("button", "reboot").await;
        self.remove_discovery("update", "firmware").await;

        self.remove_atx_discovery().await;
        self.remove_dc_discovery().await;

        info!("removed all HA discovery entries");
    }

    pub async fn cleanup_all_topics(&self) {
        self.remove_all_discovery().await;

        let state_topics = [
            self.topic(&["status"]),
            self.topic(&["video", "state"]),
            self.topic(&["cloud", "state"]),
            self.topic(&["sessions", "state"]),
            self.topic(&["usb", "state"]),
            self.topic(&["jiggler", "state"]),
            self.topic(&["network", "state"]),
            self.topic(&["system", "state"]),
            self.topic(&["virtual_media", "state"]),
            self.topic(&["update", "state"]),
            self.topic(&["atx", "state"]),
            self.topic(&["dc", "state"]),
        ];

        for topic in &state_topics {
            self.publish_string(topic, true, "").await;
        }

        info!("cleaned up all MQTT topics and discovery entries");
    }

    pub async fn publish_ha_discovery(&self) {
        let device = self.ha_device_info();
        let avail_topic = self.topic(&["status"]);
        let avail_template = "{{ 'online' if value_json.online else 'offline' }}";
        let enabled_by_default = Some(false);

        self.publish_discovery(
            "binary_sensor",
            "online",
            HaDiscoveryPayload {
                name: "Online".to_string(),
                unique_id: format!("rustkvm_{}_online", self.device_id),
                state_topic: Some(self.topic(&["status"])),
                value_template: Some("{{ 'ON' if value_json.online else 'OFF' }}".to_string()),
                device_class: Some("connectivity".to_string()),
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "binary_sensor",
            "video_signal",
            HaDiscoveryPayload {
                name: "Video Signal".to_string(),
                unique_id: format!("rustkvm_{}_video_signal", self.device_id),
                state_topic: Some(self.topic(&["video", "state"])),
                value_template: Some("{{ 'ON' if value_json.ready else 'OFF' }}".to_string()),
                device_class: Some("connectivity".to_string()),
                icon: Some("mdi:video-input-hdmi".to_string()),
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "video_resolution",
            HaDiscoveryPayload {
                name: "Video Resolution".to_string(),
                unique_id: format!("rustkvm_{}_video_resolution", self.device_id),
                state_topic: Some(self.topic(&["video", "state"])),
                value_template: Some("{{ value_json.width }}x{{ value_json.height }}".to_string()),
                icon: Some("mdi:monitor".to_string()),
                entity_category: Some("diagnostic".to_string()),
                enabled_by_default,
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "video_fps",
            HaDiscoveryPayload {
                name: "Video FPS".to_string(),
                unique_id: format!("rustkvm_{}_video_fps", self.device_id),
                state_topic: Some(self.topic(&["video", "state"])),
                value_template: Some("{{ value_json.fps | round(1) }}".to_string()),
                unit_of_measurement: Some("fps".to_string()),
                state_class: Some("measurement".to_string()),
                icon: Some("mdi:speedometer".to_string()),
                entity_category: Some("diagnostic".to_string()),
                enabled_by_default,
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "binary_sensor",
            "cloud_connected",
            HaDiscoveryPayload {
                name: "Cloud Connected".to_string(),
                unique_id: format!("rustkvm_{}_cloud_connected", self.device_id),
                state_topic: Some(self.topic(&["cloud", "state"])),
                value_template: Some("{{ 'ON' if value_json.connected else 'OFF' }}".to_string()),
                device_class: Some("connectivity".to_string()),
                icon: Some("mdi:cloud".to_string()),
                entity_category: Some("diagnostic".to_string()),
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "active_sessions",
            HaDiscoveryPayload {
                name: "Active Sessions".to_string(),
                unique_id: format!("rustkvm_{}_active_sessions", self.device_id),
                state_topic: Some(self.topic(&["sessions", "state"])),
                value_template: Some("{{ value_json.active_sessions }}".to_string()),
                icon: Some("mdi:account-multiple".to_string()),
                state_class: Some("measurement".to_string()),
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "binary_sensor",
            "usb_state",
            HaDiscoveryPayload {
                name: "USB Connected".to_string(),
                unique_id: format!("rustkvm_{}_usb_state", self.device_id),
                state_topic: Some(self.topic(&["usb", "state"])),
                value_template: Some(
                    "{{ 'ON' if value_json.state == 'configured' else 'OFF' }}".to_string(),
                ),
                device_class: Some("connectivity".to_string()),
                icon: Some("mdi:usb".to_string()),
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "ip_address",
            HaDiscoveryPayload {
                name: "IP Address".to_string(),
                unique_id: format!("rustkvm_{}_ip_address", self.device_id),
                state_topic: Some(self.topic(&["network", "state"])),
                value_template: Some("{{ value_json.ip_address }}".to_string()),
                icon: Some("mdi:ip-network".to_string()),
                entity_category: Some("diagnostic".to_string()),
                enabled_by_default,
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "hostname",
            HaDiscoveryPayload {
                name: "Hostname".to_string(),
                unique_id: format!("rustkvm_{}_hostname", self.device_id),
                state_topic: Some(self.topic(&["network", "state"])),
                value_template: Some("{{ value_json.hostname }}".to_string()),
                icon: Some("mdi:dns".to_string()),
                entity_category: Some("diagnostic".to_string()),
                enabled_by_default,
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "cpu_load",
            HaDiscoveryPayload {
                name: "CPU Load".to_string(),
                unique_id: format!("rustkvm_{}_cpu_load", self.device_id),
                state_topic: Some(self.topic(&["system", "state"])),
                value_template: Some("{{ value_json.cpu_load | round(2) }}".to_string()),
                state_class: Some("measurement".to_string()),
                icon: Some("mdi:chip".to_string()),
                entity_category: Some("diagnostic".to_string()),
                enabled_by_default,
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "temperature",
            HaDiscoveryPayload {
                name: "Temperature".to_string(),
                unique_id: format!("rustkvm_{}_temperature", self.device_id),
                state_topic: Some(self.topic(&["system", "state"])),
                value_template: Some("{{ value_json.temperature | round(1) }}".to_string()),
                device_class: Some("temperature".to_string()),
                unit_of_measurement: Some("°C".to_string()),
                state_class: Some("measurement".to_string()),
                entity_category: Some("diagnostic".to_string()),
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "memory_used",
            HaDiscoveryPayload {
                name: "Memory Used".to_string(),
                unique_id: format!("rustkvm_{}_memory_used", self.device_id),
                state_topic: Some(self.topic(&["system", "state"])),
                value_template: Some(
                    "{{ (value_json.memory_used / 1048576) | round(1) }}".to_string(),
                ),
                unit_of_measurement: Some("MB".to_string()),
                icon: Some("mdi:memory".to_string()),
                state_class: Some("measurement".to_string()),
                entity_category: Some("diagnostic".to_string()),
                enabled_by_default,
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "storage_used",
            HaDiscoveryPayload {
                name: "Storage Used".to_string(),
                unique_id: format!("rustkvm_{}_storage_used", self.device_id),
                state_topic: Some(self.topic(&["system", "state"])),
                value_template: Some(
                    "{{ (value_json.storage_used / 1048576) | round(1) }}".to_string(),
                ),
                device_class: Some("data_size".to_string()),
                unit_of_measurement: Some("MB".to_string()),
                state_class: Some("measurement".to_string()),
                entity_category: Some("diagnostic".to_string()),
                enabled_by_default,
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "storage_free",
            HaDiscoveryPayload {
                name: "Storage Free".to_string(),
                unique_id: format!("rustkvm_{}_storage_free", self.device_id),
                state_topic: Some(self.topic(&["system", "state"])),
                value_template: Some(
                    "{{ (value_json.storage_free / 1048576) | round(1) }}".to_string(),
                ),
                device_class: Some("data_size".to_string()),
                unit_of_measurement: Some("MB".to_string()),
                state_class: Some("measurement".to_string()),
                entity_category: Some("diagnostic".to_string()),
                enabled_by_default,
                availability_topic: avail_topic.clone(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        let actions_enabled = self.enable_actions;
        let vm_attrs_topic = self.topic(&["virtual_media", "state"]);
        let vm_attrs_template = "{{ {'source': value_json.source} | tojson }}";

        if actions_enabled {
            let vm_options = get_available_images().await;
            self.remove_discovery("sensor", "virtual_media").await;
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
                    json_attributes_topic: Some(vm_attrs_topic),
                    json_attributes_template: Some(vm_attrs_template.to_string()),
                    availability_topic: avail_topic.clone(),
                    availability_template: avail_template.to_string(),
                    device: Some(device.clone()),
                    ..Default::default()
                },
            )
            .await;
        } else {
            self.remove_discovery("select", "virtual_media").await;
            self.publish_discovery(
                "sensor",
                "virtual_media",
                HaDiscoveryPayload {
                    name: "Virtual Media".to_string(),
                    unique_id: format!("rustkvm_{}_virtual_media", self.device_id),
                    state_topic: Some(self.topic(&["virtual_media", "state"])),
                    value_template: Some("{{ value_json.mounted_image }}".to_string()),
                    icon: Some("mdi:disc".to_string()),
                    json_attributes_topic: Some(vm_attrs_topic),
                    json_attributes_template: Some(vm_attrs_template.to_string()),
                    availability_topic: avail_topic.clone(),
                    availability_template: avail_template.to_string(),
                    device: Some(device.clone()),
                    ..Default::default()
                },
            )
            .await;
        }

        if actions_enabled {
            self.remove_discovery("binary_sensor", "jiggler").await;
            self.publish_discovery(
                "switch",
                "jiggler",
                HaDiscoveryPayload {
                    name: "Mouse Jiggler".to_string(),
                    unique_id: format!("rustkvm_{}_jiggler", self.device_id),
                    state_topic: Some(self.topic(&["jiggler", "state"])),
                    command_topic: Some(self.topic(&["jiggler", "set"])),
                    value_template: Some("{{ 'ON' if value_json.enabled else 'OFF' }}".to_string()),
                    payload_on: Some("ON".to_string()),
                    payload_off: Some("OFF".to_string()),
                    icon: Some("mdi:mouse".to_string()),
                    availability_topic: avail_topic.clone(),
                    availability_template: avail_template.to_string(),
                    device: Some(device.clone()),
                    ..Default::default()
                },
            )
            .await;
        } else {
            self.remove_discovery("switch", "jiggler").await;
            self.publish_discovery(
                "binary_sensor",
                "jiggler",
                HaDiscoveryPayload {
                    name: "Mouse Jiggler".to_string(),
                    unique_id: format!("rustkvm_{}_jiggler", self.device_id),
                    state_topic: Some(self.topic(&["jiggler", "state"])),
                    value_template: Some("{{ 'ON' if value_json.enabled else 'OFF' }}".to_string()),
                    icon: Some("mdi:mouse".to_string()),
                    availability_topic: avail_topic.clone(),
                    availability_template: avail_template.to_string(),
                    device: Some(device.clone()),
                    ..Default::default()
                },
            )
            .await;
        }

        if actions_enabled {
            self.publish_discovery(
                "button",
                "reboot",
                HaDiscoveryPayload {
                    name: "Reboot".to_string(),
                    unique_id: format!("rustkvm_{}_reboot", self.device_id),
                    command_topic: Some(self.topic(&["reboot", "set"])),
                    payload_press: Some("PRESS".to_string()),
                    device_class: Some("restart".to_string()),
                    icon: Some("mdi:restart".to_string()),
                    entity_category: Some("config".to_string()),
                    availability_topic: avail_topic.clone(),
                    availability_template: avail_template.to_string(),
                    device: Some(device.clone()),
                    ..Default::default()
                },
            )
            .await;
        } else {
            self.remove_discovery("button", "reboot").await;
        }

        let mut firmware_payload = HaDiscoveryPayload {
            name: "Firmware".to_string(),
            unique_id: format!("rustkvm_{}_firmware", self.device_id),
            state_topic: Some(self.topic(&["update", "state"])),
            device_class: Some("firmware".to_string()),
            entity_category: Some("config".to_string()),
            release_url: Some("https://github.com/foxxcn/rustkvm/releases".to_string()),
            availability_topic: avail_topic.clone(),
            availability_template: avail_template.to_string(),
            device: Some(device.clone()),
            ..Default::default()
        };
        if actions_enabled {
            firmware_payload.command_topic = Some(self.topic(&["update", "install"]));
            firmware_payload.payload_install = Some("INSTALL".to_string());
        }
        self.publish_discovery("update", "firmware", firmware_payload).await;

        let active_extension = get_active_extension();
        match active_extension.as_str() {
            "atx-power" => {
                self.publish_atx_discovery(&device, &avail_topic, avail_template, actions_enabled)
                    .await;
                self.remove_dc_discovery().await;
            }
            "dc-power" => {
                self.publish_dc_discovery(&device, &avail_topic, avail_template, actions_enabled)
                    .await;
                self.remove_atx_discovery().await;
            }
            _ => {
                self.remove_atx_discovery().await;
                self.remove_dc_discovery().await;
            }
        }

        info!(extension = %active_extension, "published Home Assistant discovery configs");
    }

    async fn publish_atx_discovery(
        &self,
        device: &HaDevice,
        avail_topic: &str,
        avail_template: &str,
        actions_enabled: bool,
    ) {
        self.publish_discovery(
            "binary_sensor",
            "power_led",
            HaDiscoveryPayload {
                name: "ATX Power LED".to_string(),
                unique_id: format!("rustkvm_{}_power_led", self.device_id),
                state_topic: Some(self.topic(&["atx", "state"])),
                value_template: Some("{{ 'ON' if value_json.power else 'OFF' }}".to_string()),
                icon: Some("mdi:led-on".to_string()),
                availability_topic: avail_topic.to_string(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "binary_sensor",
            "hdd_led",
            HaDiscoveryPayload {
                name: "ATX HDD LED".to_string(),
                unique_id: format!("rustkvm_{}_hdd_led", self.device_id),
                state_topic: Some(self.topic(&["atx", "state"])),
                value_template: Some("{{ 'ON' if value_json.hdd else 'OFF' }}".to_string()),
                icon: Some("mdi:harddisk".to_string()),
                availability_topic: avail_topic.to_string(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        if actions_enabled {
            self.publish_discovery(
                "button",
                "atx_power_short",
                HaDiscoveryPayload {
                    name: "ATX Power (Short Press)".to_string(),
                    unique_id: format!("rustkvm_{}_atx_power_short", self.device_id),
                    command_topic: Some(self.topic(&["atx_power_short", "set"])),
                    payload_press: Some("PRESS".to_string()),
                    icon: Some("mdi:power".to_string()),
                    availability_topic: avail_topic.to_string(),
                    availability_template: avail_template.to_string(),
                    device: Some(device.clone()),
                    ..Default::default()
                },
            )
            .await;

            self.publish_discovery(
                "button",
                "atx_power_long",
                HaDiscoveryPayload {
                    name: "ATX Power (Long Press)".to_string(),
                    unique_id: format!("rustkvm_{}_atx_power_long", self.device_id),
                    command_topic: Some(self.topic(&["atx_power_long", "set"])),
                    payload_press: Some("PRESS".to_string()),
                    icon: Some("mdi:power".to_string()),
                    availability_topic: avail_topic.to_string(),
                    availability_template: avail_template.to_string(),
                    device: Some(device.clone()),
                    ..Default::default()
                },
            )
            .await;

            self.publish_discovery(
                "button",
                "atx_reset",
                HaDiscoveryPayload {
                    name: "ATX Reset".to_string(),
                    unique_id: format!("rustkvm_{}_atx_reset", self.device_id),
                    command_topic: Some(self.topic(&["atx_reset", "set"])),
                    payload_press: Some("PRESS".to_string()),
                    icon: Some("mdi:restart".to_string()),
                    availability_topic: avail_topic.to_string(),
                    availability_template: avail_template.to_string(),
                    device: Some(device.clone()),
                    ..Default::default()
                },
            )
            .await;
        } else {
            self.remove_discovery("button", "atx_power_short").await;
            self.remove_discovery("button", "atx_power_long").await;
            self.remove_discovery("button", "atx_reset").await;
        }
    }

    async fn publish_dc_discovery(
        &self,
        device: &HaDevice,
        avail_topic: &str,
        avail_template: &str,
        actions_enabled: bool,
    ) {
        self.publish_discovery(
            "sensor",
            "voltage",
            HaDiscoveryPayload {
                name: "DC Voltage".to_string(),
                unique_id: format!("rustkvm_{}_voltage", self.device_id),
                state_topic: Some(self.topic(&["dc", "state"])),
                value_template: Some("{{ value_json.voltage | round(2) }}".to_string()),
                device_class: Some("voltage".to_string()),
                unit_of_measurement: Some("V".to_string()),
                state_class: Some("measurement".to_string()),
                availability_topic: avail_topic.to_string(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "current",
            HaDiscoveryPayload {
                name: "DC Current".to_string(),
                unique_id: format!("rustkvm_{}_current", self.device_id),
                state_topic: Some(self.topic(&["dc", "state"])),
                value_template: Some("{{ value_json.current | round(3) }}".to_string()),
                device_class: Some("current".to_string()),
                unit_of_measurement: Some("A".to_string()),
                state_class: Some("measurement".to_string()),
                availability_topic: avail_topic.to_string(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        self.publish_discovery(
            "sensor",
            "power",
            HaDiscoveryPayload {
                name: "DC Power".to_string(),
                unique_id: format!("rustkvm_{}_power", self.device_id),
                state_topic: Some(self.topic(&["dc", "state"])),
                value_template: Some("{{ value_json.power | round(2) }}".to_string()),
                device_class: Some("power".to_string()),
                unit_of_measurement: Some("W".to_string()),
                state_class: Some("measurement".to_string()),
                availability_topic: avail_topic.to_string(),
                availability_template: avail_template.to_string(),
                device: Some(device.clone()),
                ..Default::default()
            },
        )
        .await;

        if actions_enabled {
            self.remove_discovery("binary_sensor", "dc_power").await;
            self.publish_discovery(
                "switch",
                "dc_power",
                HaDiscoveryPayload {
                    name: "DC Power".to_string(),
                    unique_id: format!("rustkvm_{}_dc_power", self.device_id),
                    state_topic: Some(self.topic(&["dc", "state"])),
                    command_topic: Some(self.topic(&["dc_power", "set"])),
                    value_template: Some("{{ 'ON' if value_json.isOn else 'OFF' }}".to_string()),
                    payload_on: Some("ON".to_string()),
                    payload_off: Some("OFF".to_string()),
                    device_class: Some("switch".to_string()),
                    icon: Some("mdi:power".to_string()),
                    availability_topic: avail_topic.to_string(),
                    availability_template: avail_template.to_string(),
                    device: Some(device.clone()),
                    ..Default::default()
                },
            )
            .await;
        } else {
            self.remove_discovery("switch", "dc_power").await;
            self.publish_discovery(
                "binary_sensor",
                "dc_power",
                HaDiscoveryPayload {
                    name: "DC Power".to_string(),
                    unique_id: format!("rustkvm_{}_dc_power", self.device_id),
                    state_topic: Some(self.topic(&["dc", "state"])),
                    value_template: Some("{{ 'ON' if value_json.isOn else 'OFF' }}".to_string()),
                    device_class: Some("power".to_string()),
                    icon: Some("mdi:power".to_string()),
                    availability_topic: avail_topic.to_string(),
                    availability_template: avail_template.to_string(),
                    device: Some(device.clone()),
                    ..Default::default()
                },
            )
            .await;
        }

        let dc_restore_value_template = "{% set rs = value_json.restoreState | int %}{% if rs == 0 %}off{% elif rs == 1 %}on{% elif rs == 2 %}last_state{% else %}unknown{% endif %}";
        let dc_state = get_dc_state();
        if dc_state.restore_state >= 0 {
            if actions_enabled {
                self.remove_discovery("sensor", "dc_restore").await;
                self.publish_discovery(
                    "select",
                    "dc_restore",
                    HaDiscoveryPayload {
                        name: "Restore on Power Loss".to_string(),
                        unique_id: format!("rustkvm_{}_dc_restore", self.device_id),
                        state_topic: Some(self.topic(&["dc", "state"])),
                        command_topic: Some(self.topic(&["dc_restore", "set"])),
                        value_template: Some(dc_restore_value_template.to_string()),
                        options: Some(vec![
                            "off".to_string(),
                            "on".to_string(),
                            "last_state".to_string(),
                        ]),
                        icon: Some("mdi:power-settings".to_string()),
                        entity_category: Some("config".to_string()),
                        availability_topic: avail_topic.to_string(),
                        availability_template: avail_template.to_string(),
                        device: Some(device.clone()),
                        ..Default::default()
                    },
                )
                .await;
            } else {
                self.remove_discovery("select", "dc_restore").await;
                self.publish_discovery(
                    "sensor",
                    "dc_restore",
                    HaDiscoveryPayload {
                        name: "Restore on Power Loss".to_string(),
                        unique_id: format!("rustkvm_{}_dc_restore_ro", self.device_id),
                        state_topic: Some(self.topic(&["dc", "state"])),
                        value_template: Some(dc_restore_value_template.to_string()),
                        icon: Some("mdi:power-settings".to_string()),
                        availability_topic: avail_topic.to_string(),
                        availability_template: avail_template.to_string(),
                        device: Some(device.clone()),
                        ..Default::default()
                    },
                )
                .await;
            }
        } else {
            self.remove_discovery("select", "dc_restore").await;
            self.remove_discovery("sensor", "dc_restore").await;
        }
    }
}
