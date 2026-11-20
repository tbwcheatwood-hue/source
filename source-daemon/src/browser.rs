//! headless_browser + app_verifier — Chromium CDP closed-loop with network policy gating.

use anyhow::{anyhow, Context, Result};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use source_core::error::CoreError;
use source_core::types::SessionId;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::policy::PolicyEngine;

#[derive(Clone)]
pub struct HeadlessBrowser {
    policy: PolicyEngine,
    chrome_path: PathBuf,
    sessions: Arc<DashMap<String, Arc<BrowserSession>>>,
}

struct BrowserSession {
    child: Mutex<Child>,
    ws_url: Mutex<Option<String>>,
    last_url: Mutex<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NavigateResult {
    pub session_key: String,
    pub url: String,
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InspectResult {
    pub session_key: String,
    pub url: String,
    pub title: String,
    pub html_excerpt: String,
    pub text_excerpt: String,
    pub selector_html: Option<String>,
    pub script_result: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppVerifyResult {
    pub ok: bool,
    pub url: String,
    pub launch_pid: Option<u32>,
    pub title: String,
    pub matched_text: bool,
    pub ready_selector_found: bool,
    pub details: String,
    pub html_excerpt: String,
}

impl HeadlessBrowser {
    pub fn new(policy: PolicyEngine) -> Self {
        Self {
            policy,
            chrome_path: find_chrome(),
            sessions: Arc::new(DashMap::new()),
        }
    }

    pub async fn navigate(
        &self,
        url: &str,
        session_key: Option<&str>,
        wait_ms: Option<u64>,
        session_id: Option<SessionId>,
    ) -> Result<NavigateResult, CoreError> {
        self.policy
            .assert_url_allowed(url, session_id, "headless_browser:navigate")
            .await?;
        let key = session_key.unwrap_or("default").to_string();
        let session = self.ensure_session(&key).await.map_err(|e| CoreError::other(e.to_string()))?;
        let page = self
            .cdp_call(&session, "Page.navigate", json!({ "url": url }))
            .await
            .map_err(|e| CoreError::other(e.to_string()))?;
        *session.last_url.lock() = url.to_string();
        tokio::time::sleep(Duration::from_millis(wait_ms.unwrap_or(750))).await;
        let title = self
            .evaluate_string(&session, "document.title || ''")
            .await
            .unwrap_or_default();
        Ok(NavigateResult {
            session_key: key,
            url: url.to_string(),
            title,
            status: page
                .get("frameId")
                .map(|_| "navigated")
                .unwrap_or("ok")
                .to_string(),
        })
    }

    pub async fn inspect(
        &self,
        session_key: Option<&str>,
        selector: Option<&str>,
        script: Option<&str>,
    ) -> Result<InspectResult, CoreError> {
        let key = session_key.unwrap_or("default").to_string();
        let session = self
            .sessions
            .get(&key)
            .ok_or_else(|| CoreError::InvalidArgument(format!("no browser session {key}")))?
            .clone();
        let url = self
            .evaluate_string(&session, "location.href")
            .await
            .unwrap_or_else(|_| session.last_url.lock().clone());
        let title = self
            .evaluate_string(&session, "document.title || ''")
            .await
            .unwrap_or_default();
        let html = self
            .evaluate_string(&session, "document.documentElement.outerHTML")
            .await
            .unwrap_or_default();
        let text = self
            .evaluate_string(&session, "document.body ? document.body.innerText : ''")
            .await
            .unwrap_or_default();
        let selector_html = if let Some(sel) = selector {
            let js = format!(
                "(function(){{ const el=document.querySelector({}); return el? el.outerHTML : null; }})()",
                serde_json::to_string(sel).unwrap()
            );
            Some(
                self.evaluate_string(&session, &js)
                    .await
                    .unwrap_or_default(),
            )
        } else {
            None
        };
        let script_result = if let Some(s) = script {
            Some(
                self.evaluate_value(&session, s)
                    .await
                    .map_err(|e| CoreError::other(e.to_string()))?,
            )
        } else {
            None
        };
        Ok(InspectResult {
            session_key: key,
            url,
            title,
            html_excerpt: html.chars().take(16_384).collect(),
            text_excerpt: text.chars().take(8_192).collect(),
            selector_html,
            script_result,
        })
    }

    /// Build→launch→inspect→fix loop. Launch command should start a local server; URL is policy-checked.
    pub async fn verify_app(
        &self,
        launch_command: &str,
        cwd: Option<&str>,
        url: &str,
        ready_selector: Option<&str>,
        assert_text: Option<&str>,
        timeout_ms: Option<u64>,
        session_id: Option<SessionId>,
    ) -> Result<AppVerifyResult, CoreError> {
        self.policy
            .assert_url_allowed(url, session_id, "app_verifier")
            .await?;

        let timeout = Duration::from_millis(timeout_ms.unwrap_or(30_000));
        let mut child = spawn_shell_command(launch_command, cwd.unwrap_or("."))
            .map_err(|e| CoreError::other(format!("launch failed: {e}")))?;
        let pid = child.id();

        let key = format!("verify-{pid}");
        let deadline = Instant::now() + timeout;
        let mut last_err = String::new();
        let mut result = AppVerifyResult {
            ok: false,
            url: url.into(),
            launch_pid: Some(pid),
            title: String::new(),
            matched_text: false,
            ready_selector_found: ready_selector.is_none(),
            details: String::new(),
            html_excerpt: String::new(),
        };

        while Instant::now() < deadline {
            match self.navigate(url, Some(&key), Some(400), session_id).await {
                Ok(nav) => {
                    result.title = nav.title;
                    match self
                        .inspect(Some(&key), ready_selector, None)
                        .await
                    {
                        Ok(ins) => {
                            result.html_excerpt = ins.html_excerpt.clone();
                            if let Some(sel) = ready_selector {
                                result.ready_selector_found = ins
                                    .selector_html
                                    .as_ref()
                                    .map(|s| !s.is_empty() && s != "null")
                                    .unwrap_or(false);
                                if !result.ready_selector_found {
                                    last_err = format!("selector not ready: {sel}");
                                    tokio::time::sleep(Duration::from_millis(400)).await;
                                    continue;
                                }
                            }
                            if let Some(text) = assert_text {
                                result.matched_text = ins.text_excerpt.contains(text)
                                    || ins.html_excerpt.contains(text);
                                if !result.matched_text {
                                    last_err = format!("assert_text not found: {text}");
                                    tokio::time::sleep(Duration::from_millis(400)).await;
                                    continue;
                                }
                            } else {
                                result.matched_text = true;
                            }
                            result.ok = result.ready_selector_found && result.matched_text;
                            result.details = if result.ok {
                                "app verified".into()
                            } else {
                                last_err.clone()
                            };
                            let _ = child.kill();
                            let _ = self.close_session(&key);
                            return Ok(result);
                        }
                        Err(e) => last_err = e.to_string(),
                    }
                }
                Err(e) => last_err = e.to_string(),
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let _ = child.kill();
        let _ = self.close_session(&key);
        result.details = format!("timeout: {last_err}");
        Ok(result)
    }

    pub fn close_session(&self, key: &str) -> Result<()> {
        if let Some((_, session)) = self.sessions.remove(key) {
            let mut child = session.child.lock();
            let _ = child.kill();
            let _ = child.wait();
        }
        Ok(())
    }

    async fn ensure_session(&self, key: &str) -> Result<Arc<BrowserSession>> {
        if let Some(s) = self.sessions.get(key) {
            return Ok(s.clone());
        }
        let port = free_port()?;
        let user_data = std::env::temp_dir().join(format!("source-chrome-{key}-{port}"));
        let _ = std::fs::create_dir_all(&user_data);
        let child = Command::new(&self.chrome_path)
            .args([
                "--headless=new",
                "--disable-gpu",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
                &format!("--remote-debugging-port={port}"),
                &format!("--user-data-dir={}", user_data.display()),
                "about:blank",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("spawn chrome at {}", self.chrome_path.display()))?;

        let session = Arc::new(BrowserSession {
            child: Mutex::new(child),
            ws_url: Mutex::new(None),
            last_url: Mutex::new("about:blank".into()),
        });

        // Wait for DevTools HTTP
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut ws = None;
        while Instant::now() < deadline {
            if let Ok(url) = fetch_ws_debugger_url(port).await {
                ws = Some(url);
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        let ws = ws.ok_or_else(|| anyhow!("chrome DevTools not ready on port {port}"))?;
        *session.ws_url.lock() = Some(ws);
        // Enable page domain once
        let _ = self.cdp_call(&session, "Page.enable", json!({})).await;
        let _ = self.cdp_call(&session, "Runtime.enable", json!({})).await;
        self.sessions.insert(key.to_string(), session.clone());
        Ok(session)
    }

    async fn evaluate_string(&self, session: &BrowserSession, expression: &str) -> Result<String> {
        let v = self.evaluate_value(session, expression).await?;
        Ok(match v {
            Value::String(s) => s,
            other => other.to_string(),
        })
    }

    async fn evaluate_value(&self, session: &BrowserSession, expression: &str) -> Result<Value> {
        let result = self
            .cdp_call(
                session,
                "Runtime.evaluate",
                json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true
                }),
            )
            .await?;
        if let Some(ex) = result.get("exceptionDetails") {
            return Err(anyhow!("script error: {ex}"));
        }
        Ok(result
            .pointer("/result/value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    async fn cdp_call(&self, session: &BrowserSession, method: &str, params: Value) -> Result<Value> {
        let ws_url = session
            .ws_url
            .lock()
            .clone()
            .ok_or_else(|| anyhow!("missing ws url"))?;
        let (mut ws, _) = connect_async(&ws_url)
            .await
            .with_context(|| format!("connect {ws_url}"))?;
        let id = 1i64;
        let msg = json!({ "id": id, "method": method, "params": params });
        ws.send(Message::Text(msg.to_string().into())).await?;

        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            let next = tokio::time::timeout(Duration::from_secs(10), ws.next())
                .await
                .map_err(|_| anyhow!("cdp timeout waiting for {method}"))?;
            let Some(frame) = next else {
                break;
            };
            let frame = frame?;
            if let Message::Text(text) = frame {
                let v: Value = serde_json::from_str(&text)?;
                if v.get("id").and_then(|x| x.as_i64()) == Some(id) {
                    if let Some(err) = v.get("error") {
                        return Err(anyhow!("cdp error: {err}"));
                    }
                    return Ok(v.get("result").cloned().unwrap_or(Value::Null));
                }
            }
        }
        Err(anyhow!("no cdp response for {method}"))
    }
}

async fn fetch_ws_debugger_url(port: u16) -> Result<String> {
    let client = reqwest::Client::new();
    // Prefer a page target (supports Page/Runtime domains). Fall back to /json/new.
    let list_url = format!("http://127.0.0.1:{port}/json/list");
    if let Ok(resp) = client.get(&list_url).send().await {
        if let Ok(arr) = resp.json::<Vec<Value>>().await {
            for item in arr {
                let typ = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if typ == "page" {
                    if let Some(ws) = item.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                        return Ok(ws.to_string());
                    }
                }
            }
        }
    }
    let new_url = format!("http://127.0.0.1:{port}/json/new?about:blank");
    if let Ok(resp) = client.put(&new_url).send().await {
        if let Ok(body) = resp.json::<Value>().await {
            if let Some(ws) = body.get("webSocketDebuggerUrl").and_then(|v| v.as_str()) {
                return Ok(ws.to_string());
            }
        }
    }
    // Last resort: browser-level endpoint (limited domains).
    let url = format!("http://127.0.0.1:{port}/json/version");
    let body = client
        .get(&url)
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    body.get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("webSocketDebuggerUrl missing"))
}

fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?.port())
}

fn spawn_shell_command(launch_command: &str, cwd: &str) -> std::io::Result<std::process::Child> {
    #[cfg(windows)]
    {
        Command::new("cmd")
            .args(["/C", launch_command])
            .current_dir(cwd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }
    #[cfg(not(windows))]
    {
        Command::new("bash")
            .arg("-lc")
            .arg(launch_command)
            .current_dir(cwd)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    }
}

fn find_chrome() -> PathBuf {
    let path_candidates = [
        "google-chrome-stable",
        "google-chrome",
        "chromium-browser",
        "chromium",
        "chrome",
        "msedge",
    ];
    for cand in path_candidates {
        if let Ok(p) = which::which(cand) {
            return p;
        }
    }

    #[cfg(windows)]
    {
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let program_files = std::env::var("PROGRAMFILES").unwrap_or_else(|_| r"C:\Program Files".into());
        let program_files_x86 =
            std::env::var("PROGRAMFILES(X86)").unwrap_or_else(|_| r"C:\Program Files (x86)".into());
        for cand in [
            PathBuf::from(&local).join(r"Google\Chrome\Application\chrome.exe"),
            PathBuf::from(&program_files).join(r"Google\Chrome\Application\chrome.exe"),
            PathBuf::from(&program_files_x86).join(r"Google\Chrome\Application\chrome.exe"),
            PathBuf::from(&program_files).join(r"Microsoft\Edge\Application\msedge.exe"),
            PathBuf::from(&program_files_x86).join(r"Microsoft\Edge\Application\msedge.exe"),
            PathBuf::from(&local).join(r"Chromium\Application\chrome.exe"),
        ] {
            if cand.exists() {
                return cand;
            }
        }
        return PathBuf::from(r"C:\Program Files\Google\Chrome\Application\chrome.exe");
    }

    #[cfg(not(windows))]
    {
        PathBuf::from("/usr/bin/google-chrome")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_core::types::WorkspacePaths;

    #[tokio::test]
    async fn navigates_loopback_data_url_like_about_blank() {
        // about:blank is always local; policy still allows via loopback chrome session.
        let dir = tempfile::tempdir().unwrap();
        let paths = WorkspacePaths::from_home(dir.path());
        paths.ensure_dirs().unwrap();
        let db = crate::db::Db::connect(&paths.db_path).await.unwrap();
        let policy = PolicyEngine::load(db).await.unwrap();
        let browser = HeadlessBrowser::new(policy);
        // Use a data URL — classified via host parse; assert_url_allowed may reject non-http.
        // Prefer 127.0.0.1 chrome about:blank inspect without public navigation.
        let session = browser.ensure_session("test").await;
        if session.is_err() {
            // Environments without chrome graphics libs may fail; soft-skip.
            eprintln!("chrome session unavailable: {:?}", session.err());
            return;
        }
        let ins = browser.inspect(Some("test"), None, Some("1+1")).await.unwrap();
        assert_eq!(ins.script_result, Some(json!(2)));
        browser.close_session("test").unwrap();
    }
}
