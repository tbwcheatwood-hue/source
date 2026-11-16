//! MCP Host — stdio + SSE/Streamable HTTP, hot-reload `~/.source/extensions/`,
//! filter tools by offline_capable / requires_wan per network tier.

use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use source_core::policy::NetworkTier;
use source_core::types::ToolDefinition;
use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use uuid::Uuid;

use crate::policy::PolicyEngine;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerManifest {
    pub id: String,
    pub name: String,
    #[serde(default = "default_stdio")]
    pub transport: String, // stdio | sse | streamable_http
    #[serde(default)]
    pub command: Option<Vec<String>>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default = "default_true")]
    pub offline_capable: bool,
    #[serde(default)]
    pub requires_wan: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

fn default_stdio() -> String {
    "stdio".into()
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerInfo {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub offline_capable: bool,
    pub requires_wan: bool,
    pub enabled: bool,
    pub running: bool,
    pub tools: Vec<McpToolInfo>,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub offline_capable: bool,
    pub requires_wan: bool,
}

struct StdioSession {
    child: Child,
    stdin: ChildStdin,
    reader: Arc<Mutex<BufReader<ChildStdout>>>,
    next_id: u64,
}

#[derive(Clone)]
pub struct McpHost {
    extensions_dir: PathBuf,
    policy: PolicyEngine,
    servers: Arc<DashMap<String, LoadedServer>>,
    _watcher_thread: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

struct LoadedServer {
    info: McpServerInfo,
    manifest: McpServerManifest,
    stdio: Option<Mutex<StdioSession>>,
}

impl McpHost {
    pub fn start(extensions_dir: PathBuf, policy: PolicyEngine) -> Result<Self> {
        fs::create_dir_all(&extensions_dir)?;
        let host = Self {
            extensions_dir: extensions_dir.clone(),
            policy,
            servers: Arc::new(DashMap::new()),
            _watcher_thread: Arc::new(Mutex::new(None)),
        };
        host.reload()?;
        host.spawn_watcher()?;
        Ok(host)
    }

    fn spawn_watcher(&self) -> Result<()> {
        let dir = self.extensions_dir.clone();
        let servers = self.servers.clone();
        let policy = self.policy.clone();
        let (tx, rx) = mpsc::channel();
        let mut watcher = RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            notify::Config::default().with_poll_interval(Duration::from_secs(2)),
        )?;
        watcher.watch(&dir, RecursiveMode::Recursive)?;

        let handle = thread::Builder::new()
            .name("source-mcp-watch".into())
            .spawn(move || {
                // Keep watcher alive in this thread.
                let _watcher = watcher;
                while let Ok(res) = rx.recv() {
                    match res {
                        Ok(event) => {
                            if matches!(
                                event.kind,
                                EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                            ) {
                                if let Ok(hostish) =
                                    reload_into(&dir, &policy, &servers)
                                {
                                    tracing::info!(count = hostish, "mcp extensions reloaded");
                                }
                            }
                        }
                        Err(err) => tracing::warn!(error = %err, "mcp watch error"),
                    }
                }
            })?;
        *self._watcher_thread.lock() = Some(handle);
        Ok(())
    }

    pub fn reload(&self) -> Result<Vec<McpServerInfo>> {
        reload_into(&self.extensions_dir, &self.policy, &self.servers)?;
        Ok(self.list_filtered())
    }

    pub fn list_filtered(&self) -> Vec<McpServerInfo> {
        let tier = self.policy.tier();
        self.servers
            .iter()
            .filter_map(|e| {
                let mut info = e.value().info.clone();
                if !tool_allowed_for_tier(info.offline_capable, info.requires_wan, tier) {
                    return None;
                }
                info.tools
                    .retain(|t| tool_allowed_for_tier(t.offline_capable, t.requires_wan, tier));
                Some(info)
            })
            .collect()
    }

    pub fn list_all(&self) -> Vec<McpServerInfo> {
        self.servers.iter().map(|e| e.value().info.clone()).collect()
    }

    pub async fn call_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<Value> {
        let tier = self.policy.tier();
        let server = self
            .servers
            .get(server_id)
            .ok_or_else(|| anyhow!("unknown MCP server {server_id}"))?;
        if !tool_allowed_for_tier(server.manifest.offline_capable, server.manifest.requires_wan, tier)
        {
            return Err(anyhow!(
                "MCP server {server_id} blocked by network tier {}",
                tier.as_str()
            ));
        }
        if let Some(tool) = server.info.tools.iter().find(|t| t.name == tool_name) {
            if !tool_allowed_for_tier(tool.offline_capable, tool.requires_wan, tier) {
                return Err(anyhow!(
                    "MCP tool {tool_name} requires WAN; blocked at tier {}",
                    tier.as_str()
                ));
            }
        }
        // Network-gated URL transports
        if matches!(server.manifest.transport.as_str(), "sse" | "streamable_http") {
            if let Some(url) = &server.manifest.url {
                self.policy
                    .assert_url_allowed(url, None, &format!("mcp:{server_id}"))
                    .await
                    .map_err(|e| anyhow!(e.to_string()))?;
            }
        }

        match server.manifest.transport.as_str() {
            "stdio" => {
                drop(server);
                self.call_stdio(server_id, tool_name, arguments)
            }
            "sse" | "streamable_http" => {
                let url = server
                    .manifest
                    .url
                    .clone()
                    .ok_or_else(|| anyhow!("MCP server {server_id} missing url"))?;
                drop(server);
                call_http_mcp(&url, tool_name, arguments).await
            }
            other => Err(anyhow!("unsupported MCP transport {other}")),
        }
    }

    fn call_stdio(&self, server_id: &str, tool_name: &str, arguments: Value) -> Result<Value> {
        let server = self
            .servers
            .get(server_id)
            .ok_or_else(|| anyhow!("unknown MCP server {server_id}"))?;
        let stdio = server
            .stdio
            .as_ref()
            .ok_or_else(|| anyhow!("stdio session not running for {server_id}"))?;
        let mut session = stdio.lock();
        let id = session.next_id;
        session.next_id += 1;
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            }
        });
        writeln!(session.stdin, "{}", req)?;
        session.stdin.flush()?;

        let mut reader = session.reader.lock();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        while std::time::Instant::now() < deadline {
            let mut line = String::new();
            let n = reader.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(line)?;
            if v.get("id").and_then(|x| x.as_u64()) == Some(id)
                || v.get("id").and_then(|x| x.as_i64()) == Some(id as i64)
            {
                if let Some(err) = v.get("error") {
                    return Err(anyhow!("mcp error: {err}"));
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
        Err(anyhow!("timeout waiting for MCP tool result"))
    }

    /// Convert filtered MCP tools into daemon ToolDefinition entries (namespaced).
    pub fn as_tool_definitions(&self) -> Vec<ToolDefinition> {
        let mut out = Vec::new();
        for info in self.list_filtered() {
            for tool in info.tools {
                out.push(ToolDefinition {
                    name: format!("mcp__{}__{}", info.id, tool.name),
                    description: format!("[MCP:{}] {}", info.name, tool.description),
                    input_schema: tool.input_schema,
                    categories: vec![source_core::policy::ApprovalCategory::Mcp],
                    enabled: info.enabled,
                    offline_capable: tool.offline_capable && info.offline_capable,
                    requires_wan: tool.requires_wan || info.requires_wan,
                });
            }
        }
        out
    }
}

pub fn tool_allowed_for_tier(offline_capable: bool, requires_wan: bool, tier: NetworkTier) -> bool {
    match tier {
        NetworkTier::LocalOnly | NetworkTier::LocalAndLan => offline_capable && !requires_wan,
        NetworkTier::UnrestrictedConfigurable => true,
    }
}

fn reload_into(
    extensions_dir: &Path,
    policy: &PolicyEngine,
    servers: &DashMap<String, LoadedServer>,
) -> Result<usize> {
    let mut seen = Vec::new();
    if extensions_dir.exists() {
        for entry in fs::read_dir(extensions_dir)? {
            let entry = entry?;
            let path = entry.path();
            let manifest_path = if path.is_dir() {
                path.join("mcp.json")
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                == Some("json")
            {
                path.clone()
            } else {
                continue;
            };
            if !manifest_path.exists() {
                continue;
            }
            let text = fs::read_to_string(&manifest_path)?;
            let mut manifest: McpServerManifest = serde_json::from_str(&text)
                .with_context(|| format!("parse {}", manifest_path.display()))?;
            if manifest.id.is_empty() {
                manifest.id = Uuid::new_v4().to_string();
            }
            seen.push(manifest.id.clone());
            // Stop previous
            if let Some((_, old)) = servers.remove(&manifest.id) {
                if let Some(stdio) = old.stdio {
                    let mut s = stdio.lock();
                    let _ = s.child.kill();
                }
            }
            let loaded = start_server(manifest, manifest_path, policy)?;
            servers.insert(loaded.info.id.clone(), loaded);
        }
    }
    // Remove gone
    let stale: Vec<String> = servers
        .iter()
        .filter(|e| !seen.contains(e.key()))
        .map(|e| e.key().clone())
        .collect();
    for id in stale {
        if let Some((_, old)) = servers.remove(&id) {
            if let Some(stdio) = old.stdio {
                let mut s = stdio.lock();
                let _ = s.child.kill();
            }
        }
    }
    Ok(servers.len())
}

fn start_server(
    manifest: McpServerManifest,
    path: PathBuf,
    _policy: &PolicyEngine,
) -> Result<LoadedServer> {
    let mut tools = Vec::new();
    let mut stdio = None;
    let mut running = false;

    match manifest.transport.as_str() {
        "stdio" => {
            let cmd = manifest
                .command
                .clone()
                .ok_or_else(|| anyhow!("stdio MCP {} needs command", manifest.id))?;
            if cmd.is_empty() {
                return Err(anyhow!("empty command for {}", manifest.id));
            }
            let mut command = Command::new(&cmd[0]);
            if cmd.len() > 1 {
                command.args(&cmd[1..]);
            }
            for (k, v) in &manifest.env {
                command.env(k, v);
            }
            command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
            match command.spawn() {
                Ok(mut child) => {
                    let stdin = child.stdin.take().context("mcp stdin")?;
                    let stdout = child.stdout.take().context("mcp stdout")?;
                    let reader = Arc::new(Mutex::new(BufReader::new(stdout)));
                    let mut session = StdioSession {
                        child,
                        stdin,
                        reader: reader.clone(),
                        next_id: 1,
                    };
                    // initialize + tools/list
                    let init = json!({
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "initialize",
                        "params": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": {},
                            "clientInfo": { "name": "source-daemon", "version": env!("CARGO_PKG_VERSION") }
                        }
                    });
                    let _ = writeln!(session.stdin, "{}", init);
                    let _ = session.stdin.flush();
                    session.next_id = 2;
                    // Read init response (best effort)
                    {
                        let mut r = session.reader.lock();
                        let _ = r.read_line(&mut String::new());
                    }
                    let list_req = json!({
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "tools/list",
                        "params": {}
                    });
                    let _ = writeln!(session.stdin, "{}", list_req);
                    let _ = session.stdin.flush();
                    session.next_id = 3;
                    {
                        let mut r = session.reader.lock();
                        let mut line = String::new();
                        if r.read_line(&mut line).unwrap_or(0) > 0 {
                            if let Ok(v) = serde_json::from_str::<Value>(line.trim()) {
                                if let Some(arr) = v.pointer("/result/tools").and_then(|x| x.as_array()) {
                                    for t in arr {
                                        tools.push(McpToolInfo {
                                            name: t
                                                .get("name")
                                                .and_then(|x| x.as_str())
                                                .unwrap_or("tool")
                                                .to_string(),
                                            description: t
                                                .get("description")
                                                .and_then(|x| x.as_str())
                                                .unwrap_or("")
                                                .to_string(),
                                            input_schema: t
                                                .get("inputSchema")
                                                .cloned()
                                                .unwrap_or_else(|| json!({"type":"object"})),
                                            offline_capable: manifest.offline_capable,
                                            requires_wan: manifest.requires_wan,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    running = true;
                    stdio = Some(Mutex::new(session));
                }
                Err(err) => {
                    tracing::warn!(id = %manifest.id, error = %err, "failed to spawn MCP stdio server");
                }
            }
        }
        "sse" | "streamable_http" => {
            // Tools discovered lazily / via optional tools field in manifest file sibling.
            let tools_path = path.parent().unwrap_or(Path::new(".")).join("tools.json");
            if tools_path.exists() {
                if let Ok(v) = serde_json::from_str::<Value>(&fs::read_to_string(tools_path)?) {
                    if let Some(arr) = v.as_array() {
                        for t in arr {
                            tools.push(McpToolInfo {
                                name: t.get("name").and_then(|x| x.as_str()).unwrap_or("tool").into(),
                                description: t
                                    .get("description")
                                    .and_then(|x| x.as_str())
                                    .unwrap_or("")
                                    .into(),
                                input_schema: t
                                    .get("inputSchema")
                                    .cloned()
                                    .unwrap_or_else(|| json!({"type":"object"})),
                                offline_capable: manifest.offline_capable,
                                requires_wan: manifest.requires_wan,
                            });
                        }
                    }
                }
            }
            running = manifest.url.is_some();
        }
        other => return Err(anyhow!("unsupported transport {other}")),
    }

    let info = McpServerInfo {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        transport: manifest.transport.clone(),
        offline_capable: manifest.offline_capable,
        requires_wan: manifest.requires_wan,
        enabled: manifest.enabled,
        running,
        tools,
        path,
    };
    Ok(LoadedServer {
        info,
        manifest,
        stdio,
    })
}

async fn call_http_mcp(base_url: &str, tool_name: &str, arguments: Value) -> Result<Value> {
    let client = reqwest::Client::new();
    // Streamable HTTP / JSON-RPC style POST to base URL
    let req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": { "name": tool_name, "arguments": arguments }
    });
    let resp = client
        .post(base_url)
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .json(&req)
        .send()
        .await?
        .error_for_status()?;
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if ct.contains("text/event-stream") {
        let text = resp.text().await?;
        // Parse last data: line as JSON-RPC
        for line in text.lines().rev() {
            if let Some(data) = line.strip_prefix("data:") {
                let v: Value = serde_json::from_str(data.trim())?;
                if let Some(err) = v.get("error") {
                    return Err(anyhow!("mcp sse error: {err}"));
                }
                return Ok(v.get("result").cloned().unwrap_or(Value::Null));
            }
        }
        Err(anyhow!("no SSE data in MCP response"))
    } else {
        let v: Value = resp.json().await?;
        if let Some(err) = v.get("error") {
            return Err(anyhow!("mcp http error: {err}"));
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_filters_wan_tools() {
        assert!(tool_allowed_for_tier(true, false, NetworkTier::LocalOnly));
        assert!(!tool_allowed_for_tier(true, true, NetworkTier::LocalOnly));
        assert!(!tool_allowed_for_tier(false, false, NetworkTier::LocalAndLan));
        assert!(tool_allowed_for_tier(false, true, NetworkTier::UnrestrictedConfigurable));
    }
}
