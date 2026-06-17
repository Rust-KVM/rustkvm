use std::rc::Rc;

use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::Value;

use crate::rpc::RpcClient;

pub type Rpc = StoredValue<Option<Rc<RpcClient>>, LocalStorage>;

pub fn fire(rpc: Rpc, method: &'static str, params: Value) {
    if let Some(client) = rpc.get_value() {
        spawn_local(async move {
            let _ = client.call(method, params).await;
        });
    }
}

pub fn json_str(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn as_string(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

pub fn json_string_list(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}
