use std::collections::HashMap;
use std::panic;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use futures::future::BoxFuture;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::{Instrument, debug, error, warn};
use webrtc::data_channel::data_channel_message::DataChannelMessage;

use super::types::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use crate::session::Session;

pub trait RpcHandler: Send + Sync {
    fn call_async(&self, params: Option<Value>) -> BoxFuture<'_, Result<Value>>;
}

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
    fn call_async(&self, params: Option<Value>) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move {
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
        })
    }
}

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
    fn call_async(&self, _params: Option<Value>) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move {
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
        })
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
    fn call_async(&self, params: Option<Value>) -> BoxFuture<'_, Result<Value>> {
        Box::pin(async move { (self.func)(params).await })
    }
}

fn convert_parameters<P: DeserializeOwned>(params: Value) -> Result<P> {
    let params_to_use = if let Value::Object(mut map) = params {
        if map.len() == 1 {
            if let Some(inner) = map.remove("params") { inner } else { Value::Object(map) }
        } else {
            Value::Object(map)
        }
    } else {
        params
    };

    match serde_json::from_value::<P>(params_to_use.clone()) {
        Ok(result) => Ok(result),
        Err(primary_error) => match enhanced_parameter_conversion::<P>(&params_to_use) {
            Ok(result) => Ok(result),
            Err(_) => Err(anyhow!("Parameter conversion failed: {}", primary_error)),
        },
    }
}

fn enhanced_parameter_conversion<P: DeserializeOwned>(params: &Value) -> Result<P> {
    match params {
        Value::Object(map) => {
            let converted_map = convert_object_values(map)?;
            let json_value = Value::Object(converted_map);
            serde_json::from_value(json_value)
                .map_err(|e| anyhow!("Object conversion failed: {}", e))
        }

        Value::Array(arr) => {
            let converted_array = convert_array_values(arr)?;
            let json_value = Value::Array(converted_array);
            serde_json::from_value(json_value)
                .map_err(|e| anyhow!("Array conversion failed: {}", e))
        }

        _ => {
            let converted_value = convert_scalar_value(params)?;
            serde_json::from_value(converted_value)
                .map_err(|e| anyhow!("Scalar conversion failed: {}", e))
        }
    }
}

fn convert_object_values(
    map: &serde_json::Map<String, Value>,
) -> Result<serde_json::Map<String, Value>> {
    let mut converted_map = serde_json::Map::new();

    for (key, value) in map {
        let converted_value = match value {
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

            Value::String(s) => {
                if let Ok(int_val) = s.parse::<i64>() {
                    Value::Number(serde_json::Number::from(int_val))
                } else if let Ok(float_val) = s.parse::<f64>() {
                    if let Some(number) = serde_json::Number::from_f64(float_val) {
                        Value::Number(number)
                    } else {
                        value.clone()
                    }
                } else {
                    value.clone()
                }
            }

            Value::Object(nested_map) => Value::Object(convert_object_values(nested_map)?),

            Value::Array(nested_array) => Value::Array(convert_array_values(nested_array)?),

            _ => value.clone(),
        };

        converted_map.insert(key.clone(), converted_value);
    }

    Ok(converted_map)
}

fn convert_array_values(arr: &[Value]) -> Result<Vec<Value>> {
    let mut converted_array = Vec::new();

    for value in arr {
        let converted_value = match value {
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

            Value::Object(nested_map) => Value::Object(convert_object_values(nested_map)?),

            Value::Array(nested_array) => Value::Array(convert_array_values(nested_array)?),

            _ => value.clone(),
        };

        converted_array.push(converted_value);
    }

    Ok(converted_array)
}

fn convert_scalar_value(value: &Value) -> Result<Value> {
    match value {
        Value::Number(n) if n.is_f64() => {
            if let Some(f_val) = n.as_f64() {
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
            if let Ok(int_val) = s.parse::<i64>() {
                Ok(Value::Number(serde_json::Number::from(int_val)))
            } else if let Ok(float_val) = s.parse::<f64>() {
                if let Some(number) = serde_json::Number::from_f64(float_val) {
                    Ok(Value::Number(number))
                } else {
                    Ok(value.clone())
                }
            } else {
                Ok(value.clone())
            }
        }

        _ => Ok(value.clone()),
    }
}

pub struct RpcRegistry {
    handlers: HashMap<String, Arc<dyn RpcHandler>>,
}

impl RpcRegistry {
    pub fn new() -> Self {
        Self { handlers: HashMap::new() }
    }

    pub fn register<H>(&mut self, method: &str, handler: H)
    where
        H: RpcHandler + 'static,
    {
        self.handlers.insert(method.to_string(), Arc::new(handler));
    }

    pub fn register_typed<P, R, F>(&mut self, method: &str, func: F)
    where
        P: DeserializeOwned + Send + Sync + 'static,
        R: Serialize + Send + Sync + 'static,
        F: Fn(P) -> Result<R> + Send + Sync + 'static,
    {
        self.register(method, TypedHandler::new(func));
    }

    pub fn register_no_params<R, F>(&mut self, method: &str, func: F)
    where
        R: Serialize + Send + Sync + 'static,
        F: Fn() -> Result<R> + Send + Sync + 'static,
    {
        self.register(method, NoParamsHandler::new(func));
    }

    pub fn get_handler(&self, method: &str) -> Option<&Arc<dyn RpcHandler>> {
        self.handlers.get(method)
    }

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

pub struct JsonRpcProcessor {
    registry: Arc<RpcRegistry>,
}

impl JsonRpcProcessor {
    pub fn new(registry: Arc<RpcRegistry>) -> Self {
        Self { registry }
    }

    pub async fn handle_message(&self, message: DataChannelMessage, session: &Session) {
        let request: JsonRpcRequest = match serde_json::from_slice(&message.data) {
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

        crate::observability::metrics::RPC_CALLS_TOTAL.with_label_values(&[&request.method]).inc();

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

        let _rpc_timer = crate::observability::metrics::RPC_LATENCY_SECONDS.start_timer();
        let span = tracing::info_span!("rpc", method = %request.method, id = ?request.id);
        match handler.call_async(request.params).instrument(span).await {
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
        debug!("Sending JSON-RPC event: method={}, data={}", method, event_json);

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
