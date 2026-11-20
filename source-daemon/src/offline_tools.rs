//! Built-in offline tool suite executors:
//! fs_ops, terminal_exec, local_rag_docs, headless_browser, app_verifier,
//! sqlite_db_inspector, git_ops, archive_and_system.

use anyhow::{anyhow, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde_json::{json, Value};
use source_core::policy::ApprovalCategory;
use source_core::types::{SessionId, ToolCall, ToolDefinition, ToolResult};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Column, Row, SqlitePool};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use tar::{Archive, Builder};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

use crate::analyzer::{SymbolSearchQuery, TreeSitterAnalyzer};
use crate::browser::HeadlessBrowser;
use crate::mcp_host::tool_allowed_for_tier;
use crate::policy::PolicyEngine;
use crate::pty_runner::PtyRunner;
use crate::rag::OfflineRag;
use crate::templates::TemplateManager;

pub struct OfflineToolContext<'a> {
    pub policy: &'a PolicyEngine,
    pub pty: &'a PtyRunner,
    pub rag: &'a OfflineRag,
    pub templates: &'a TemplateManager,
    pub browser: &'a HeadlessBrowser,
    pub analyzer: &'a TreeSitterAnalyzer,
    pub session_id: SessionId,
}

pub fn phase3_builtin_tools() -> Vec<ToolDefinition> {
    use ApprovalCategory::*;
    vec![
        ToolDefinition {
            name: "fs_ops".into(),
            description: "Filesystem ops: read/write/list/mkdir/remove/copy/move/stat. For desktop apps prefer templates_ops create_desktop_app (never substitute HTML-only).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["read","write","list","mkdir","remove","copy","move","stat"] },
                    "path": { "type": "string" },
                    "dest": { "type": "string" },
                    "content": { "type": "string" },
                    "recursive": { "type": "boolean" }
                },
                "required": ["op", "path"]
            }),
            categories: vec![FileEdits],
            enabled: true,
            offline_capable: true,
            requires_wan: false,
        },
        ToolDefinition {
            name: "terminal_exec".into(),
            description: "Persistent PTY terminal: open/write/read/close or one-shot exec".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["open","write","read","close","list","exec"] },
                    "pty_id": { "type": "string" },
                    "command": { "type": "string" },
                    "data": { "type": "string" },
                    "cwd": { "type": "string" },
                    "wait_ms": { "type": "integer" }
                },
                "required": ["op"]
            }),
            categories: vec![Shell],
            enabled: true,
            offline_capable: true,
            requires_wan: false,
        },
        ToolDefinition {
            name: "local_rag_docs".into(),
            description: "Index/query offline docs via sqlite-vec + Tantivy hybrid RAG".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["index","query","status"] },
                    "query": { "type": "string" },
                    "top_k": { "type": "integer" },
                    "force": { "type": "boolean" },
                    "docs_root": { "type": "string" }
                },
                "required": ["op"]
            }),
            categories: vec![],
            enabled: true,
            offline_capable: true,
            requires_wan: false,
        },
        ToolDefinition {
            name: "headless_browser".into(),
            description: "Chromium headless navigate/inspect (network-policy gated)".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["navigate","inspect","close"] },
                    "url": { "type": "string" },
                    "session_key": { "type": "string" },
                    "selector": { "type": "string" },
                    "script": { "type": "string" },
                    "wait_ms": { "type": "integer" }
                },
                "required": ["op"]
            }),
            categories: vec![Network],
            enabled: true,
            offline_capable: true,
            requires_wan: false,
        },
        ToolDefinition {
            name: "app_verifier".into(),
            description: "Closed-loop launch→navigate→inspect→assert for local apps. Supports kind=desktop to build/verify Tauri projects via scripts/build-desktop.sh (never abandon desktop for HTML-only).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "kind": { "type": "string", "enum": ["web", "desktop"] },
                    "launch_command": { "type": "string" },
                    "cwd": { "type": "string" },
                    "url": { "type": "string" },
                    "ready_selector": { "type": "string" },
                    "assert_text": { "type": "string" },
                    "timeout_ms": { "type": "integer" },
                    "install_deps": { "type": "boolean" },
                    "build_only": { "type": "boolean" }
                },
                "required": []
            }),
            categories: vec![Shell, Network],
            enabled: true,
            offline_capable: true,
            requires_wan: false,
        },
        ToolDefinition {
            name: "templates_ops".into(),
            description: "Offline app factory: list/scaffold templates, create_desktop_app (Tauri v2), build_desktop, linux_deps, windows_deps, vendor_status. REQUIRED for desktop requests — never refuse or substitute HTML-only.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "op": {
                        "type": "string",
                        "enum": [
                            "list",
                            "scaffold",
                            "create_desktop_app",
                            "build_desktop",
                            "linux_deps",
                            "windows_deps",
                            "vendor_status"
                        ]
                    },
                    "template_id": { "type": "string" },
                    "dest": { "type": "string" },
                    "name": { "type": "string" },
                    "path": { "type": "string" },
                    "install_deps": { "type": "boolean" }
                },
                "required": ["op"]
            }),
            categories: vec![FileEdits, Shell],
            enabled: true,
            offline_capable: true,
            requires_wan: false,
        },
        ToolDefinition {
            name: "sqlite_db_inspector".into(),
            description: "Inspect local SQLite databases: tables/schema/query".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["tables","schema","query"] },
                    "path": { "type": "string" },
                    "table": { "type": "string" },
                    "sql": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["op", "path"]
            }),
            categories: vec![],
            enabled: true,
            offline_capable: true,
            requires_wan: false,
        },
        ToolDefinition {
            name: "git_ops".into(),
            description: "Local git operations: status/diff/log/add/commit/branch".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["status","diff","log","add","commit","branch","rev_parse"] },
                    "cwd": { "type": "string" },
                    "message": { "type": "string" },
                    "paths": { "type": "array", "items": { "type": "string" } },
                    "ref_name": { "type": "string" },
                    "limit": { "type": "integer" }
                },
                "required": ["op"]
            }),
            categories: vec![FileEdits],
            enabled: true,
            offline_capable: true,
            requires_wan: false,
        },
        ToolDefinition {
            name: "archive_and_system".into(),
            description: "Archives (tar.gz/zip) and system info (uname, df, which, env)".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "op": { "type": "string", "enum": ["tar_create","tar_extract","zip_create","zip_extract","uname","df","which","env","disk_usage"] },
                    "path": { "type": "string" },
                    "dest": { "type": "string" },
                    "sources": { "type": "array", "items": { "type": "string" } },
                    "name": { "type": "string" }
                },
                "required": ["op"]
            }),
            categories: vec![Shell],
            enabled: true,
            offline_capable: true,
            requires_wan: false,
        },
        // Keep Phase-2 names as aliases still registered separately; also expose symbol search helper tool
        ToolDefinition {
            name: "symbol_search".into(),
            description: "Multi-file tree-sitter symbol search across a workspace".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "root": { "type": "string" },
                    "query": { "type": "string" },
                    "kind": { "type": "string" },
                    "language": { "type": "string" },
                    "path_glob": { "type": "string" },
                    "max_results": { "type": "integer" }
                },
                "required": ["root", "query"]
            }),
            categories: vec![],
            enabled: true,
            offline_capable: true,
            requires_wan: false,
        },
    ]
}

pub async fn execute_offline_tool(
    ctx: &OfflineToolContext<'_>,
    tool: &ToolDefinition,
    call: &ToolCall,
) -> ToolResult {
    // Tier filter
    let tier = ctx.policy.tier();
    if !tool_allowed_for_tier(tool.offline_capable, tool.requires_wan, tier) {
        return fail(
            call,
            &tool.name,
            format!(
                "tool {} blocked at network tier {} (offline_capable={}, requires_wan={})",
                tool.name,
                tier.as_str(),
                tool.offline_capable,
                tool.requires_wan
            ),
        );
    }

    let result = match tool.name.as_str() {
        "fs_ops" => exec_fs_ops(&call.arguments),
        "terminal_exec" => exec_terminal(ctx, &call.arguments).await,
        "local_rag_docs" => exec_rag(ctx, &call.arguments).await,
        "headless_browser" => exec_browser(ctx, &call.arguments).await,
        "app_verifier" => exec_verifier(ctx, &call.arguments).await,
        "templates_ops" => exec_templates(ctx, &call.arguments),
        "sqlite_db_inspector" => exec_sqlite(&call.arguments).await,
        "git_ops" => exec_git(&call.arguments),
        "archive_and_system" => exec_archive_system(&call.arguments),
        "symbol_search" => exec_symbol_search(ctx, &call.arguments),
        other => Err(anyhow!("not an offline suite tool: {other}")),
    };

    match result {
        Ok(output) => ToolResult {
            call_id: call.id.clone(),
            name: tool.name.clone(),
            ok: true,
            output,
            error: None,
        },
        Err(e) => fail(call, &tool.name, e.to_string()),
    }
}

fn fail(call: &ToolCall, name: &str, err: String) -> ToolResult {
    ToolResult {
        call_id: call.id.clone(),
        name: name.to_string(),
        ok: false,
        output: String::new(),
        error: Some(err),
    }
}

fn exec_fs_ops(args: &Value) -> Result<String> {
    let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let path = PathBuf::from(args.get("path").and_then(|v| v.as_str()).unwrap_or(""));
    match op {
        "read" => Ok(fs::read_to_string(&path)?),
        "write" => {
            let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, content)?;
            Ok(format!("wrote {} bytes", content.len()))
        }
        "list" => {
            let mut names = Vec::new();
            for e in fs::read_dir(&path)? {
                let e = e?;
                let file_type = if e.file_type()?.is_dir() { "dir" } else { "file" };
                names.push(format!("{}\t{}", file_type, e.file_name().to_string_lossy()));
            }
            names.sort();
            Ok(names.join("\n"))
        }
        "mkdir" => {
            let recursive = args.get("recursive").and_then(|v| v.as_bool()).unwrap_or(true);
            if recursive {
                fs::create_dir_all(&path)?;
            } else {
                fs::create_dir(&path)?;
            }
            Ok(format!("mkdir {}", path.display()))
        }
        "remove" => {
            let recursive = args.get("recursive").and_then(|v| v.as_bool()).unwrap_or(false);
            if path.is_dir() {
                if recursive {
                    fs::remove_dir_all(&path)?;
                } else {
                    fs::remove_dir(&path)?;
                }
            } else {
                fs::remove_file(&path)?;
            }
            Ok(format!("removed {}", path.display()))
        }
        "copy" => {
            let dest = PathBuf::from(args.get("dest").and_then(|v| v.as_str()).unwrap_or(""));
            if path.is_dir() {
                copy_dir(&path, &dest)?;
            } else {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&path, &dest)?;
            }
            Ok(format!("copied {} -> {}", path.display(), dest.display()))
        }
        "move" => {
            let dest = PathBuf::from(args.get("dest").and_then(|v| v.as_str()).unwrap_or(""));
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&path, &dest)?;
            Ok(format!("moved {} -> {}", path.display(), dest.display()))
        }
        "stat" => {
            let meta = fs::metadata(&path)?;
            Ok(serde_json::to_string_pretty(&json!({
                "path": path,
                "is_dir": meta.is_dir(),
                "is_file": meta.is_file(),
                "len": meta.len(),
                "readonly": meta.permissions().readonly(),
            }))?)
        }
        other => Err(anyhow!("unknown fs_ops op {other}")),
    }
}

async fn exec_terminal(ctx: &OfflineToolContext<'_>, args: &Value) -> Result<String> {
    let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "open" => {
            let info = ctx.pty.open(
                args.get("cwd").and_then(|v| v.as_str()).map(PathBuf::from),
                None,
                None,
                None,
                None,
            )?;
            Ok(serde_json::to_string_pretty(&info)?)
        }
        "write" => {
            let id = args.get("pty_id").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("pty_id required"))?;
            let data = args.get("data").and_then(|v| v.as_str()).unwrap_or("");
            ctx.pty.write(id, data)?;
            Ok("ok".into())
        }
        "read" => {
            let id = args.get("pty_id").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("pty_id required"))?;
            let wait = args.get("wait_ms").and_then(|v| v.as_u64());
            let r = ctx.pty.read(id, None, wait)?;
            Ok(serde_json::to_string_pretty(&r)?)
        }
        "close" => {
            let id = args.get("pty_id").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("pty_id required"))?;
            let info = ctx.pty.close(id)?;
            Ok(serde_json::to_string_pretty(&info)?)
        }
        "list" => Ok(serde_json::to_string_pretty(&ctx.pty.list())?),
        "exec" => {
            let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let cwd = args.get("cwd").and_then(|v| v.as_str());
            #[cfg(windows)]
            let mut cmd = {
                let mut c = Command::new("cmd");
                c.args(["/C", command]);
                c
            };
            #[cfg(not(windows))]
            let mut cmd = {
                let mut c = Command::new("bash");
                c.arg("-lc").arg(command);
                c
            };
            if let Some(c) = cwd {
                cmd.current_dir(c);
            }
            let out = cmd.output()?;
            Ok(serde_json::to_string_pretty(&json!({
                "exit_code": out.status.code(),
                "stdout": String::from_utf8_lossy(&out.stdout),
                "stderr": String::from_utf8_lossy(&out.stderr),
            }))?)
        }
        other => Err(anyhow!("unknown terminal_exec op {other}")),
    }
}

async fn exec_rag(ctx: &OfflineToolContext<'_>, args: &Value) -> Result<String> {
    let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "index" => {
            let docs = args
                .get("docs_root")
                .and_then(|v| v.as_str())
                .map(PathBuf::from);
            let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
            let status = ctx.rag.index_docs(docs, force).await?;
            Ok(serde_json::to_string_pretty(&status)?)
        }
        "query" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let top_k = args.get("top_k").and_then(|v| v.as_u64()).unwrap_or(8) as usize;
            let hits = ctx.rag.query(q, top_k, 0.5, 0.5).await?;
            Ok(serde_json::to_string_pretty(&hits)?)
        }
        "status" => Ok(serde_json::to_string_pretty(&ctx.rag.status().await?)?),
        other => Err(anyhow!("unknown local_rag_docs op {other}")),
    }
}

async fn exec_browser(ctx: &OfflineToolContext<'_>, args: &Value) -> Result<String> {
    let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "navigate" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let key = args.get("session_key").and_then(|v| v.as_str());
            let wait = args.get("wait_ms").and_then(|v| v.as_u64());
            let r = ctx
                .browser
                .navigate(url, key, wait, Some(ctx.session_id))
                .await
                .map_err(|e| anyhow!(e.to_string()))?;
            Ok(serde_json::to_string_pretty(&r)?)
        }
        "inspect" => {
            let key = args.get("session_key").and_then(|v| v.as_str());
            let selector = args.get("selector").and_then(|v| v.as_str());
            let script = args.get("script").and_then(|v| v.as_str());
            let r = ctx
                .browser
                .inspect(key, selector, script)
                .await
                .map_err(|e| anyhow!(e.to_string()))?;
            Ok(serde_json::to_string_pretty(&r)?)
        }
        "close" => {
            let key = args.get("session_key").and_then(|v| v.as_str()).unwrap_or("default");
            ctx.browser.close_session(key)?;
            Ok("closed".into())
        }
        other => Err(anyhow!("unknown headless_browser op {other}")),
    }
}

async fn exec_verifier(ctx: &OfflineToolContext<'_>, args: &Value) -> Result<String> {
    let kind = args
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("web");
    if kind == "desktop" {
        let cwd = args
            .get("cwd")
            .or_else(|| args.get("path"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("app_verifier kind=desktop requires cwd/path to a Tauri project"))?;
        let install_deps = args
            .get("install_deps")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let build = ctx
            .templates
            .build_desktop(Path::new(cwd), install_deps)?;
        let build_only = args
            .get("build_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if build_only || args.get("url").and_then(|v| v.as_str()).is_none() {
            return Ok(serde_json::to_string_pretty(&json!({
                "kind": "desktop",
                "build": build,
                "linux_deps": crate::templates::linux_tauri_apt_install_command(),
                "windows_deps": crate::templates::windows_tauri_deps_notes(),
                "note": "Desktop target retained. On Linux, install WebKitGTK/-dev if missing; on Windows 11 use MSVC + WebView2 and .\\scripts\\build-desktop.ps1 — do not switch to HTML-only."
            }))?);
        }
        // Optional: after build, also verify a URL if provided (e.g. tauri dev webview host).
    }

    let launch = args
        .get("launch_command")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
    if launch.is_empty() || url.is_empty() {
        return Err(anyhow!(
            "app_verifier web mode requires launch_command + url (or use kind=desktop with cwd)"
        ));
    }
    let r = ctx
        .browser
        .verify_app(
            launch,
            args.get("cwd").and_then(|v| v.as_str()),
            url,
            args.get("ready_selector").and_then(|v| v.as_str()),
            args.get("assert_text").and_then(|v| v.as_str()),
            args.get("timeout_ms").and_then(|v| v.as_u64()),
            Some(ctx.session_id),
        )
        .await
        .map_err(|e| anyhow!(e.to_string()))?;
    Ok(serde_json::to_string_pretty(&r)?)
}

fn exec_templates(ctx: &OfflineToolContext<'_>, args: &Value) -> Result<String> {
    let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "list" => Ok(serde_json::to_string_pretty(&ctx.templates.list()?)?),
        "scaffold" => {
            let template_id = args
                .get("template_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("template_id required"))?;
            let dest = args
                .get("dest")
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("dest required"))?;
            let name = args.get("name").and_then(|v| v.as_str());
            let path = ctx.templates.scaffold(template_id, Path::new(dest), name)?;
            Ok(serde_json::to_string_pretty(&json!({ "dest": path }))?)
        }
        "create_desktop_app" => {
            let dest = args
                .get("dest")
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("dest required for create_desktop_app"))?;
            let name = args.get("name").and_then(|v| v.as_str());
            let result = ctx.templates.create_desktop_app(Path::new(dest), name)?;
            Ok(serde_json::to_string_pretty(&result)?)
        }
        "build_desktop" => {
            let path = args
                .get("path")
                .or_else(|| args.get("dest"))
                .or_else(|| args.get("cwd"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("path required for build_desktop"))?;
            let install_deps = args
                .get("install_deps")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let result = ctx.templates.build_desktop(Path::new(path), install_deps)?;
            Ok(serde_json::to_string_pretty(&result)?)
        }
        "linux_deps" => Ok(serde_json::to_string_pretty(&json!({
            "packages": crate::templates::LINUX_TAURI_APT_PACKAGES,
            "apt_install": crate::templates::linux_tauri_apt_install_command(),
            "helper": "./scripts/build-desktop.sh --install-deps"
        }))?),
        "windows_deps" => Ok(serde_json::to_string_pretty(&json!({
            "notes": crate::templates::windows_tauri_deps_notes(),
            "helper": ".\\scripts\\build-desktop.ps1 -CheckDeps",
            "build": ".\\scripts\\build-desktop.ps1"
        }))?),
        "vendor_status" => Ok(serde_json::to_string_pretty(&ctx.templates.vendor_status()?)?),
        other => Err(anyhow!("unknown templates_ops op {other}")),
    }
}

async fn exec_sqlite(args: &Value) -> Result<String> {
    let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{path}"))?.read_only(true);
    let pool = SqlitePool::connect_with(options).await?;
    match op {
        "tables" => {
            let rows = sqlx::query(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .fetch_all(&pool)
            .await?;
            let names: Vec<String> = rows
                .into_iter()
                .map(|r| r.try_get::<String, _>("name"))
                .collect::<std::result::Result<_, _>>()?;
            Ok(names.join("\n"))
        }
        "schema" => {
            let table = args.get("table").and_then(|v| v.as_str());
            let sql = if let Some(t) = table {
                format!(
                    "SELECT sql FROM sqlite_master WHERE type IN ('table','index') AND tbl_name = '{}' ORDER BY type",
                    t.replace('\'', "''")
                )
            } else {
                "SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY name".into()
            };
            let rows = sqlx::query(&sql).fetch_all(&pool).await?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.try_get::<Option<String>, _>("sql")?.unwrap_or_default());
            }
            Ok(out.join(";\n\n"))
        }
        "query" => {
            let sql = args.get("sql").and_then(|v| v.as_str()).unwrap_or("");
            let trimmed = sql.trim_start().to_ascii_lowercase();
            if !(trimmed.starts_with("select") || trimmed.starts_with("with") || trimmed.starts_with("pragma")) {
                return Err(anyhow!("sqlite_db_inspector query only allows SELECT/WITH/PRAGMA"));
            }
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(100);
            let rows = sqlx::query(sql).fetch_all(&pool).await?;
            let mut out = Vec::new();
            for (i, row) in rows.into_iter().enumerate() {
                if i as u64 >= limit {
                    break;
                }
                let cols = row.columns();
                let mut map = serde_json::Map::new();
                for col in cols {
                    let name = col.name();
                    let val: Value = if let Ok(v) = row.try_get::<i64, _>(name) {
                        json!(v)
                    } else if let Ok(v) = row.try_get::<f64, _>(name) {
                        json!(v)
                    } else if let Ok(v) = row.try_get::<String, _>(name) {
                        json!(v)
                    } else if let Ok(v) = row.try_get::<Option<String>, _>(name) {
                        json!(v)
                    } else {
                        Value::Null
                    };
                    map.insert(name.to_string(), val);
                }
                out.push(Value::Object(map));
            }
            Ok(serde_json::to_string_pretty(&out)?)
        }
        other => Err(anyhow!("unknown sqlite_db_inspector op {other}")),
    }
}

fn exec_git(args: &Value) -> Result<String> {
    let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let cwd = args.get("cwd").and_then(|v| v.as_str()).unwrap_or(".");
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd);
    match op {
        "status" => {
            cmd.args(["status", "--porcelain=v1", "-b"]);
        }
        "diff" => {
            cmd.args(["diff"]);
        }
        "log" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);
            cmd.args(["log", &format!("-{limit}"), "--oneline", "--decorate"]);
        }
        "add" => {
            cmd.arg("add");
            if let Some(paths) = args.get("paths").and_then(|v| v.as_array()) {
                for p in paths {
                    if let Some(s) = p.as_str() {
                        cmd.arg(s);
                    }
                }
            } else {
                cmd.arg("-A");
            }
        }
        "commit" => {
            let msg = args.get("message").and_then(|v| v.as_str()).unwrap_or("source commit");
            cmd.args(["commit", "-m", msg]);
        }
        "branch" => {
            if let Some(name) = args.get("ref_name").and_then(|v| v.as_str()) {
                cmd.args(["branch", name]);
            } else {
                cmd.args(["branch", "-vv"]);
            }
        }
        "rev_parse" => {
            let r = args.get("ref_name").and_then(|v| v.as_str()).unwrap_or("HEAD");
            cmd.args(["rev-parse", r]);
        }
        other => return Err(anyhow!("unknown git_ops op {other}")),
    }
    let out = cmd.output()?;
    Ok(format!(
        "exit={}\n{}{}",
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

fn exec_archive_system(args: &Value) -> Result<String> {
    let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "tar_create" => {
            let dest = PathBuf::from(args.get("dest").and_then(|v| v.as_str()).unwrap_or("archive.tar.gz"));
            let sources = args
                .get("sources")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let file = File::create(&dest)?;
            let enc = GzEncoder::new(file, Compression::default());
            let mut builder = Builder::new(enc);
            for s in sources {
                let p = PathBuf::from(s.as_str().unwrap_or(""));
                if p.is_dir() {
                    builder.append_dir_all(p.file_name().unwrap_or_default(), &p)?;
                } else {
                    builder.append_path_with_name(&p, p.file_name().unwrap_or_default())?;
                }
            }
            builder.finish()?;
            Ok(format!("created {}", dest.display()))
        }
        "tar_extract" => {
            let path = PathBuf::from(args.get("path").and_then(|v| v.as_str()).unwrap_or(""));
            let dest = PathBuf::from(args.get("dest").and_then(|v| v.as_str()).unwrap_or("."));
            fs::create_dir_all(&dest)?;
            let file = File::open(&path)?;
            let dec = GzDecoder::new(file);
            let mut archive = Archive::new(dec);
            archive.unpack(&dest)?;
            Ok(format!("extracted {} -> {}", path.display(), dest.display()))
        }
        "zip_create" => {
            let dest = PathBuf::from(args.get("dest").and_then(|v| v.as_str()).unwrap_or("archive.zip"));
            let sources = args
                .get("sources")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let file = File::create(&dest)?;
            let mut zip = ZipWriter::new(file);
            let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for s in sources {
                let p = PathBuf::from(s.as_str().unwrap_or(""));
                if p.is_file() {
                    zip.start_file(p.file_name().unwrap().to_string_lossy(), opts)?;
                    let mut f = File::open(&p)?;
                    let mut buf = Vec::new();
                    f.read_to_end(&mut buf)?;
                    zip.write_all(&buf)?;
                } else if p.is_dir() {
                    for entry in WalkDir::new(&p).into_iter().filter_map(|e| e.ok()) {
                        if entry.file_type().is_file() {
                            let rel = entry.path().strip_prefix(&p)?;
                            zip.start_file(rel.to_string_lossy(), opts)?;
                            zip.write_all(&fs::read(entry.path())?)?;
                        }
                    }
                }
            }
            zip.finish()?;
            Ok(format!("created {}", dest.display()))
        }
        "zip_extract" => {
            let path = PathBuf::from(args.get("path").and_then(|v| v.as_str()).unwrap_or(""));
            let dest = PathBuf::from(args.get("dest").and_then(|v| v.as_str()).unwrap_or("."));
            fs::create_dir_all(&dest)?;
            let file = File::open(&path)?;
            let mut archive = ZipArchive::new(file)?;
            for i in 0..archive.len() {
                let mut file = archive.by_index(i)?;
                let outpath = dest.join(file.name());
                if file.name().ends_with('/') {
                    fs::create_dir_all(&outpath)?;
                } else {
                    if let Some(parent) = outpath.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    let mut outfile = File::create(&outpath)?;
                    std::io::copy(&mut file, &mut outfile)?;
                }
            }
            Ok(format!("extracted {} -> {}", path.display(), dest.display()))
        }
        "uname" => {
            #[cfg(windows)]
            {
                let out = Command::new("cmd")
                    .args(["/C", "ver"])
                    .output()
                    .or_else(|_| {
                        Command::new("powershell")
                            .args([
                                "-NoProfile",
                                "-Command",
                                "[System.Environment]::OSVersion.VersionString",
                            ])
                            .output()
                    })?;
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            }
            #[cfg(not(windows))]
            {
                let out = Command::new("uname").arg("-a").output()?;
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            }
        }
        "df" => {
            #[cfg(windows)]
            {
                let out = Command::new("powershell")
                    .args([
                        "-NoProfile",
                        "-Command",
                        "Get-PSDrive -PSProvider FileSystem | Format-Table -AutoSize | Out-String",
                    ])
                    .output()?;
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            }
            #[cfg(not(windows))]
            {
                let out = Command::new("df").args(["-h"]).output()?;
                Ok(String::from_utf8_lossy(&out.stdout).into_owned())
            }
        }
        "which" => {
            let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
            match which::which(name) {
                Ok(p) => Ok(p.display().to_string()),
                Err(e) => Err(anyhow!(e.to_string())),
            }
        }
        "env" => {
            let mut pairs: Vec<_> = std::env::vars().collect();
            pairs.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(pairs
                .into_iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("\n"))
        }
        "disk_usage" => {
            let path = PathBuf::from(args.get("path").and_then(|v| v.as_str()).unwrap_or("."));
            let bytes: u64 = WalkDir::new(&path)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
                .sum();
            Ok(json!({ "path": path, "bytes": bytes }).to_string())
        }
        other => Err(anyhow!("unknown archive_and_system op {other}")),
    }
}

fn exec_symbol_search(ctx: &OfflineToolContext<'_>, args: &Value) -> Result<String> {
    let root = PathBuf::from(args.get("root").and_then(|v| v.as_str()).unwrap_or("."));
    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let hits = ctx.analyzer.search(
        &root,
        SymbolSearchQuery {
            query,
            kind: args.get("kind").and_then(|v| v.as_str()).map(|s| s.to_string()),
            language: args
                .get("language")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            path_glob: args
                .get("path_glob")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            max_results: args
                .get("max_results")
                .and_then(|v| v.as_u64())
                .unwrap_or(50) as usize,
        },
    )?;
    Ok(serde_json::to_string_pretty(&hits)?)
}

fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    for entry in WalkDir::new(src) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src)?;
        let target = dest.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
