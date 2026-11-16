//! Tauri v2 IPC bridge to source-daemon (HTTP JSON-RPC). Real commands — no stubs.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use source_core::protocol::{JsonRpcId, JsonRpcRequest, JsonRpcResponse, PROTOCOL_VERSION};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tauri::State;

static RPC_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct DaemonBridge {
    pub base_url: String,
    client: reqwest::Client,
}

impl DaemonBridge {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
        }
    }

    pub async fn rpc(&self, method: &str, params: Value) -> Result<Value, String> {
        let id = RPC_ID.fetch_add(1, Ordering::Relaxed) as i64;
        let req = JsonRpcRequest::new(JsonRpcId::Number(id), method, params);
        let url = format!("{}/rpc", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable at {url}: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("daemon HTTP {}", resp.status()));
        }
        let body: JsonRpcResponse = resp
            .json()
            .await
            .map_err(|e| format!("invalid rpc response: {e}"))?;
        if let Some(err) = body.error {
            return Err(format!("{}: {}", err.code, err.message));
        }
        Ok(body.result.unwrap_or(Value::Null))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BridgeInfo {
    pub protocol_version: String,
    pub daemon_base_url: String,
    pub desktop_version: String,
}

#[tauri::command]
async fn bridge_info(bridge: State<'_, Arc<DaemonBridge>>) -> Result<BridgeInfo, String> {
    Ok(BridgeInfo {
        protocol_version: PROTOCOL_VERSION.to_string(),
        daemon_base_url: bridge.base_url.clone(),
        desktop_version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[tauri::command]
async fn daemon_rpc(
    bridge: State<'_, Arc<DaemonBridge>>,
    method: String,
    params: Option<Value>,
) -> Result<Value, String> {
    bridge.rpc(&method, params.unwrap_or(Value::Object(Default::default()))).await
}

#[tauri::command]
async fn daemon_status(bridge: State<'_, Arc<DaemonBridge>>) -> Result<Value, String> {
    bridge.rpc("source/status", Value::Object(Default::default())).await
}

#[tauri::command]
async fn daemon_network_tier(bridge: State<'_, Arc<DaemonBridge>>) -> Result<Value, String> {
    bridge
        .rpc("source/network/getTier", Value::Object(Default::default()))
        .await
}

#[tauri::command]
async fn daemon_set_network_tier(
    bridge: State<'_, Arc<DaemonBridge>>,
    tier: String,
) -> Result<Value, String> {
    bridge
        .rpc(
            "source/network/setTier",
            serde_json::json!({ "tier": tier }),
        )
        .await
}

#[tauri::command]
async fn daemon_health(bridge: State<'_, Arc<DaemonBridge>>) -> Result<Value, String> {
    let url = format!("{}/health", bridge.base_url);
    let resp = bridge
        .client
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

pub fn default_daemon_url() -> String {
    std::env::var("SOURCE_DAEMON_URL").unwrap_or_else(|_| "http://127.0.0.1:7420".into())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let bridge = Arc::new(DaemonBridge::new(default_daemon_url()));
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(bridge)
        .invoke_handler(tauri::generate_handler![
            bridge_info,
            daemon_rpc,
            daemon_status,
            daemon_network_tier,
            daemon_set_network_tier,
            daemon_health,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Source desktop");
}