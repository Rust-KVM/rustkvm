//! JSON-RPC 2.0 protocol implementation for remote procedure calls.
//!
//! This module provides a complete JSON-RPC 2.0 implementation for handling
//! RPC requests sent through WebRTC data channels. It supports method calls,
//! error handling, and event notifications.
//!
//! # Components
//! - `JsonRpcRequest`, `JsonRpcResponse`, `JsonRpcEvent`: Protocol data structures
//! - `RpcHandler` trait: Handler interface definition
//! - `RpcRegistry`: Method registration and management
//! - `JsonRpcProcessor`: Main message processor
//!
//! # Safety
//! - Input validation and parameter checking
//! - Error handling to prevent panics
//! - Type-safe parameter serialization/deserialization

use std::collections::HashMap;
use std::panic;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use futures::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{debug, error, info, warn};
use webrtc::data_channel::data_channel_message::DataChannelMessage;

use crate::cloud::types::{CloudConnectionState, CloudState};
use crate::config::get_config_manager;
use crate::hardware::edid;
use crate::hardware::usb::KeyboardState as HidKeyboardState;
use crate::jsonrpc::handlers::*;
use crate::session::Session;
use crate::webrtc::{get_current_session, get_rpc_channel};

/// JSON-RPC 2.0 request structure
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
    pub id: Option<Value>,
}

/// JSON-RPC 2.0 response structure
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
    pub id: Option<Value>,
}

/// JSON-RPC 2.0 event structure (notification)
#[derive(Debug, Serialize)]
pub struct JsonRpcEvent {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

/// JSON-RPC error structure
#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    /// Parse error (-32700)
    pub fn parse_error() -> Self {
        Self { code: -32700, message: "Parse error".to_string(), data: None }
    }

    /// Invalid request (-32600)
    pub fn invalid_request() -> Self {
        Self { code: -32600, message: "Invalid Request".to_string(), data: None }
    }

    /// Method not found (-32601)
    pub fn method_not_found() -> Self {
        Self { code: -32601, message: "Method not found".to_string(), data: None }
    }

    /// Invalid params (-32602)
    pub fn invalid_params(data: Option<Value>) -> Self {
        Self { code: -32602, message: "Invalid params".to_string(), data }
    }

    /// Internal error (-32603)
    pub fn internal_error(data: Option<String>) -> Self {
        Self { code: -32603, message: "Internal error".to_string(), data: data.map(Value::String) }
    }
}

/// RPC handler trait
pub trait RpcHandler: Send + Sync {
    /// Execute RPC method
    fn call(&self, params: Option<Value>) -> Result<Value>;

    /// Execute RPC method asynchronously
    fn call_async(&self, params: Option<Value>) -> BoxFuture<'_, Result<Value>>;
}

/// Simple function handler
pub struct FunctionHandler<F> {
    func: F,
}

impl<F> FunctionHandler<F>
where
    F: Fn(Option<Value>) -> Result<Value> + Send + Sync,
{
    pub fn new(func: F) -> Self {
        Self { func }
    }
}

impl<F> RpcHandler for FunctionHandler<F>
where
    F: Fn(Option<Value>) -> Result<Value> + Send + Sync,
{
    fn call(&self, params: Option<Value>) -> Result<Value> {
        (self.func)(params)
    }

    fn call_async(&self, params: Option<Value>) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move { self.call(params) })
    }
}

/// Typed parameter handler
pub struct TypedHandler<P, R, F> {
    func: F,
    _phantom: std::marker::PhantomData<(P, R)>,
}

impl<P, R, F> TypedHandler<P, R, F>
where
    P: DeserializeOwned + Send + Sync + 'static,
    R: Serialize + Send + Sync + 'static,
    F: Fn(P) -> Result<R> + Send + Sync,
{
    pub fn new(func: F) -> Self {
        Self { func, _phantom: std::marker::PhantomData }
    }
}

impl<P, R, F> RpcHandler for TypedHandler<P, R, F>
where
    P: DeserializeOwned + Send + Sync + 'static,
    R: Serialize + Send + Sync + 'static,
    F: Fn(P) -> Result<R> + Send + Sync,
{
    fn call(&self, params: Option<Value>) -> Result<Value> {
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let params_value = params.unwrap_or(Value::Object(serde_json::Map::new()));

            let params = convert_parameters::<P>(params_value)
                .map_err(|e| anyhow!("Parameter conversion failed: {}", e))?;

            let result = (self.func)(params)?;
            serde_json::to_value(result).map_err(|e| anyhow!("Serialization failed: {}", e))
        }));

        match result {
            Ok(value) => value,
            Err(panic_payload) => {
                let error_msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    format!("Handler panicked: {}", s)
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    format!("Handler panicked: {}", s)
                } else {
                    "Handler panicked with unknown payload".to_string()
                };
                error!("RPC handler panic recovered: {}", error_msg);
                Err(anyhow!("Internal error: {}", error_msg))
            }
        }
    }

    fn call_async(&self, params: Option<Value>) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move { self.call(params) })
    }
}

/// No parameters handler
pub struct NoParamsHandler<R, F> {
    func: F,
    _phantom: std::marker::PhantomData<R>,
}

impl<R, F> NoParamsHandler<R, F>
where
    R: Serialize + Send + Sync + 'static,
    F: Fn() -> Result<R> + Send + Sync,
{
    pub fn new(func: F) -> Self {
        Self { func, _phantom: std::marker::PhantomData }
    }
}

impl<R, F> RpcHandler for NoParamsHandler<R, F>
where
    R: Serialize + Send + Sync + 'static,
    F: Fn() -> Result<R> + Send + Sync,
{
    fn call(&self, _params: Option<Value>) -> Result<Value> {
        let result = panic::catch_unwind(panic::AssertUnwindSafe(|| {
            let result = (self.func)()?;
            serde_json::to_value(result).map_err(|e| anyhow!("Serialization failed: {}", e))
        }));

        match result {
            Ok(value) => value,
            Err(panic_payload) => {
                let error_msg = if let Some(s) = panic_payload.downcast_ref::<&str>() {
                    format!("Handler panicked: {}", s)
                } else if let Some(s) = panic_payload.downcast_ref::<String>() {
                    format!("Handler panicked: {}", s)
                } else {
                    "Handler panicked with unknown payload".to_string()
                };
                error!("RPC handler panic recovered: {}", error_msg);
                Err(anyhow!("Internal error: {}", error_msg))
            }
        }
    }

    fn call_async(&self, params: Option<Value>) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move { self.call(params) })
    }
}

pub struct AsyncHandler<F> {
    func: F,
}

impl<F> AsyncHandler<F>
where
    F: Fn(Option<Value>) -> BoxFuture<'static, Result<Value>> + Send + Sync + 'static,
{
    pub fn new(func: F) -> Self {
        Self { func }
    }
}

impl<F> RpcHandler for AsyncHandler<F>
where
    F: Fn(Option<Value>) -> BoxFuture<'static, Result<Value>> + Send + Sync + 'static,
{
    fn call(&self, params: Option<Value>) -> Result<Value> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on((self.func)(params))
        })
    }

    fn call_async(&self, params: Option<Value>) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move { (self.func)(params).await })
    }
}

/// Provides flexible parameter type conversion with fallback strategies
fn convert_parameters<P: DeserializeOwned>(params: Value) -> Result<P> {
    let params_to_use = if let Value::Object(ref map) = params {
        if map.len() == 1 {
            map.get("params").cloned().unwrap_or_else(|| params.clone())
        } else {
            params.clone()
        }
    } else {
        params.clone()
    };

    // First try direct deserialization
    match serde_json::from_value::<P>(params_to_use.clone()) {
        Ok(result) => Ok(result),
        Err(primary_error) => {
            // Try enhanced conversion strategies
            match enhanced_parameter_conversion::<P>(&params_to_use) {
                Ok(result) => Ok(result),
                Err(_) => {
                    // Return the original error for better debugging
                    Err(anyhow!("Parameter conversion failed: {}", primary_error))
                }
            }
        }
    }
}

/// Enhanced parameter conversion with type coercion strategies
fn enhanced_parameter_conversion<P: DeserializeOwned>(params: &Value) -> Result<P> {
    match params {
        // Handle object parameters - convert Map to Struct via JSON round-trip
        Value::Object(map) => {
            let converted_map = convert_object_values(map)?;
            let json_value = Value::Object(converted_map);
            serde_json::from_value(json_value)
                .map_err(|e| anyhow!("Object conversion failed: {}", e))
        }

        // Handle array parameters with element type conversion
        Value::Array(arr) => {
            let converted_array = convert_array_values(arr)?;
            let json_value = Value::Array(converted_array);
            serde_json::from_value(json_value)
                .map_err(|e| anyhow!("Array conversion failed: {}", e))
        }

        // Handle scalar values with type coercion
        _ => {
            let converted_value = convert_scalar_value(params)?;
            serde_json::from_value(converted_value)
                .map_err(|e| anyhow!("Scalar conversion failed: {}", e))
        }
    }
}

/// Convert object values with type coercion
fn convert_object_values(
    map: &serde_json::Map<String, Value>,
) -> Result<serde_json::Map<String, Value>> {
    let mut converted_map = serde_json::Map::new();

    for (key, value) in map {
        let converted_value = match value {
            // Convert float64 to integer types where reasonable
            Value::Number(n) if n.is_f64() => {
                if let Some(f_val) = n.as_f64() {
                    if f_val.fract() == 0.0 && f_val >= 0.0 && f_val <= u32::MAX as f64 {
                        Value::Number(serde_json::Number::from(f_val as u32))
                    } else {
                        value.clone()
                    }
                } else {
                    value.clone()
                }
            }

            // Convert string numbers to actual numbers where appropriate
            Value::String(s) => {
                if let Ok(int_val) = s.parse::<i64>() {
                    Value::Number(serde_json::Number::from(int_val))
                } else if let Ok(float_val) = s.parse::<f64>() {
                    if let Some(number) = serde_json::Number::from_f64(float_val) {
                        Value::Number(number)
                    } else {
                        // Invalid float, keep as string
                        value.clone()
                    }
                } else {
                    value.clone()
                }
            }

            // Recursively convert nested objects and arrays
            Value::Object(nested_map) => Value::Object(convert_object_values(nested_map)?),

            Value::Array(nested_array) => Value::Array(convert_array_values(nested_array)?),

            _ => value.clone(),
        };

        converted_map.insert(key.clone(), converted_value);
    }

    Ok(converted_map)
}

/// Convert array values with element type coercion
fn convert_array_values(arr: &[Value]) -> Result<Vec<Value>> {
    let mut converted_array = Vec::new();

    for value in arr {
        let converted_value = match value {
            // Convert float64 to uint8 for byte arrays
            Value::Number(n) if n.is_f64() => {
                if let Some(f_val) = n.as_f64() {
                    if (0.0..=255.0).contains(&f_val) && f_val.fract() == 0.0 {
                        Value::Number(serde_json::Number::from(f_val as u8))
                    } else {
                        value.clone()
                    }
                } else {
                    value.clone()
                }
            }

            // Recursively convert nested structures
            Value::Object(nested_map) => Value::Object(convert_object_values(nested_map)?),

            Value::Array(nested_array) => Value::Array(convert_array_values(nested_array)?),

            _ => value.clone(),
        };

        converted_array.push(converted_value);
    }

    Ok(converted_array)
}

/// Convert scalar values with type coercion
fn convert_scalar_value(value: &Value) -> Result<Value> {
    match value {
        Value::Number(n) if n.is_f64() => {
            if let Some(f_val) = n.as_f64() {
                // Convert float to int if it's a whole number and within reasonable range
                if f_val.fract() == 0.0 && f_val >= i32::MIN as f64 && f_val <= i32::MAX as f64 {
                    Ok(Value::Number(serde_json::Number::from(f_val as i32)))
                } else {
                    Ok(value.clone())
                }
            } else {
                Ok(value.clone())
            }
        }

        Value::String(s) => {
            // Try to convert string numbers to actual numbers
            if let Ok(int_val) = s.parse::<i64>() {
                Ok(Value::Number(serde_json::Number::from(int_val)))
            } else if let Ok(float_val) = s.parse::<f64>() {
                if let Some(number) = serde_json::Number::from_f64(float_val) {
                    Ok(Value::Number(number))
                } else {
                    // Invalid float, keep as string
                    Ok(value.clone())
                }
            } else {
                Ok(value.clone())
            }
        }

        _ => Ok(value.clone()),
    }
}

/// RPC handler registry
pub struct RpcRegistry {
    handlers: HashMap<String, Arc<dyn RpcHandler>>,
}

impl RpcRegistry {
    /// Create new registry
    pub fn new() -> Self {
        Self { handlers: HashMap::new() }
    }

    /// Register RPC handler
    pub fn register<H>(&mut self, method: &str, handler: H)
    where
        H: RpcHandler + 'static,
    {
        self.handlers.insert(method.to_string(), Arc::new(handler));
    }

    /// Register function handler
    pub fn register_function<F>(&mut self, method: &str, func: F)
    where
        F: Fn(Option<Value>) -> Result<Value> + Send + Sync + 'static,
    {
        self.register(method, FunctionHandler::new(func));
    }

    /// Register typed handler
    pub fn register_typed<P, R, F>(&mut self, method: &str, func: F)
    where
        P: DeserializeOwned + Send + Sync + 'static,
        R: Serialize + Send + Sync + 'static,
        F: Fn(P) -> Result<R> + Send + Sync + 'static,
    {
        self.register(method, TypedHandler::new(func));
    }

    /// Register no-parameters handler
    pub fn register_no_params<R, F>(&mut self, method: &str, func: F)
    where
        R: Serialize + Send + Sync + 'static,
        F: Fn() -> Result<R> + Send + Sync + 'static,
    {
        self.register(method, NoParamsHandler::new(func));
    }

    /// Get handler
    pub fn get_handler(&self, method: &str) -> Option<&Arc<dyn RpcHandler>> {
        self.handlers.get(method)
    }

    /// List all registered methods
    pub fn list_methods(&self) -> Vec<&String> {
        self.handlers.keys().collect()
    }

    /// Register asynchronous handler
    pub fn register_async<F>(&mut self, method: &str, func: F)
    where
        F: Fn(Option<Value>) -> BoxFuture<'static, Result<Value>> + Send + Sync + 'static,
    {
        self.register(method, AsyncHandler::new(func));
    }
}

impl Default for RpcRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// JSON-RPC processor
pub struct JsonRpcProcessor {
    registry: Arc<RpcRegistry>,
}

impl JsonRpcProcessor {
    /// Create new processor
    pub fn new(registry: Arc<RpcRegistry>) -> Self {
        Self { registry }
    }

    /// Handle JSON-RPC message
    pub async fn handle_message(&self, message: DataChannelMessage, session: &Session) {
        let message_str = match String::from_utf8(message.data.to_vec()) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to parse message as UTF-8: {}", e);
                let error_response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError::parse_error()),
                    id: None,
                };
                if let Err(e) = self.send_response(&error_response, session).await {
                    error!("Failed to send error response: {}", e);
                }
                return;
            }
        };

        let request: JsonRpcRequest = match serde_json::from_str(&message_str) {
            Ok(req) => req,
            Err(e) => {
                warn!("Failed to parse JSON-RPC request: {}", e);
                let error_response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError::parse_error()),
                    id: None,
                };
                if let Err(e) = self.send_response(&error_response, session).await {
                    error!("Failed to send error response: {}", e);
                }
                return;
            }
        };

        debug!("Received RPC request: method={}, id={:?}", request.method, request.id);

        // Get handler
        let handler = match self.registry.get_handler(&request.method) {
            Some(h) => h,
            None => {
                warn!("Method not found: {}", request.method);
                let error_response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError::method_not_found()),
                    id: request.id,
                };
                if let Err(e) = self.send_response(&error_response, session).await {
                    error!("Failed to send error response: {}", e);
                }
                return;
            }
        };

        // Call handler
        match handler.call_async(request.params).await {
            Ok(result) => {
                let response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: Some(result),
                    error: None,
                    id: request.id,
                };
                if let Err(e) = self.send_response(&response, session).await {
                    error!("Failed to send response: {}", e);
                }
            }
            Err(e) => {
                error!("RPC handler error: {}", e);
                let error_response = JsonRpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(JsonRpcError::internal_error(Some(e.to_string()))),
                    id: request.id,
                };
                if let Err(e) = self.send_response(&error_response, session).await {
                    error!("Failed to send error response: {}", e);
                }
            }
        }
    }

    /// Send response via RPC data channel
    async fn send_response(&self, response: &JsonRpcResponse, session: &Session) -> Result<()> {
        let response_json = serde_json::to_string(response)?;
        debug!("Sending JSON-RPC response: {}", response_json);

        if let Some(rpc_channel) = &session.rpc_channel {
            if let Err(e) = rpc_channel.send_text(response_json).await {
                return Err(anyhow!("Failed to send RPC response: {}", e));
            }
        } else {
            warn!("No RPC channel available for session: {}", session.id);
        }

        Ok(())
    }

    /// Send event (notification) via RPC data channel
    pub async fn send_event(
        &self,
        method: &str,
        params: Option<Value>,
        session: &Session,
    ) -> Result<()> {
        let event_json = serde_json::to_string(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))?;
        info!("Sending JSON-RPC event: method={}, data={}", method, event_json);

        if let Some(rpc_channel) = &session.rpc_channel {
            if let Err(e) = rpc_channel.send_text(event_json).await {
                return Err(anyhow!("Failed to send RPC event: {}", e));
            }
        } else {
            warn!("No RPC channel available for session: {}", session.id);
        }

        Ok(())
    }
}

/// Default RPC handler implementations
pub mod handlers {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

    use parking_lot::{Mutex, RwLock};
    use tokio::process::Command;
    use tokio::time::{Duration, timeout};

    use super::*;
    use crate::cloud::CloudManager;
    use crate::hardware::usb as usb_mod;
    use crate::hardware::usb::storage as storage_mod;

    static USB_STATE: once_cell::sync::OnceCell<RwLock<String>> = once_cell::sync::OnceCell::new();
    static KEYBOARD_LED: once_cell::sync::OnceCell<RwLock<HidKeyboardState>> =
        once_cell::sync::OnceCell::new();
    static CLOUD_STATE: once_cell::sync::OnceCell<RwLock<CloudConnectionState>> =
        once_cell::sync::OnceCell::new();
    static NETWORK_IP: once_cell::sync::OnceCell<RwLock<String>> = once_cell::sync::OnceCell::new();

    static STREAM_QUALITY_FACTOR: AtomicI32 = AtomicI32::new(100); // Store as percentage
    static AUTO_UPDATE_ENABLED: AtomicBool = AtomicBool::new(false);
    static DISPLAY_ROTATION: OnceLock<Mutex<String>> = OnceLock::new();
    static KEYBOARD_LAYOUT: OnceLock<Mutex<String>> = OnceLock::new();

    /// Ping handler
    pub fn ping() -> Result<String> {
        Ok("pong".to_string())
    }
    // USB state
    pub fn get_usb_state() -> Result<String> {
        let cell = USB_STATE.get_or_init(|| parking_lot::RwLock::new("unknown".to_string()));
        Ok(cell.read().clone())
    }

    pub fn set_usb_state(state: String) -> Result<Value> {
        let cell = USB_STATE.get_or_init(|| parking_lot::RwLock::new("unknown".to_string()));
        *cell.write() = state;
        Ok(Value::Null)
    }

    // Keyboard LED state
    pub fn get_keyboard_led_state() -> Result<HidKeyboardState> {
        let cell =
            KEYBOARD_LED.get_or_init(|| parking_lot::RwLock::new(HidKeyboardState::default()));
        Ok(*cell.read())
    }

    pub fn set_keyboard_led_state(state: HidKeyboardState) -> Result<Value> {
        let cell =
            KEYBOARD_LED.get_or_init(|| parking_lot::RwLock::new(HidKeyboardState::default()));
        *cell.write() = state;
        Ok(Value::Null)
    }

    // Cloud connection state management
    pub async fn get_cloud_state() -> Result<CloudState> {
        // Get real-time state from CloudManager
        let cloud_manager = CloudManager::new();
        Ok(cloud_manager.get_cloud_state().await)
    }

    pub async fn deregister_device() -> Result<Value> {
        // Call CloudManager to deregister device
        let cloud_manager = CloudManager::new();
        cloud_manager.deregister_device().await?;
        Ok(Value::Null)
    }

    /// Reset configuration to defaults and persist
    pub async fn reset_config() -> Result<Value> {
        let config_manager = get_config_manager();
        config_manager
            .update(|cfg| {
                *cfg = crate::config::types::Config::default();
            })
            .await?;
        info!("Configuration reset to default and saved");
        Ok(Value::Null)
    }

    #[derive(Deserialize)]
    pub struct CloudStateParam {
        pub state: String,
    }

    pub fn set_cloud_state(param: CloudStateParam) -> Result<Value> {
        let new_state = param
            .state
            .parse::<CloudConnectionState>()
            .map_err(|e| anyhow!("invalid cloud state: {}", e))?;
        let cell = CLOUD_STATE.get_or_init(|| RwLock::new(CloudConnectionState::NotConfigured));
        *cell.write() = new_state;
        Ok(Value::Null)
    }

    // Cloud URL configuration
    #[derive(Deserialize)]
    pub struct CloudUrlParams {
        #[serde(rename = "apiUrl")]
        pub api_url: String,
        #[serde(rename = "appUrl")]
        pub app_url: String,
    }

    pub async fn set_cloud_url(params: CloudUrlParams) -> Result<Value> {
        let cloud_manager = CloudManager::new();
        cloud_manager.set_cloud_url(&params.api_url, &params.app_url).await?;
        Ok(Value::Null)
    }

    // Network IPv4 address (skeleton)
    pub fn get_network_ip_address() -> Result<String> {
        let cell = NETWORK_IP.get_or_init(|| RwLock::new(String::new()));
        Ok(cell.read().clone())
    }

    #[derive(Deserialize)]
    pub struct NetworkIpParam {
        pub ip: String,
    }

    pub fn set_network_ip_address(param: NetworkIpParam) -> Result<Value> {
        let cell = NETWORK_IP.get_or_init(|| RwLock::new(String::new()));
        *cell.write() = param.ip;
        Ok(Value::Null)
    }

    // Network state management
    #[derive(Serialize)]
    pub struct NetworkStateResponse {
        pub online: bool,
        pub ip: String,
        pub gateway: Option<String>,
        pub dns: Vec<String>,
    }

    pub fn get_network_state() -> Result<NetworkStateResponse> {
        // TODO: Implement real network state detection
        let ip = get_network_ip_address().unwrap_or_default();
        Ok(NetworkStateResponse { online: !ip.is_empty(), ip, gateway: None, dns: vec![] })
    }

    #[derive(Debug, Serialize, Deserialize)]
    pub struct NetworkSettingsResponse {
        pub dhcp_enabled: bool,
        pub static_ip: Option<String>,
        pub gateway: Option<String>,
        pub dns: Vec<String>,
    }

    pub fn get_network_settings() -> Result<NetworkSettingsResponse> {
        // TODO: Implement real network settings retrieval
        Ok(NetworkSettingsResponse {
            dhcp_enabled: true,
            static_ip: None,
            gateway: None,
            dns: vec![],
        })
    }

    #[derive(Deserialize)]
    pub struct NetworkSettingsParams {
        pub settings: NetworkSettingsResponse,
    }

    pub fn set_network_settings(params: NetworkSettingsParams) -> Result<Value> {
        // TODO: Implement real network settings configuration
        info!("Network settings updated: {:?}", params.settings);
        Ok(Value::Null)
    }

    pub fn renew_dhcp_lease() -> Result<Value> {
        // TODO: Implement DHCP lease renewal
        info!("DHCP lease renewal requested");
        Ok(Value::Null)
    }

    // Wake-on-LAN management
    pub async fn get_wake_on_lan_devices() -> Result<Vec<crate::config::types::WakeOnLanDevice>> {
        let mgr = get_config_manager();
        let cfg = mgr.get().await;
        Ok(cfg.wake_on_lan_devices)
    }

    #[derive(Deserialize)]
    pub struct SetWakeOnLanDevicesParams {
        pub devices: Vec<crate::config::types::WakeOnLanDevice>,
    }

    pub async fn set_wake_on_lan_devices(params: SetWakeOnLanDevicesParams) -> Result<Value> {
        let mgr = get_config_manager();
        let devices = params.devices;
        mgr.update(move |cfg| {
            cfg.wake_on_lan_devices = devices;
        })
        .await?;
        info!("Wake-on-LAN devices updated");
        Ok(Value::Null)
    }

    #[derive(Deserialize)]
    pub struct SendWOLMagicPacketParams {
        #[serde(rename = "macAddress")]
        pub mac_address: String,
    }

    pub async fn send_wol_magic_packet(params: SendWOLMagicPacketParams) -> Result<Value> {
        crate::wol::send_magic_packet(&params.mac_address).await?;
        Ok(Value::Null)
    }

    // Composite display state for client convenience
    #[derive(Serialize)]
    pub struct DisplayState {
        pub ip: String,
        pub usb_connected: bool,
        pub cloud_state: String,
        pub keyboard_led: HidKeyboardState,
    }

    pub fn get_display_state() -> Result<DisplayState> {
        let ip = get_network_ip_address().unwrap_or_default();
        let usb_connected = get_usb_state().unwrap_or_default() == "configured";
        let cloud_manager = CloudManager::new();
        let cloud_state = cloud_manager.get_state().as_str().to_string();
        let keyboard_led = get_keyboard_led_state().unwrap_or_default();
        Ok(DisplayState { ip, usb_connected, cloud_state, keyboard_led })
    }

    // ---- USB HID input handlers (keyboard/mouse) ----
    #[derive(Deserialize)]
    pub struct KeyboardReportParams {
        pub modifier: u8,
        pub keys: Vec<u8>,
    }

    pub fn keyboard_report(params: KeyboardReportParams) -> Result<Value> {
        let mgr =
            usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
        mgr.read()
            .hid()
            .keyboard_report(params.modifier, &params.keys)
            .map_err(|e| anyhow!("keyboard report failed: {}", e))?;
        Ok(Value::Null)
    }

    #[derive(Deserialize)]
    pub struct AbsMouseReportParams {
        pub x: i32,
        pub y: i32,
        pub buttons: u8,
    }

    pub fn abs_mouse_report(params: AbsMouseReportParams) -> Result<Value> {
        let mgr =
            usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
        mgr.read()
            .hid()
            .abs_mouse_report(params.x, params.y, params.buttons)
            .map_err(|e| anyhow!("abs mouse report failed: {}", e))?;
        Ok(Value::Null)
    }

    #[derive(Deserialize)]
    pub struct RelMouseReportParams {
        pub dx: i8,
        pub dy: i8,
        pub buttons: u8,
    }

    pub fn rel_mouse_report(params: RelMouseReportParams) -> Result<Value> {
        let mgr =
            usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
        mgr.read()
            .hid()
            .rel_mouse_report(params.dx, params.dy, params.buttons)
            .map_err(|e| anyhow!("rel mouse report failed: {}", e))?;
        Ok(Value::Null)
    }

    #[derive(Deserialize)]
    pub struct WheelReportParams {
        #[serde(rename = "wheelY")]
        pub wheel_y: i8,
    }

    pub fn wheel_report(params: WheelReportParams) -> Result<Value> {
        let mgr =
            usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
        mgr.read()
            .hid()
            .abs_mouse_wheel_report(params.wheel_y)
            .map_err(|e| anyhow!("wheel report failed: {}", e))?;
        Ok(Value::Null)
    }

    /// Get device ID
    pub fn get_device_id() -> Result<String> {
        Ok(crate::hardware::hw::get_device_id())
    }

    /// Reboot system
    #[derive(Deserialize)]
    pub struct RebootParams {
        #[serde(default)]
        pub force: bool,
    }

    pub fn reboot(params: RebootParams) -> Result<Value> {
        info!("Reboot requested, force: {}", params.force);

        let mut cmd = std::process::Command::new("reboot");
        if params.force {
            cmd.arg("-f");
        }

        match cmd.spawn() {
            Ok(_) => {
                tokio::spawn(async {
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                    std::process::exit(0);
                });
                Ok(Value::Null)
            }
            Err(e) => Err(anyhow!("Failed to execute reboot command: {}", e)),
        }
    }

    /// Serial port configuration
    #[derive(Deserialize)]
    pub struct SerialSettings {
        pub baud_rate: String,
        pub data_bits: String,
        pub stop_bits: String,
        pub parity: String,
    }

    #[derive(Serialize)]
    pub struct SerialSettingsResponse {
        pub baud_rate: String,
        pub data_bits: String,
        pub stop_bits: String,
        pub parity: String,
    }

    pub fn set_serial_settings(settings: SerialSettings) -> Result<Value> {
        // Validate baud rate
        let _baud_rate: u32 = settings
            .baud_rate
            .parse()
            .map_err(|_| anyhow!("Invalid baud rate: {}", settings.baud_rate))?;

        // Validate data bits
        let data_bits: u8 = settings
            .data_bits
            .parse()
            .map_err(|_| anyhow!("Invalid data bits: {}", settings.data_bits))?;
        if !(5..=8).contains(&data_bits) {
            return Err(anyhow!("Data bits must be between 5 and 8"));
        }

        // Validate stop bits
        match settings.stop_bits.as_str() {
            "1" | "1.5" | "2" => {}
            _ => return Err(anyhow!("Invalid stop bits: {}", settings.stop_bits)),
        }

        // Validate parity
        match settings.parity.as_str() {
            "none" | "odd" | "even" | "mark" | "space" => {}
            _ => return Err(anyhow!("Invalid parity: {}", settings.parity)),
        }

        info!("Serial settings updated successfully");
        Ok(Value::Null)
    }

    pub fn get_serial_settings() -> Result<SerialSettingsResponse> {
        Ok(SerialSettingsResponse {
            baud_rate: "115200".to_string(),
            data_bits: "8".to_string(),
            stop_bits: "1".to_string(),
            parity: "none".to_string(),
        })
    }

    /// Display rotation
    #[derive(Deserialize)]
    pub struct DisplayRotationParams {
        pub rotation: String,
    }

    #[derive(Serialize)]
    pub struct DisplayRotationResponse {
        pub rotation: String,
    }

    pub fn set_display_rotation(params: DisplayRotationParams) -> Result<Value> {
        match params.rotation.as_str() {
            "0" | "90" | "180" | "270" => {
                let rotation_mutex = DISPLAY_ROTATION.get_or_init(|| Mutex::new("0".to_string()));
                *rotation_mutex.lock() = params.rotation.clone();
                info!("Display rotation set to: {}", params.rotation);
                Ok(Value::Null)
            }
            _ => Err(anyhow!("Invalid rotation value: {}", params.rotation)),
        }
    }

    pub fn get_display_rotation() -> Result<DisplayRotationResponse> {
        let rotation_mutex = DISPLAY_ROTATION.get_or_init(|| Mutex::new("0".to_string()));
        let rotation = rotation_mutex.lock().clone();
        Ok(DisplayRotationResponse { rotation })
    }

    /// Backlight settings
    #[derive(Deserialize)]
    pub struct BacklightSettings {
        pub max_brightness: i32,
        pub dim_after: i32,
        pub off_after: i32,
    }

    #[derive(Serialize)]
    pub struct BacklightSettingsResponse {
        pub max_brightness: i32,
        pub dim_after: i32,
        pub off_after: i32,
    }

    pub fn set_backlight_settings(settings: BacklightSettings) -> Result<Value> {
        if !(0..=255).contains(&settings.max_brightness) {
            return Err(anyhow!("max_brightness must be between 0 and 255"));
        }
        if settings.dim_after < 0 {
            return Err(anyhow!("dim_after must be a positive integer"));
        }
        if settings.off_after < 0 {
            return Err(anyhow!("off_after must be a positive integer"));
        }

        // Apply backlight settings
        if let Err(e) = std::fs::write(
            "/sys/class/backlight/backlight/brightness",
            settings.max_brightness.to_string(),
        ) {
            warn!("Failed to set brightness: {}", e);
        }

        info!(
            "Backlight settings applied: brightness={}, dim_after={}, off_after={}",
            settings.max_brightness, settings.dim_after, settings.off_after
        );
        Ok(Value::Null)
    }

    pub fn get_backlight_settings() -> Result<BacklightSettingsResponse> {
        let max_brightness =
            std::fs::read_to_string("/sys/class/backlight/backlight/max_brightness")
                .and_then(|s| {
                    s.trim()
                        .parse()
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
                })
                .unwrap_or(255);

        Ok(BacklightSettingsResponse { max_brightness, dim_after: 300, off_after: 600 })
    }

    /// Stream quality
    pub fn get_stream_quality_factor() -> Result<f64> {
        let factor = STREAM_QUALITY_FACTOR.load(Ordering::Relaxed) as f64 / 100.0;
        Ok(factor)
    }

    #[derive(Deserialize)]
    pub struct StreamQualityParams {
        pub factor: f64,
    }

    pub fn set_stream_quality_factor(params: StreamQualityParams) -> Result<Value> {
        if !(0.0..=1.0).contains(&params.factor) {
            return Err(anyhow!("Quality factor must be between 0.0 and 1.0"));
        }

        let percentage = (params.factor * 100.0) as i32;
        STREAM_QUALITY_FACTOR.store(percentage, Ordering::Relaxed);

        // Spawn async task to update video quality
        let factor = params.factor as f32;
        tokio::spawn(async move {
            if let Err(e) = crate::video::update_video_quality(factor).await {
                warn!("Failed to update video quality: {}", e);
            }
        });

        info!("Stream quality factor set to: {}", params.factor);
        Ok(Value::Null)
    }

    /// Auto update
    pub fn get_auto_update_state() -> Result<bool> {
        Ok(AUTO_UPDATE_ENABLED.load(Ordering::Relaxed))
    }

    #[derive(Deserialize)]
    pub struct AutoUpdateParams {
        pub enabled: bool,
    }

    pub fn set_auto_update_state(params: AutoUpdateParams) -> Result<bool> {
        AUTO_UPDATE_ENABLED.store(params.enabled, Ordering::Relaxed);
        info!("Auto update state set to: {}", params.enabled);
        Ok(params.enabled)
    }

    pub async fn get_video_state() -> Result<crate::video::VideoInputState> {
        Ok(crate::video::get_video_state().await)
    }

    #[derive(Serialize)]
    pub struct UpdateStatusResponse {
        #[serde(rename = "updateAvailable")]
        pub update_available: bool,
        #[serde(rename = "currentVersion")]
        pub current_version: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub error: Option<String>,
    }

    pub fn get_update_status() -> Result<UpdateStatusResponse> {
        let current_version = env!("CARGO_PKG_VERSION").to_string();
        Ok(UpdateStatusResponse { update_available: false, current_version, error: None })
    }

    #[derive(Deserialize)]
    pub struct KeyboardMacrosParams {
        pub macros: Vec<serde_json::Value>,
    }

    pub async fn get_keyboard_macros() -> Result<Vec<crate::config::KeyboardMacro>> {
        let mgr = get_config_manager();
        let cfg = mgr.get().await;
        Ok(cfg.keyboard_macros)
    }

    pub async fn set_keyboard_macros(params: KeyboardMacrosParams) -> Result<serde_json::Value> {
        // Validate macro count limit
        if params.macros.len() > crate::config::types::MAX_MACROS_PER_DEVICE {
            anyhow::bail!("too many macros (max {})", crate::config::types::MAX_MACROS_PER_DEVICE);
        }

        let mut new_macros = Vec::with_capacity(params.macros.len());
        for (i, macro_value) in params.macros.into_iter().enumerate() {
            let macro_obj: serde_json::Map<String, serde_json::Value> =
                serde_json::from_value(macro_value)
                    .map_err(|e| anyhow::anyhow!("invalid macro at index {}: {}", i, e))?;

            // Extract and validate fields
            let id =
                macro_obj.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()).unwrap_or_else(
                    || {
                        format!(
                            "macro-{}",
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_nanos())
                                .unwrap_or(0)
                        )
                    },
                );

            let name = macro_obj.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();

            let sort_order = macro_obj
                .get("sortOrder")
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or((i + 1) as u32);

            // Parse steps
            let mut steps = Vec::new();
            if let Some(steps_array) = macro_obj.get("steps").and_then(|v| v.as_array()) {
                if steps_array.is_empty() {
                    anyhow::bail!("macro at index {} must have at least one step", i);
                }
                for step_value in steps_array.iter() {
                    let step_obj = match step_value.as_object() {
                        Some(obj) => obj,
                        None => continue,
                    };

                    let mut step = crate::config::KeyboardMacroStep {
                        keys: Vec::new(),
                        modifiers: Vec::new(),
                        delay: 0,
                    };

                    // Parse keys
                    if let Some(keys_array) = step_obj.get("keys").and_then(|v| v.as_array()) {
                        for key_value in keys_array {
                            if let Some(key_str) = key_value.as_str() {
                                step.keys.push(key_str.to_string());
                            }
                        }
                    }

                    // Parse modifiers
                    if let Some(mods_array) = step_obj.get("modifiers").and_then(|v| v.as_array()) {
                        for mod_value in mods_array {
                            if let Some(mod_str) = mod_value.as_str() {
                                step.modifiers.push(mod_str.to_string());
                            }
                        }
                    }

                    // Parse delay
                    if let Some(delay_value) = step_obj.get("delay").and_then(|v| v.as_u64()) {
                        step.delay = delay_value as u32;
                    }

                    steps.push(step);
                }
            }

            let mut macro_item =
                crate::config::KeyboardMacro { id, name, steps, sort_order: Some(sort_order) };

            // Validate macro
            if let Err(e) = macro_item.validate() {
                anyhow::bail!("invalid macro at index {}: {}", i, e);
            }

            new_macros.push(macro_item);
        }

        // Update configuration
        let mgr = get_config_manager();
        mgr.update(|cfg| {
            cfg.keyboard_macros = new_macros;
        })
        .await?;

        Ok(serde_json::Value::Null)
    }

    pub fn get_edid() -> Result<String> {
        let edid_bytes = edid::read_edid()?;
        Ok(hex::encode(&edid_bytes))
    }

    pub fn set_edid(params: serde_json::Value) -> Result<Value> {
        let edid_str = params.get("edid").and_then(|v| v.as_str()).unwrap_or("");

        let edid_hex = if edid_str.is_empty() {
            info!("Restoring EDID to default");
            "00ffffffffffff0031d8341200000000221a010380301b780fee91a3544c99260f50542fcf00315945598180814090409500a940b300023a801871382d40582c4500e00e1100001e000000fd001855185e11000a202020202020000000fc0068646d690a202020202020202000000010000000000000000000000000000001c402032df04c101f04132221200514021101230907078301000068030c001000002201e200eae3050000e30601001a3680a070381f4030203500e00e1100001a1a1d008051d01c2040803500e00e1100001c000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000007d"
        } else {
            info!("Setting EDID: {}", edid_str);
            edid_str
        };

        if edid_hex.len() > 512 {
            return Err(anyhow!("invalid edid"));
        }

        let edid_bytes = hex::decode(edid_hex).map_err(|_| anyhow!("Invalid EDID format"))?;
        edid::write_edid(&edid_bytes)?;

        // Save EDID to config, allowing it to be restored on reboot
        let config_manager = get_config_manager();
        let edid_hex_to_save = edid_hex.to_ascii_lowercase();
        tokio::spawn(async move {
            if let Err(e) = config_manager
                .update(|config| {
                    config.edid_string = Some(edid_hex_to_save);
                })
                .await
            {
                warn!("Failed to save EDID to config: {}", e);
            }
        });

        info!("EDID configuration updated");
        Ok(Value::Null)
    }

    /// USB device management
    #[derive(Serialize)]
    pub struct UsbDevicesResponse {
        pub absolute_mouse: bool,
        pub relative_mouse: bool,
        pub keyboard: bool,
        pub mass_storage: bool,
    }

    pub fn get_usb_devices() -> Result<UsbDevicesResponse> {
        Ok(UsbDevicesResponse {
            absolute_mouse: true,
            relative_mouse: false,
            keyboard: true,
            mass_storage: true,
        })
    }

    #[derive(Deserialize)]
    pub struct UsbDeviceStateParams {
        pub device: String,
        pub enabled: bool,
    }

    pub fn set_usb_device_state(params: UsbDeviceStateParams) -> Result<Value> {
        match params.device.as_str() {
            "absoluteMouse" | "relativeMouse" | "keyboard" | "massStorage" => {
                info!("USB device {} state set to: {}", params.device, params.enabled);
                Ok(Value::Null)
            }
            _ => Err(anyhow!("Invalid USB device: {}", params.device)),
        }
    }

    /// USB emulation (bound/configured). Uses UsbManager state if available.
    pub fn get_usb_emulation_state() -> Result<bool> {
        // Bound to UDC by checking dwc3 driver path with current UDC name
        if let Some(mgr) = usb_mod::get_usb_manager() {
            let mgr_guard = mgr.read();
            let udc = mgr_guard.get_udc_name();
            let path = format!("/sys/bus/platform/drivers/dwc3/{}", udc);
            return Ok(std::path::Path::new(&path).exists());
        }
        Ok(false)
    }

    #[derive(Deserialize)]
    pub struct UsbEmulationParams {
        pub enabled: bool,
    }

    pub fn set_usb_emulation_state(params: UsbEmulationParams) -> Result<Value> {
        let mgr =
            usb_mod::get_usb_manager().ok_or_else(|| anyhow!("USB manager not initialized"))?;
        let mgr_guard = mgr.read();
        let udc = mgr_guard.get_udc_name().to_string();
        let bind_path = "/sys/bus/platform/drivers/dwc3/bind";
        let unbind_path = "/sys/bus/platform/drivers/dwc3/unbind";
        if params.enabled {
            std::fs::write(bind_path, &udc).map_err(|e| anyhow!("error binding UDC: {}", e))?;
        } else {
            std::fs::write(unbind_path, &udc).map_err(|e| anyhow!("error unbinding UDC: {}", e))?;
        }
        info!("USB emulation state set to: {}", params.enabled);
        Ok(Value::Null)
    }

    /// Keyboard layout
    pub fn get_keyboard_layout() -> Result<String> {
        let layout_mutex = KEYBOARD_LAYOUT.get_or_init(|| Mutex::new("us".to_string()));
        let layout = layout_mutex.lock().clone();
        Ok(layout)
    }

    #[derive(Deserialize)]
    pub struct KeyboardLayoutParams {
        pub layout: String,
    }

    pub fn set_keyboard_layout(params: KeyboardLayoutParams) -> Result<Value> {
        // Validate layout
        const VALID_LAYOUTS: &[&str] = &["us", "uk", "de", "fr", "es", "it", "jp", "kr"];
        if !VALID_LAYOUTS.contains(&params.layout.as_str()) {
            return Err(anyhow!("Unsupported keyboard layout: {}", params.layout));
        }

        let layout_mutex = KEYBOARD_LAYOUT.get_or_init(|| Mutex::new("us".to_string()));
        *layout_mutex.lock() = params.layout.clone();

        info!("Keyboard layout set to: {}", params.layout);
        Ok(Value::Null)
    }

    /// Network settings
    pub async fn get_local_loopback_only() -> Result<bool> {
        let config_manager = crate::config::get_config_manager();
        let config = config_manager.get().await;
        Ok(config.local_loopback_only)
    }

    #[derive(Deserialize)]
    pub struct LocalLoopbackParams {
        pub enabled: bool,
    }

    pub async fn set_local_loopback_only(params: LocalLoopbackParams) -> Result<Value> {
        let config_manager = crate::config::get_config_manager();

        config_manager
            .update(|config| {
                config.local_loopback_only = params.enabled;
            })
            .await?;

        let new_state = config_manager.get().await.local_loopback_only;

        info!("Local loopback only mode set to: {}", new_state);
        Ok(serde_json::to_value(new_state)?)
    }

    // =====================
    // Developer mode
    // =====================
    #[derive(Deserialize)]
    pub struct DevModeParams {
        pub enabled: bool,
    }

    /// Return developer mode state
    pub async fn get_dev_mode_state_handler() -> Result<crate::config::DevModeState> {
        let state = crate::config::get_dev_mode_state().await?;
        Ok(state)
    }

    /// Set developer mode state and restart/stop SSH via dropbear.sh
    pub async fn set_dev_mode_state_handler(params: DevModeParams) -> Result<Value> {
        crate::config::set_dev_mode_state(params.enabled).await?;

        // Try to start/stop SSH (best-effort)
        tokio::spawn(async {
            let run = async { Command::new("/oem/usr/bin/dropbear.sh").arg("auto").output().await };
            match timeout(Duration::from_secs(2), run).await {
                Ok(Ok(output)) => {
                    if !output.status.success() {
                        warn!(
                            "dropbear.sh exited non-zero: status={:?} stderr={}",
                            output.status.code(),
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                }
                Ok(Err(e)) => {
                    warn!("dropbear.sh exec error: {}", e);
                }
                Err(_) => {
                    warn!("dropbear.sh timed out");
                }
            }
        });

        Ok(Value::Null)
    }

    // =====================
    // SSH key management
    // =====================
    const SSH_KEY_DIR: &str = "/userdata/dropbear/.ssh";
    const SSH_KEY_FILE: &str = "/userdata/dropbear/.ssh/authorized_keys";

    /// Read authorized_keys content. Returns empty string if file does not exist.
    pub fn get_ssh_key_state() -> Result<String> {
        match std::fs::read_to_string(SSH_KEY_FILE) {
            Ok(s) => Ok(s),
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    Ok(String::new())
                } else {
                    Err(anyhow!("error reading SSH key file: {}", e))
                }
            }
        }
    }

    #[derive(Deserialize)]
    pub struct SshKeyParam {
        #[serde(rename = "sshKey")]
        pub ssh_key: String,
    }

    /// Write or remove authorized_keys based on input. When non-empty, ensure dir perms 0700 and file perms 0600.
    pub fn set_ssh_key_state(param: SshKeyParam) -> Result<Value> {
        let ssh_key = param.ssh_key;
        if !ssh_key.is_empty() {
            // Ensure directory exists with 0700
            std::fs::create_dir_all(SSH_KEY_DIR)
                .map_err(|e| anyhow!("failed to create SSH key directory: {}", e))?;
            let dir_meta = std::fs::metadata(SSH_KEY_DIR)
                .map_err(|e| anyhow!("failed to stat SSH key directory: {}", e))?;
            let mut dir_perm = dir_meta.permissions();
            dir_perm.set_mode(0o700);
            std::fs::set_permissions(SSH_KEY_DIR, dir_perm)
                .map_err(|e| anyhow!("failed to set SSH key directory permissions: {}", e))?;

            // Write file (truncate) then set 0600
            {
                use std::io::Write;
                let mut f = std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(SSH_KEY_FILE)
                    .map_err(|e| anyhow!("failed to write SSH key: {}", e))?;
                f.write_all(ssh_key.as_bytes())
                    .map_err(|e| anyhow!("failed to write SSH key: {}", e))?;
            }
            let meta = std::fs::metadata(SSH_KEY_FILE)
                .map_err(|e| anyhow!("failed to stat SSH key file: {}", e))?;
            let mut perm = meta.permissions();
            perm.set_mode(0o600);
            std::fs::set_permissions(SSH_KEY_FILE, perm)
                .map_err(|e| anyhow!("failed to set SSH key permissions: {}", e))?;
        } else {
            // Remove file when empty string
            if let Err(e) = std::fs::remove_file(SSH_KEY_FILE)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                return Err(anyhow!("failed to remove SSH key file: {}", e));
            }
        }

        Ok(Value::Null)
    }

    // =====================
    // Virtual media (storage)
    // =====================

    #[derive(Serialize)]
    pub struct VirtualMediaStateResponse {
        pub source: String,
        pub mode: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub filename: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub url: Option<String>,
        pub size: i64,
    }

    pub fn get_virtual_media_state() -> Result<Option<VirtualMediaStateResponse>> {
        let s = storage_mod::get_virtual_media_state();
        let mapped = s.map(|st| VirtualMediaStateResponse {
            source: match st.source {
                storage_mod::VirtualMediaSource::WebRTC => "WebRTC".into(),
                storage_mod::VirtualMediaSource::HTTP => "HTTP".into(),
                storage_mod::VirtualMediaSource::Storage => "Storage".into(),
            },
            mode: match st.mode {
                storage_mod::VirtualMediaMode::CDROM => "CDROM".into(),
                storage_mod::VirtualMediaMode::Disk => "Disk".into(),
            },
            filename: st.filename,
            url: st.url,
            size: st.size,
        });
        Ok(mapped)
    }

    fn parse_vm_mode(mode: &str) -> Result<storage_mod::VirtualMediaMode> {
        match mode.to_ascii_uppercase().as_str() {
            "CDROM" => Ok(storage_mod::VirtualMediaMode::CDROM),
            "DISK" => Ok(storage_mod::VirtualMediaMode::Disk),
            _ => Err(anyhow!("invalid mode")),
        }
    }

    #[derive(Deserialize)]
    pub struct MountHttpParams {
        pub url: String,
        pub mode: String,
    }

    pub async fn mount_with_http(params: MountHttpParams) -> Result<Value> {
        let mode = parse_vm_mode(&params.mode)?;
        storage_mod::mount_with_http(&params.url, mode).await?;
        Ok(Value::Null)
    }

    #[derive(Deserialize)]
    pub struct MountWebRtcParams {
        pub filename: String,
        pub size: i64,
        pub mode: String,
    }

    pub async fn mount_with_webrtc(params: MountWebRtcParams) -> Result<Value> {
        let mode = parse_vm_mode(&params.mode)?;
        storage_mod::mount_with_webrtc(&params.filename, params.size, mode).await?;
        Ok(Value::Null)
    }

    #[derive(Deserialize)]
    pub struct MountStorageParams {
        pub filename: String,
        pub mode: String,
    }

    pub async fn mount_with_storage(params: MountStorageParams) -> Result<Value> {
        let mode = parse_vm_mode(&params.mode)?;
        storage_mod::mount_with_storage(&params.filename, mode).await?;
        Ok(Value::Null)
    }

    pub async fn unmount_image() -> Result<Value> {
        storage_mod::unmount_image().await?;
        Ok(Value::Null)
    }

    // ---------- Storage files & uploads ----------

    #[derive(Serialize)]
    pub struct StorageSpaceResponse {
        #[serde(rename = "bytesUsed")]
        pub bytes_used: i64,
        #[serde(rename = "bytesFree")]
        pub bytes_free: i64,
    }

    #[derive(Serialize)]
    pub struct StorageFileInfo {
        pub filename: String,
        pub size: i64,
        #[serde(rename = "createdAt")]
        pub created_at: String,
        #[serde(rename = "totalBytes")]
        pub total_bytes: i64,
    }

    #[derive(Serialize)]
    pub struct StorageFilesResponse {
        pub files: Vec<StorageFileInfo>,
    }

    #[derive(Deserialize)]
    pub struct StartUploadParams {
        pub filename: String,
        pub size: i64,
    }

    #[derive(Serialize)]
    pub struct StartUploadResponse {
        #[serde(rename = "alreadyUploadedBytes")]
        pub already_uploaded_bytes: i64,
        #[serde(rename = "dataChannel")]
        pub data_channel: String,
    }

    pub async fn start_storage_file_upload(
        params: StartUploadParams,
    ) -> Result<StartUploadResponse> {
        let up = storage_mod::start_storage_file_upload(&params.filename, params.size).await?;
        Ok(StartUploadResponse {
            already_uploaded_bytes: up.already_uploaded_bytes,
            data_channel: up.data_channel,
        })
    }

    // List storage files
    pub async fn list_storage_files() -> Result<StorageFilesResponse> {
        let list = storage_mod::list_storage_files().await?;
        let files = list
            .files
            .into_iter()
            .map(|e| StorageFileInfo {
                filename: e.filename,
                size: e.size,
                created_at: e.created_at,
                total_bytes: e.total_bytes,
            })
            .collect();
        Ok(StorageFilesResponse { files })
    }

    #[derive(Deserialize)]
    pub struct DeleteStorageFileParams {
        pub filename: String,
    }

    pub async fn delete_storage_file(params: DeleteStorageFileParams) -> Result<Value> {
        storage_mod::delete_storage_file(&params.filename).await?;
        Ok(Value::Null)
    }

    // Disk space
    pub async fn get_storage_space() -> Result<StorageSpaceResponse> {
        let s = storage_mod::get_storage_space().await?;
        Ok(StorageSpaceResponse { bytes_used: s.bytes_used, bytes_free: s.bytes_free })
    }

    #[derive(Deserialize)]
    pub struct MountBuiltInImageParams {
        pub filename: String,
    }

    pub async fn mount_built_in_image(params: MountBuiltInImageParams) -> Result<Value> {
        storage_mod::mount_built_in_image(&params.filename).await?;
        Ok(Value::Null)
    }

    // ---- Mass storage mode (cdrom/file) ----
    #[derive(Deserialize)]
    pub struct MassStorageModeParams {
        pub mode: String, // "cdrom" | "file"
    }

    pub async fn set_mass_storage_mode(params: MassStorageModeParams) -> Result<String> {
        let mode = params.mode.to_ascii_lowercase();
        let cdrom = match mode.as_str() {
            "cdrom" => true,
            "file" => false,
            _ => return Err(anyhow!("invalid mode: {}", params.mode)),
        };
        storage_mod::set_mass_storage_mode(cdrom).await?;
        get_mass_storage_mode().await
    }

    pub async fn get_mass_storage_mode() -> Result<String> {
        let cdrom = storage_mod::get_mass_storage_cdrom_enabled().await?;
        Ok(if cdrom { "cdrom".to_string() } else { "file".to_string() })
    }

    // ---- Check mount URL usability (HTTP) ----
    #[derive(Deserialize)]
    pub struct CheckMountUrlParams {
        pub url: String,
    }

    #[derive(Serialize)]
    pub struct VirtualMediaUrlInfo {
        #[serde(rename = "Usable")]
        pub usable: bool,
        #[serde(rename = "Reason", skip_serializing_if = "Option::is_none")]
        pub reason: Option<String>,
        #[serde(rename = "Size")]
        pub size: i64,
    }

    pub fn check_mount_url(params: CheckMountUrlParams) -> Result<VirtualMediaUrlInfo> {
        let r = storage_mod::check_mount_url(&params.url)?;
        Ok(VirtualMediaUrlInfo { usable: r.usable, reason: r.reason, size: r.size })
    }

    // ---- Alias to keep protocol compatible ----
    #[derive(Deserialize)]
    pub struct RpcMountBuiltInImageParams {
        pub filename: String,
    }

    pub async fn rpc_mount_built_in_image(params: RpcMountBuiltInImageParams) -> Result<Value> {
        mount_built_in_image(MountBuiltInImageParams { filename: params.filename }).await
    }
}

/// Broadcast helpers to current session over RPC
pub async fn broadcast_usb_state(state: String) {
    let _ = handlers::set_usb_state(state.clone());
    if let Some(session_id) = get_current_session().await {
        let mut session = Session::new(session_id.clone());
        if let Some(rpc_channel) = get_rpc_channel(&session_id).await {
            session.rpc_channel = Some(rpc_channel);
        }
        let processor = JsonRpcProcessor::new(Arc::new(create_default_registry()));
        let params = serde_json::Value::String(state);
        if let Err(e) = processor.send_event("usbState", Some(params), &session).await {
            warn!("Failed to send usbState event: {}", e);
        }
    }
}

pub async fn broadcast_keyboard_led_state(state: HidKeyboardState) {
    let _ = handlers::set_keyboard_led_state(state);
    if let Some(session_id) = get_current_session().await {
        let mut session = Session::new(session_id.clone());
        if let Some(rpc_channel) = get_rpc_channel(&session_id).await {
            session.rpc_channel = Some(rpc_channel);
        }
        let processor = JsonRpcProcessor::new(Arc::new(create_default_registry()));
        let params = serde_json::to_value(state).unwrap_or(serde_json::json!({}));
        if let Err(e) = processor.send_event("keyboardLedState", Some(params), &session).await {
            warn!("Failed to send keyboardLedState event: {}", e);
        }
    }
}

/// Create default RPC registry
pub fn create_default_registry() -> RpcRegistry {
    let mut registry = RpcRegistry::new();

    // Basic methods
    registry.register_no_params("ping", handlers::ping);
    registry.register_no_params("getDeviceID", handlers::get_device_id);
    registry.register_typed("reboot", handlers::reboot);

    // TODO: 51 RPC methods not implemented, mainly include:
    // - Hardware control: USB HID, video, power management
    // - Virtual media: disk mounting, storage management
    // - Network functions: DHCP, WOL, cloud services
    // - System functions: updates, developer mode

    // Serial settings
    registry.register_typed("setSerialSettings", handlers::set_serial_settings);
    registry.register_no_params("getSerialSettings", handlers::get_serial_settings);

    // Display settings
    registry.register_typed("setDisplayRotation", handlers::set_display_rotation);
    registry.register_no_params("getDisplayRotation", handlers::get_display_rotation);

    // Backlight settings
    registry.register_typed("setBacklightSettings", handlers::set_backlight_settings);
    registry.register_no_params("getBacklightSettings", handlers::get_backlight_settings);

    // Stream quality
    registry.register_no_params("getStreamQualityFactor", handlers::get_stream_quality_factor);
    registry.register_typed("setStreamQualityFactor", handlers::set_stream_quality_factor);

    // Auto update
    registry.register_no_params("getAutoUpdateState", handlers::get_auto_update_state);
    registry.register_typed("setAutoUpdateState", handlers::set_auto_update_state);

    // EDID management
    registry.register_no_params("getEDID", handlers::get_edid);
    registry.register_typed("setEDID", handlers::set_edid);

    // USB device management
    registry.register_no_params("getUsbDevices", handlers::get_usb_devices);
    registry.register_typed("setUsbDeviceState", handlers::set_usb_device_state);
    registry.register_no_params("getUsbEmulationState", handlers::get_usb_emulation_state);
    registry.register_typed("setUsbEmulationState", handlers::set_usb_emulation_state);

    // USB HID input
    registry.register_typed("keyboardReport", handlers::keyboard_report);
    registry.register_typed("absMouseReport", handlers::abs_mouse_report);
    registry.register_typed("relMouseReport", handlers::rel_mouse_report);
    registry.register_typed("wheelReport", handlers::wheel_report);

    // Keyboard layout
    registry.register_no_params("getKeyboardLayout", handlers::get_keyboard_layout);
    registry.register_typed("setKeyboardLayout", handlers::set_keyboard_layout);

    // USB state & keyboard LED
    registry.register_no_params("getUSBState", handlers::get_usb_state);
    registry.register_no_params("getKeyboardLedState", handlers::get_keyboard_led_state);
    // OTA update status
    registry.register_no_params("getUpdateStatus", handlers::get_update_status);
    // TLS state
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
    // Keyboard macros
    registry.register_async("getKeyboardMacros", |_params| {
        Box::pin(async move {
            let result = handlers::get_keyboard_macros().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("setKeyboardMacros", |params| {
        Box::pin(async move {
            let raw = params.ok_or(anyhow!("Missing required parameters"))?;

            // Handle both { params: { macros } } and { macros } formats
            let params_val = if let Value::Object(map) = &raw {
                map.get("params").cloned().unwrap_or_else(|| raw.clone())
            } else {
                raw
            };

            let params: handlers::KeyboardMacrosParams = serde_json::from_value(params_val)?;
            handlers::set_keyboard_macros(params).await
        })
    });

    // Cloud/network/display
    registry.register_async("getCloudState", |_params| {
        Box::pin(async move {
            let result = handlers::get_cloud_state().await?;
            Ok(serde_json::to_value(result)?)
        })
    });

    registry.register_typed("setCloudState", handlers::set_cloud_state);

    registry.register_async("deregisterDevice", |_params| {
        Box::pin(async move { handlers::deregister_device().await })
    });

    // Reset configuration
    registry.register_async("resetConfig", |_params| {
        Box::pin(async move { handlers::reset_config().await })
    });

    registry.register_async("setCloudUrl", |params| {
        Box::pin(async move {
            let params: CloudUrlParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            handlers::set_cloud_url(params).await
        })
    });

    registry.register_no_params("getNetworkState", handlers::get_network_state);
    registry.register_no_params("getNetworkSettings", handlers::get_network_settings);
    registry.register_typed("setNetworkSettings", handlers::set_network_settings);
    registry.register_no_params("renewDHCPLease", handlers::renew_dhcp_lease);
    registry.register_no_params("getNetworkIpAddress", handlers::get_network_ip_address);
    registry.register_typed("setNetworkIpAddress", handlers::set_network_ip_address);
    registry.register_no_params("getDisplayState", handlers::get_display_state);

    // Network settings
    registry.register_async("getLocalLoopbackOnly", |_params| {
        Box::pin(async move {
            let result = handlers::get_local_loopback_only().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("setLocalLoopbackOnly", |params| {
        Box::pin(async move {
            let params: handlers::LocalLoopbackParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            handlers::set_local_loopback_only(params).await
        })
    });

    // Wake-on-LAN
    registry.register_async("getWakeOnLanDevices", |_params| {
        Box::pin(async move {
            let list = handlers::get_wake_on_lan_devices().await?;
            Ok(serde_json::to_value(list)?)
        })
    });
    registry.register_async("setWakeOnLanDevices", |params| {
        Box::pin(async move {
            let raw = params.ok_or(anyhow!("Missing required parameters"))?;
            let inner = raw.get("params").ok_or_else(|| anyhow!("Missing field 'params'"))?.clone();
            let params: handlers::SetWakeOnLanDevicesParams = serde_json::from_value(inner)?;
            handlers::set_wake_on_lan_devices(params).await
        })
    });
    registry.register_async("sendWOLMagicPacket", |params| {
        Box::pin(async move {
            let params: handlers::SendWOLMagicPacketParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            handlers::send_wol_magic_packet(params).await
        })
    });

    // Virtual media (storage) RPCs
    registry.register_no_params("getVirtualMediaState", handlers::get_virtual_media_state);
    // Video state
    registry.register_async("getVideoState", |_params| {
        Box::pin(async move {
            let result = handlers::get_video_state().await?;
            Ok(serde_json::to_value(result)?)
        })
    });

    registry.register_async("mountWithHTTP", |params| {
        Box::pin(async move {
            let params: MountHttpParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            handlers::mount_with_http(params).await
        })
    });

    registry.register_async("mountWithWebRTC", |params| {
        Box::pin(async move {
            let params: MountWebRtcParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            handlers::mount_with_webrtc(params).await
        })
    });

    registry.register_async("mountWithStorage", |params| {
        Box::pin(async move {
            let params: MountStorageParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            handlers::mount_with_storage(params).await
        })
    });

    registry.register_async("unmountImage", |_params| {
        Box::pin(async move { handlers::unmount_image().await })
    });

    // Mass storage mode
    registry.register_async("setMassStorageMode", |params| {
        Box::pin(async move {
            let params: MassStorageModeParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            let result = handlers::set_mass_storage_mode(params).await?;
            Ok(serde_json::to_value(result)?)
        })
    });

    registry.register_async("getMassStorageMode", |_params| {
        Box::pin(async move {
            let result = handlers::get_mass_storage_mode().await?;
            Ok(serde_json::to_value(result)?)
        })
    });

    registry.register_typed("checkMountUrl", handlers::check_mount_url);

    registry.register_async("startStorageFileUpload", |params| {
        Box::pin(async move {
            let params: StartUploadParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            let result = handlers::start_storage_file_upload(params).await?;
            Ok(serde_json::to_value(result)?)
        })
    });

    registry.register_async("listStorageFiles", |_params| {
        Box::pin(async move {
            let result = handlers::list_storage_files().await?;
            Ok(serde_json::to_value(result)?)
        })
    });

    registry.register_async("deleteStorageFile", |params| {
        Box::pin(async move {
            let params: DeleteStorageFileParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            handlers::delete_storage_file(params).await
        })
    });

    registry.register_async("getStorageSpace", |_params| {
        Box::pin(async move {
            let result = handlers::get_storage_space().await?;
            Ok(serde_json::to_value(result)?)
        })
    });

    registry.register_async("mountBuiltInImage", |params| {
        Box::pin(async move {
            let params: MountBuiltInImageParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            handlers::mount_built_in_image(params).await
        })
    });

    registry.register_async("rpcMountBuiltInImage", |params| {
        Box::pin(async move {
            let params: RpcMountBuiltInImageParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            handlers::rpc_mount_built_in_image(params).await
        })
    });

    // SSH key management
    registry.register_no_params("getSSHKeyState", handlers::get_ssh_key_state);
    registry.register_typed("setSSHKeyState", handlers::set_ssh_key_state);

    // Developer mode
    registry.register_async("getDevModeState", |_params| {
        Box::pin(async move {
            let result = handlers::get_dev_mode_state_handler().await?;
            Ok(serde_json::to_value(result)?)
        })
    });
    registry.register_async("setDevModeState", |params| {
        Box::pin(async move {
            let params: handlers::DevModeParams =
                serde_json::from_value(params.ok_or(anyhow!("Missing required parameters"))?)?;
            handlers::set_dev_mode_state_handler(params).await
        })
    });

    registry
}
