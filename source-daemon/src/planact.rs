//! Plan/Act state machine — Plan = read-only; Act = execution with per-category approval gates.

use anyhow::Result;
use chrono::Utc;
use parking_lot::RwLock;
use source_core::error::CoreError;
use source_core::policy::{ApprovalCategory, ApprovalDecision};
use source_core::types::{
    AuditEvent, AuditEventKind, Mode, SessionId, ToolCall, ToolDefinition, ToolResult,
};
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

use crate::db::Db;
use crate::policy::PolicyEngine;
use crate::shadow::ShadowGit;

#[derive(Clone)]
pub struct PlanActMachine {
    db: Db,
    policy: PolicyEngine,
    shadow: Arc<ShadowGit>,
    /// Categories that auto-approve in Act mode (session-scoped overrides can be added later).
    auto_approve: Arc<RwLock<HashSet<ApprovalCategory>>>,
}

impl PlanActMachine {
    pub fn new(db: Db, policy: PolicyEngine, shadow: Arc<ShadowGit>) -> Self {
        // Conservative defaults: nothing auto-approves until user opts in via resolve.
        // File edits / shell / network / mcp / cross-node all require gates in Act.
        let auto = HashSet::new();
        Self {
            db,
            policy,
            shadow,
            auto_approve: Arc::new(RwLock::new(auto)),
        }
    }

    pub async fn set_mode(&self, session_id: SessionId, mode: Mode) -> Result<Mode, CoreError> {
        let session = self
            .db
            .get_session(&session_id)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?
            .ok_or_else(|| CoreError::SessionNotFound(session_id.to_string()))?;

        if session.mode == mode {
            return Ok(mode);
        }

        self.db
            .update_session_mode(&session_id, mode)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;

        let _ = self
            .db
            .append_audit(AuditEvent {
                id: Uuid::new_v4(),
                session_id: Some(session_id),
                kind: AuditEventKind::ModeChange,
                summary: format!("mode {} -> {}", session.mode, mode),
                detail: serde_json::json!({
                    "from": session.mode.as_str(),
                    "to": mode.as_str(),
                }),
                content_hash: String::new(),
                prev_hash: None,
                created_at: Utc::now(),
            })
            .await;

        Ok(mode)
    }

    pub fn is_read_only_tool(name: &str) -> bool {
        matches!(
            name,
            "read_file"
                | "list_dir"
                | "search"
                | "grep"
                | "repo_map"
                | "status"
                | "evaluate_network"
                | "echo"
        )
    }

    pub async fn gate_categories(
        &self,
        session_id: SessionId,
        categories: &[ApprovalCategory],
        summary: &str,
        detail: serde_json::Value,
    ) -> Result<(), CoreError> {
        let session = self
            .db
            .get_session(&session_id)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?
            .ok_or_else(|| CoreError::SessionNotFound(session_id.to_string()))?;

        match session.mode {
            Mode::Plan => {
                // Plan is read-only: only allow if every category is absent (pure read) —
                // callers should use is_read_only_tool for tools. Any gated category is denied.
                if categories.is_empty() {
                    return Ok(());
                }
                Err(CoreError::InvalidModeTransition {
                    from: "plan".into(),
                    to: format!("execute:{summary}"),
                })
            }
            Mode::Act => {
                for cat in categories {
                    if self.auto_approve.read().contains(cat) {
                        continue;
                    }
                    // Create pending approval and require resolve before proceeding.
                    let id = Uuid::new_v4();
                    self.db
                        .insert_approval(id, Some(&session_id), *cat, summary, &detail)
                        .await
                        .map_err(|e| CoreError::Other(e.to_string()))?;
                    let _ = self
                        .db
                        .append_audit(AuditEvent {
                            id: Uuid::new_v4(),
                            session_id: Some(session_id),
                            kind: AuditEventKind::Approval,
                            summary: format!("approval required: {} — {summary}", cat.as_str()),
                            detail: serde_json::json!({
                                "approval_id": id,
                                "category": cat.as_str(),
                                "detail": detail,
                            }),
                            content_hash: String::new(),
                            prev_hash: None,
                            created_at: Utc::now(),
                        })
                        .await;
                    return Err(CoreError::ApprovalRequired(id.to_string()));
                }
                Ok(())
            }
        }
    }

    pub async fn resolve_approval(
        &self,
        approval_id: Uuid,
        decision: ApprovalDecision,
    ) -> Result<(), CoreError> {
        let ok = self
            .db
            .resolve_approval(approval_id, decision)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        if !ok {
            return Err(CoreError::InvalidArgument(format!(
                "approval {approval_id} not pending"
            )));
        }
        let _ = self
            .db
            .append_audit(AuditEvent {
                id: Uuid::new_v4(),
                session_id: None,
                kind: AuditEventKind::Approval,
                summary: format!("approval {approval_id} -> {:?}", decision),
                detail: serde_json::json!({
                    "approval_id": approval_id,
                    "decision": decision,
                }),
                content_hash: String::new(),
                prev_hash: None,
                created_at: Utc::now(),
            })
            .await;
        if matches!(decision, ApprovalDecision::Denied) {
            return Err(CoreError::ApprovalDenied(approval_id.to_string()));
        }
        Ok(())
    }

    pub fn set_auto_approve(&self, category: ApprovalCategory, enabled: bool) {
        let mut guard = self.auto_approve.write();
        if enabled {
            guard.insert(category);
        } else {
            guard.remove(&category);
        }
    }

    /// Invoke a tool through Plan/Act + network gates. Implements Phase-2 built-ins fully.
    pub async fn invoke_tool(
        &self,
        session_id: SessionId,
        call: ToolCall,
    ) -> Result<ToolResult, CoreError> {
        let tool = self
            .db
            .get_tool(&call.name)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?
            .ok_or_else(|| CoreError::InvalidArgument(format!("unknown tool {}", call.name)))?;

        if !tool.enabled {
            return Err(CoreError::InvalidArgument(format!(
                "tool {} disabled",
                tool.name
            )));
        }

        let read_only = Self::is_read_only_tool(&tool.name);
        if !read_only {
            self.gate_categories(
                session_id,
                &tool.categories,
                &format!("tool {}", tool.name),
                call.arguments.clone(),
            )
            .await?;
        }

        // Network category tools must pass policy interceptor.
        if tool.categories.contains(&ApprovalCategory::Network) {
            if let Some(host) = call.arguments.get("host").and_then(|v| v.as_str()) {
                self.policy
                    .intercept(host, None, Some(session_id), &format!("tool:{}", tool.name))
                    .await?;
            }
            if let Some(url) = call.arguments.get("url").and_then(|v| v.as_str()) {
                self.policy
                    .assert_url_allowed(url, Some(session_id), &format!("tool:{}", tool.name))
                    .await?;
            }
        }

        let result = self.execute_builtin(&tool, &call, session_id).await;

        let _ = self
            .db
            .append_audit(AuditEvent {
                id: Uuid::new_v4(),
                session_id: Some(session_id),
                kind: AuditEventKind::Tool,
                summary: format!(
                    "tool {} -> {}",
                    call.name,
                    if result.ok { "ok" } else { "err" }
                ),
                detail: serde_json::json!({
                    "call": call,
                    "ok": result.ok,
                    "error": result.error,
                }),
                content_hash: String::new(),
                prev_hash: None,
                created_at: Utc::now(),
            })
            .await;

        Ok(result)
    }

    async fn execute_builtin(
        &self,
        tool: &ToolDefinition,
        call: &ToolCall,
        session_id: SessionId,
    ) -> ToolResult {
        match tool.name.as_str() {
            "echo" => ToolResult {
                call_id: call.id.clone(),
                name: tool.name.clone(),
                ok: true,
                output: call
                    .arguments
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                error: None,
            },
            "read_file" => {
                let path = call
                    .arguments
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match std::fs::read_to_string(path) {
                    Ok(content) => ToolResult {
                        call_id: call.id.clone(),
                        name: tool.name.clone(),
                        ok: true,
                        output: content,
                        error: None,
                    },
                    Err(e) => ToolResult {
                        call_id: call.id.clone(),
                        name: tool.name.clone(),
                        ok: false,
                        output: String::new(),
                        error: Some(e.to_string()),
                    },
                }
            }
            "list_dir" => {
                let path = call
                    .arguments
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or(".");
                match std::fs::read_dir(path) {
                    Ok(rd) => {
                        let mut names = Vec::new();
                        for entry in rd.flatten() {
                            names.push(entry.file_name().to_string_lossy().into_owned());
                        }
                        names.sort();
                        ToolResult {
                            call_id: call.id.clone(),
                            name: tool.name.clone(),
                            ok: true,
                            output: names.join("\n"),
                            error: None,
                        }
                    }
                    Err(e) => ToolResult {
                        call_id: call.id.clone(),
                        name: tool.name.clone(),
                        ok: false,
                        output: String::new(),
                        error: Some(e.to_string()),
                    },
                }
            }
            "write_file" => {
                let path = call
                    .arguments
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let content = call
                    .arguments
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match std::fs::write(path, content) {
                    Ok(()) => {
                        let _ = self
                            .shadow
                            .audit_shell(Some(session_id), &format!("write_file {path}"), 0, "")
                            .await;
                        ToolResult {
                            call_id: call.id.clone(),
                            name: tool.name.clone(),
                            ok: true,
                            output: format!("wrote {} bytes to {path}", content.len()),
                            error: None,
                        }
                    }
                    Err(e) => ToolResult {
                        call_id: call.id.clone(),
                        name: tool.name.clone(),
                        ok: false,
                        output: String::new(),
                        error: Some(e.to_string()),
                    },
                }
            }
            "shell" => {
                let command = call
                    .arguments
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let output = std::process::Command::new("bash")
                    .arg("-lc")
                    .arg(command)
                    .output();
                match output {
                    Ok(out) => {
                        let code = out.status.code().unwrap_or(-1);
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        let combined = format!("{stdout}{stderr}");
                        let _ = self
                            .shadow
                            .audit_shell(
                                Some(session_id),
                                command,
                                code,
                                &combined.chars().rev().take(2000).collect::<String>().chars().rev().collect::<String>(),
                            )
                            .await;
                        ToolResult {
                            call_id: call.id.clone(),
                            name: tool.name.clone(),
                            ok: out.status.success(),
                            output: combined,
                            error: if out.status.success() {
                                None
                            } else {
                                Some(format!("exit {code}"))
                            },
                        }
                    }
                    Err(e) => ToolResult {
                        call_id: call.id.clone(),
                        name: tool.name.clone(),
                        ok: false,
                        output: String::new(),
                        error: Some(e.to_string()),
                    },
                }
            }
            "evaluate_network" => {
                let host = call
                    .arguments
                    .get("host")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let decision = self.policy.evaluate(host, None);
                ToolResult {
                    call_id: call.id.clone(),
                    name: tool.name.clone(),
                    ok: true,
                    output: serde_json::to_string_pretty(&decision).unwrap_or_default(),
                    error: None,
                }
            }
            "fetch_url" => {
                let url = call
                    .arguments
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                // Policy already checked above when Network category present.
                match reqwest::Client::new().get(&url).send().await {
                    Ok(resp) => {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        ToolResult {
                            call_id: call.id.clone(),
                            name: tool.name.clone(),
                            ok: status.is_success(),
                            output: text.chars().take(16_384).collect(),
                            error: if status.is_success() {
                                None
                            } else {
                                Some(status.to_string())
                            },
                        }
                    }
                    Err(e) => ToolResult {
                        call_id: call.id.clone(),
                        name: tool.name.clone(),
                        ok: false,
                        output: String::new(),
                        error: Some(e.to_string()),
                    },
                }
            }
            other => ToolResult {
                call_id: call.id.clone(),
                name: tool.name.clone(),
                ok: false,
                output: String::new(),
                error: Some(format!("no builtin executor for {other}")),
            },
        }
    }

    pub async fn register_builtin_tools(&self) -> Result<()> {
        let tools = builtin_tools();
        for t in tools {
            self.db.register_tool(&t).await?;
        }
        Ok(())
    }
}

fn builtin_tools() -> Vec<ToolDefinition> {
    use source_core::policy::ApprovalCategory::*;
    vec![
        ToolDefinition {
            name: "echo".into(),
            description: "Echo text back (read-only)".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"]
            }),
            categories: vec![],
            enabled: true,
        },
        ToolDefinition {
            name: "read_file".into(),
            description: "Read a UTF-8 file".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
            categories: vec![],
            enabled: true,
        },
        ToolDefinition {
            name: "list_dir".into(),
            description: "List directory entries".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
            categories: vec![],
            enabled: true,
        },
        ToolDefinition {
            name: "write_file".into(),
            description: "Write a UTF-8 file".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            }),
            categories: vec![FileEdits],
            enabled: true,
        },
        ToolDefinition {
            name: "shell".into(),
            description: "Run a shell command".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"]
            }),
            categories: vec![Shell],
            enabled: true,
        },
        ToolDefinition {
            name: "evaluate_network".into(),
            description: "Evaluate host against current network tier".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "host": { "type": "string" } },
                "required": ["host"]
            }),
            categories: vec![],
            enabled: true,
        },
        ToolDefinition {
            name: "fetch_url".into(),
            description: "HTTP GET a URL (network gated)".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "url": { "type": "string" } },
                "required": ["url"]
            }),
            categories: vec![Network],
            enabled: true,
        },
    ]
}