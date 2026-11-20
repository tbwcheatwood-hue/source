//! Tauri v2 IPC bridge to source-daemon (HTTP JSON-RPC). Real commands — no stubs.
//! On startup, auto-starts a bundled `source-daemon` sidecar when health check fails.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use source_core::protocol::{JsonRpcId, JsonRpcRequest, JsonRpcResponse, PROTOCOL_VERSION};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, State};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;

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

    pub async fn health_ok(&self) -> bool {
        let url = format!("{}/health", self.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
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
    bridge
        .rpc(&method, params.unwrap_or(Value::Object(Default::default())))
        .await
}

#[tauri::command]
async fn daemon_status(bridge: State<'_, Arc<DaemonBridge>>) -> Result<Value, String> {
    bridge
        .rpc("source/status", Value::Object(Default::default()))
        .await
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

fn daemon_bind_from_url(base_url: &str) -> String {
    url::Url::parse(base_url)
        .ok()
        .and_then(|u| {
            let host = u.host_str()?.to_string();
            let port = u.port().unwrap_or(7420);
            Some(format!("{host}:{port}"))
        })
        .unwrap_or_else(|| "127.0.0.1:7420".into())
}

async fn wait_for_daemon(bridge: &DaemonBridge, attempts: u32) -> bool {
    for _ in 0..attempts {
        if bridge.health_ok().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}

/// Start bundled sidecar when nothing is listening; fall back to PATH `source-daemon`.
async fn ensure_daemon_running(app: &AppHandle, bridge: &DaemonBridge) {
    if bridge.health_ok().await {
        tracing::info!("source-daemon already healthy at {}", bridge.base_url);
        return;
    }

    let bind = daemon_bind_from_url(&bridge.base_url);
    tracing::info!("source-daemon not reachable; attempting auto-start on {bind}");

    let mut started = false;

    match app.shell().sidecar("source-daemon") {
        Ok(cmd) => match cmd.args(["--bind", &bind]).spawn() {
            Ok((mut rx, _child)) => {
                started = true;
                tauri::async_runtime::spawn(async move {
                    while let Some(event) = rx.recv().await {
                        if let CommandEvent::Terminated(payload) = event {
                            tracing::warn!(?payload, "source-daemon sidecar exited");
                            break;
                        }
                    }
                });
            }
            Err(e) => tracing::warn!("sidecar spawn failed: {e}"),
        },
        Err(e) => tracing::warn!("sidecar unavailable: {e}"),
    }

    if !started {
        let daemon_name = if cfg!(windows) {
            "source-daemon.exe"
        } else {
            "source-daemon"
        };
        // Prefer a sibling binary next to the desktop exe (portable layouts).
        let sibling = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join(daemon_name)));
        let exe = sibling
            .filter(|p| p.exists())
            .unwrap_or_else(|| std::path::PathBuf::from(daemon_name));
        match std::process::Command::new(&exe)
            .args(["--bind", &bind])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {
                started = true;
                tracing::info!("started source-daemon from {}", exe.display());
            }
            Err(e) => tracing::warn!(
                "could not start source-daemon from PATH/sibling ({}): {e}",
                exe.display()
            ),
        }
    }

    if started {
        if wait_for_daemon(bridge, 40).await {
            tracing::info!("source-daemon is healthy at {}", bridge.base_url);
        } else {
            tracing::warn!(
                "source-daemon started but /health not ready at {}",
                bridge.base_url
            );
        }
    } else {
        tracing::warn!(
            "desktop UI will run without auto-started daemon; start manually: cargo run -p source-daemon -- --bind {bind}"
        );
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let bridge = Arc::new(DaemonBridge::new(default_daemon_url()));
    let bridge_setup = bridge.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(bridge)
        .setup(move |app| {
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                ensure_daemon_running(&handle, &bridge_setup).await;
            });
            Ok(())
        })
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
