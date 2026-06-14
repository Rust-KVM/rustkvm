use tokio::sync::Mutex;

mod commands;
mod discovery;
mod lifecycle;
mod manager;
mod publish;
mod rpc;
mod stubs;
mod tls;
mod types;

pub use lifecycle::{restart_mqtt, start_mqtt};
pub(crate) use manager::MqttManager;
pub use rpc::{
    rpc_get_mqtt_settings, rpc_get_mqtt_status, rpc_set_mqtt_settings, rpc_test_mqtt_connection,
    validate_base_topic,
};
pub use types::{
    AtxState, DcPowerState, HaDevice, HaDiscoveryPayload, MQTT_PASSWORD_MASK, MqttCloudState,
    MqttConfig, MqttJigglerState, MqttNetworkState, MqttSessionsState, MqttStatusPayload,
    MqttStatusResponse, MqttSystemState, MqttTestResult, MqttUpdateState, MqttUsbState,
    MqttVideoState, MqttVirtualMediaState,
};

pub(crate) static MQTT_MANAGER: tokio::sync::OnceCell<Mutex<Option<MqttManager>>> =
    tokio::sync::OnceCell::const_new();
