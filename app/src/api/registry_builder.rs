use std::sync::Arc;

use anyhow::anyhow;
use once_cell::sync::Lazy;
use serde_json::Value;

use super::handlers::{hid, media, network, system, usb};
use super::registry::RpcRegistry;

static DEFAULT_REGISTRY: Lazy<Arc<RpcRegistry>> = Lazy::new(|| Arc::new(create_default_registry()));

pub fn default_registry() -> Arc<RpcRegistry> {
    DEFAULT_REGISTRY.clone()
}

pub fn create_default_registry() -> RpcRegistry {
    let mut registry = RpcRegistry::new();

    registry.register_no_params("ping", system::ping);
    registry.register_no_params("getDeviceID", system::get_device_id);
    registry.register_no_params("getFailsafeMode", system::get_failsafe_mode);
    registry.register_typed("reboot", system::reboot);

    registry.register_typed("setSerialSettings", system::set_serial_settings);
    registry.register_no_params("getSerialSettings", system::get_serial_settings);

    registry.register_async("setDisplayRotation", |params| {
        Box::pin(async move {
            let p: media::DisplayRotationParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            media::set_display_rotation(p).await
        })
    });
    registry.register_async("getDisplayRotation", |_params| {
        Box::pin(async move {
            let v = media::get_display_rotation().await?;
            Ok(serde_json::to_value(v)?)
        })
    });
    registry.register_async("setBacklightSettings", |params| {
        Box::pin(async move {
            let p: media::BacklightSettings =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            media::set_backlight_settings(p).await
        })
    });
    registry.register_async("getBacklightSettings", |_params| {
        Box::pin(async move {
            let v = media::get_backlight_settings().await?;
            Ok(serde_json::to_value(v)?)
        })
    });

    registry.register_no_params("getStreamQualityFactor", media::get_stream_quality_factor);
    registry.register_typed("setStreamQualityFactor", media::set_stream_quality_factor);
    registry.register_async("getAutoUpdateState", |_params| {
        Box::pin(async move {
            let v = system::get_auto_update_state().await?;
            Ok(serde_json::to_value(v)?)
        })
    });
    registry.register_async("setAutoUpdateState", |params| {
        Box::pin(async move {
            let params: system::AutoUpdateParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            let v = system::set_auto_update_state(params).await?;
            Ok(serde_json::to_value(v)?)
        })
    });

    registry.register_no_params("getEDID", media::get_edid);
    registry.register_typed("setEDID", media::set_edid);

    registry.register_no_params("getUsbDevices", usb::get_usb_devices);
    registry.register_typed("setUsbDevices", usb::set_usb_devices);
    registry.register_typed("setUsbDeviceState", usb::set_usb_device_state);
    registry.register_no_params("getUsbEmulationState", usb::get_usb_emulation_state);
    registry.register_async("setUsbEmulationState", |params| {
        Box::pin(async move {
            let p: usb::UsbEmulationParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            usb::set_usb_emulation_state(p).await
        })
    });

    registry.register_typed("keyboardReport", hid::keyboard_report);
    registry.register_typed("absMouseReport", hid::abs_mouse_report);
    registry.register_typed("relMouseReport", hid::rel_mouse_report);
    registry.register_typed("wheelReport", hid::wheel_report);
    registry.register_no_params("getKeyboardLayout", hid::get_keyboard_layout);
    registry.register_typed("setKeyboardLayout", hid::set_keyboard_layout);
    registry.register_no_params("getKeyboardLedState", hid::get_keyboard_led_state);
    registry.register_no_params("getKeyDownState", hid::get_key_down_state);
    registry.register_typed("keypressReport", hid::keypress_report);

    registry.register_no_params("getUSBState", usb::get_usb_state);
    registry.register_typed("setUSBState", usb::set_usb_state);
    registry.register_no_params("getUsbConfig", usb::get_usb_config);
    registry.register_typed("setUsbConfig", usb::set_usb_config);

    registry.register_no_params("getUpdateStatus", system::get_update_status);
    registry.register_async("getLocalVersion", |_params| {
        Box::pin(async move {
            let v = system::get_local_version().await?;
            Ok(serde_json::to_value(v)?)
        })
    });
    registry.register_no_params("isUpdatePending", system::is_update_pending);
    registry.register_async("getDevChannelState", |_params| {
        Box::pin(async move {
            let v = system::get_dev_channel_state().await?;
            Ok(serde_json::to_value(v)?)
        })
    });
    registry.register_async("setDevChannelState", |params| {
        Box::pin(async move {
            let params: system::DevChannelParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            system::set_dev_channel_state(params).await
        })
    });

    registry.register_async("getTLSState", |_params| {
        Box::pin(async move {
            let result = crate::tls::get_tls_state().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("setTLSState", |params| {
        Box::pin(async move {
            let raw = params.ok_or(anyhow!("Missing required parameters"))?;
            let state_val = if let Some(s) = raw.get("state") { s.clone() } else { raw };
            let state: crate::tls::TlsState = serde_json::from_value(state_val)?;
            crate::tls::apply_tls_state(
                &state.mode,
                state.certificate.as_deref(),
                state.private_key.as_deref(),
            )
            .await?;
            Ok(serde_json::Value::Null)
        })
    });

    registry.register_async("getKeyboardMacros", |_params| {
        Box::pin(async move {
            let result = hid::get_keyboard_macros().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("setKeyboardMacros", |params| {
        Box::pin(async move {
            let raw = params.ok_or(anyhow!("Missing required parameters"))?;
            let params_val = if let Value::Object(map) = &raw {
                map.get("params").cloned().unwrap_or_else(|| raw.clone())
            } else {
                raw
            };
            let params: hid::KeyboardMacrosParams = serde_json::from_value(params_val)?;
            hid::set_keyboard_macros(params).await
        })
    });

    registry.register_async("getCloudState", |_params| {
        Box::pin(async move {
            let result = network::get_cloud_state().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_typed("setCloudState", network::set_cloud_state);
    registry.register_async("deregisterDevice", |_params| {
        Box::pin(async move { network::deregister_device().await })
    });
    registry.register_async("resetConfig", |_params| {
        Box::pin(async move { system::reset_config().await })
    });
    registry.register_async("setCloudUrl", |params| {
        Box::pin(async move {
            let params: network::CloudUrlParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            network::set_cloud_url(params).await
        })
    });

    registry.register_no_params("getNetworkState", network::get_network_state);
    registry.register_async("getNetworkSettings", |_params| {
        Box::pin(async move {
            let v = network::get_network_settings().await?;
            Ok(serde_json::to_value(v)?)
        })
    });
    registry.register_async("setNetworkSettings", |params| {
        Box::pin(async move {
            let params: network::NetworkSettingsParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            network::set_network_settings(params).await
        })
    });
    registry.register_no_params("renewDHCPLease", network::renew_dhcp_lease);
    registry.register_no_params("getNetworkIpAddress", network::get_network_ip_address);
    registry.register_typed("setNetworkIpAddress", network::set_network_ip_address);
    registry.register_no_params("getDisplayState", media::get_display_state);

    registry.register_async("getLocalLoopbackOnly", |_params| {
        Box::pin(async move {
            let result = network::get_local_loopback_only().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("setLocalLoopbackOnly", |params| {
        Box::pin(async move {
            let params: network::LocalLoopbackParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            network::set_local_loopback_only(params).await
        })
    });

    registry.register_async("getWakeOnLanDevices", |_params| {
        Box::pin(async move {
            let list = network::get_wake_on_lan_devices().await?;
            Ok(serde_json::to_value(list)?)
        })
    });
    registry.register_async("setWakeOnLanDevices", |params| {
        Box::pin(async move {
            let raw = params.ok_or(anyhow!("Missing required parameters"))?;
            let inner = raw.get("params").ok_or_else(|| anyhow!("Missing field 'params'"))?.clone();
            let params: network::SetWakeOnLanDevicesParams = serde_json::from_value(inner)?;
            network::set_wake_on_lan_devices(params).await
        })
    });
    registry.register_async("sendWOLMagicPacket", |params| {
        Box::pin(async move {
            let params: network::SendWOLMagicPacketParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            network::send_wol_magic_packet(params).await
        })
    });

    registry.register_no_params("getVirtualMediaState", media::get_virtual_media_state);
    registry.register_async("getVideoState", |_params| {
        Box::pin(async move {
            let result = media::get_video_state().await?;
            Ok(serde_json::to_value(result)?)
        })
    });

    registry.register_async("mountWithHTTP", |params| {
        Box::pin(async move {
            let params: media::MountHttpParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            media::mount_with_http(params).await
        })
    });
    registry.register_async("mountWithWebRTC", |params| {
        Box::pin(async move {
            let params: media::MountWebRtcParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            media::mount_with_webrtc(params).await
        })
    });
    registry.register_async("mountWithStorage", |params| {
        Box::pin(async move {
            let params: media::MountStorageParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            media::mount_with_storage(params).await
        })
    });
    registry.register_async("unmountImage", |_params| {
        Box::pin(async move { media::unmount_image().await })
    });
    registry.register_async("setMassStorageMode", |params| {
        Box::pin(async move {
            let params: media::MassStorageModeParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            let result = media::set_mass_storage_mode(params).await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("getMassStorageMode", |_params| {
        Box::pin(async move {
            let result = media::get_mass_storage_mode().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_typed("checkMountUrl", media::check_mount_url);

    registry.register_async("startStorageFileUpload", |params| {
        Box::pin(async move {
            let params: media::StartUploadParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            let result = media::start_storage_file_upload(params).await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("listStorageFiles", |_params| {
        Box::pin(async move {
            let result = media::list_storage_files().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("deleteStorageFile", |params| {
        Box::pin(async move {
            let params: media::DeleteStorageFileParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            media::delete_storage_file(params).await
        })
    });
    registry.register_async("getStorageSpace", |_params| {
        Box::pin(async move {
            let result = media::get_storage_space().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("mountBuiltInImage", |params| {
        Box::pin(async move {
            let params: media::MountBuiltInImageParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            media::mount_built_in_image(params).await
        })
    });
    registry.register_async("rpcMountBuiltInImage", |params| {
        Box::pin(async move {
            let params: media::RpcMountBuiltInImageParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            media::rpc_mount_built_in_image(params).await
        })
    });

    registry.register_async("getSSHKeyState", |_params| {
        Box::pin(async move {
            let v = system::get_ssh_key_state().await?;
            Ok(serde_json::to_value(v)?)
        })
    });
    registry.register_async("setSSHKeyState", |params| {
        Box::pin(async move {
            let p: system::SshKeyParam =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            system::set_ssh_key_state(p).await
        })
    });
    registry.register_async("getDevModeState", |_params| {
        Box::pin(async move {
            let result = system::get_dev_mode_state_handler().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("setDevModeState", |params| {
        Box::pin(async move {
            let params: system::DevModeParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            system::set_dev_mode_state_handler(params).await
        })
    });

    registry.register_no_params("getVideoCodecPreference", media::get_video_codec_preference);
    registry.register_typed("setVideoCodecPreference", media::set_video_codec_preference);
    registry.register_no_params("getVideoSleepMode", media::get_video_sleep_mode);
    registry.register_typed("setVideoSleepMode", media::set_video_sleep_mode);
    registry.register_async("getVideoLogStatus", |_params| {
        Box::pin(async move {
            let v = media::get_video_log_status().await?;
            Ok(serde_json::to_value(v)?)
        })
    });

    registry.register_async("getUpdateStatusChannel", |_params| {
        Box::pin(async move {
            let result = system::get_update_status_channel()?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("checkUpdateComponents", |params| {
        Box::pin(async move {
            let params: system::CheckUpdateParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing params"))?)?;
            let result = system::check_update_components(params).await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry
        .register_async("tryUpdate", |_params| Box::pin(async move { system::try_update().await }));
    registry.register_async("tryUpdateComponents", |params| {
        Box::pin(async move {
            let params: system::TryUpdateComponentsParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing params"))?)?;
            system::try_update_components(params).await
        })
    });
    registry.register_async("factoryReset", |_params| {
        Box::pin(async move { system::factory_reset().await })
    });

    registry.register_no_params("getDefaultLogLevel", system::get_default_log_level);
    registry.register_typed("setDefaultLogLevel", system::set_default_log_level);
    registry.register_typed("emitTestLog", system::emit_test_log);
    registry.register_async("getTimezones", |_params| {
        Box::pin(async move {
            let result = system::get_timezones()?;
            Ok(serde_json::to_value(result)?)
        })
    });

    registry.register_async("getPublicIPAddresses", |params| {
        Box::pin(async move {
            let p = params.unwrap_or(serde_json::json!({}));
            let params: network::CheckPublicIPParams = serde_json::from_value(p)?;
            let result = network::get_public_ip_addresses(params).await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("checkPublicIPAddresses", |_params| {
        Box::pin(async move { network::check_public_ip_addresses().await })
    });

    registry.register_no_params("getDCPowerState", system::get_dc_power_state);
    registry.register_async("setDCPowerState", |params| {
        Box::pin(async move {
            let p: system::SetDcPowerParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            system::set_dc_power_state(p).await
        })
    });
    registry.register_async("setDCRestoreState", |params| {
        Box::pin(async move {
            let p: system::DcRestoreParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            system::set_dc_restore_state(p).await
        })
    });
    registry.register_no_params("getATXState", system::get_atx_state);
    registry.register_async("setATXPowerAction", |params| {
        Box::pin(async move {
            let p: system::AtxPowerActionParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            system::set_atx_power_action(p).await
        })
    });
    registry.register_no_params("getActiveExtension", system::get_active_extension);
    registry.register_async("setActiveExtension", |params| {
        Box::pin(async move {
            let p: system::ExtensionParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            system::set_active_extension(p).await
        })
    });

    registry.register_async("getJigglerState", |_params| {
        Box::pin(async move {
            let result = system::get_jiggler_state().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("setJigglerState", |params| {
        Box::pin(async move {
            let raw = params.ok_or(anyhow!("Missing required parameters"))?;
            let p: system::JigglerStateParams = serde_json::from_value(raw)?;
            system::set_jiggler_state(p).await
        })
    });
    registry.register_no_params("getJigglerConfig", system::get_jiggler_config);
    registry.register_async("setJigglerConfig", |params| {
        Box::pin(async move {
            let raw = params.ok_or(anyhow!("Missing required parameters"))?;
            let p: system::SetJigglerConfigParams = serde_json::from_value(raw)?;
            system::set_jiggler_config(p).await
        })
    });

    registry.register_async("getTailscaleStatus", |_params| {
        Box::pin(async move { network::get_tailscale_status() })
    });
    registry.register_no_params("getTailscaleControlURL", network::get_tailscale_control_url);
    registry.register_typed("setTailscaleControlURL", network::set_tailscale_control_url);

    registry.register_async("getMqttSettings", |_params| {
        Box::pin(async move {
            let result = system::get_mqtt_settings().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("setMqttSettings", |params| {
        Box::pin(async move {
            let raw = params.ok_or(anyhow!("Missing required parameters"))?;
            let p: system::SetMqttSettingsParams = serde_json::from_value(raw)?;
            system::set_mqtt_settings(p).await
        })
    });
    registry.register_async("getMqttStatus", |_params| {
        Box::pin(async move {
            let result = system::get_mqtt_status().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("testMqttConnection", |params| {
        Box::pin(async move {
            let raw = params.ok_or(anyhow!("Missing required parameters"))?;
            let p: system::SetMqttSettingsParams = serde_json::from_value(raw)?;
            system::test_mqtt_connection(p).await
        })
    });

    registry.register_typed("sendCustomCommand", system::send_custom_command);
    registry.register_async("getSerialCommandHistory", |_params| {
        Box::pin(async move {
            let v = system::get_serial_command_history().await?;
            Ok(serde_json::to_value(v)?)
        })
    });
    registry.register_async("setSerialCommandHistory", |params| {
        Box::pin(async move {
            let p: system::CommandHistoryParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            system::set_serial_command_history(p).await
        })
    });
    registry.register_async("deleteSerialCommandHistory", |_params| {
        Box::pin(async move { system::delete_serial_command_history().await })
    });
    registry.register_typed("setTerminalPaused", system::set_terminal_paused);

    registry.register_async("executeKeyboardMacro", |params| {
        Box::pin(async move {
            let steps: Vec<serde_json::Value> =
                serde_json::from_value(params.ok_or(anyhow!("Missing params"))?)?;
            hid::execute_keyboard_macro(steps).await
        })
    });
    registry.register_no_params("cancelKeyboardMacro", hid::cancel_keyboard_macro);

    registry
}
