use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use source_core::policy::{ApprovalCategory, ApprovalDecision, NetworkPolicy, NetworkTier};
use source_core::types::{
    AuditEvent, AuditEventKind, CheckpointId, CheckpointMeta, LlmProviderId, LlmProviderKind, Mode,
    ProviderConfig, SessionId, SessionMeta, ToolDefinition,
};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Clone)]
pub struct Db {
    pool: SqlitePool,
}

impl Db {
    pub async fn connect(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(8)
            .connect_with(options)
            .await
            .with_context(|| format!("connect sqlite {}", db_path.display()))?;

        let db = Self { pool };
        db.migrate().await?;
        db.ensure_default_policy().await?;
        db.ensure_default_providers().await?;
        Ok(db)
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    async fn migrate(&self) -> Result<()> {
        let sql = include_str!("../migrations/001_init.sql");
        // sqlx migrate! needs a folder at compile time; execute bundled SQL for portability.
        for stmt in split_sql(sql) {
            sqlx::query(stmt).execute(&self.pool).await.with_context(|| {
                format!("migration statement failed: {}", &stmt[..stmt.len().min(80)])
            })?;
        }
        Ok(())
    }

    async fn ensure_default_policy(&self) -> Result<()> {
        let row = sqlx::query("SELECT id FROM network_policy WHERE id = 1")
            .fetch_optional(&self.pool)
            .await?;
        if row.is_none() {
            let now = Utc::now().to_rfc3339();
            sqlx::query(
                "INSERT INTO network_policy (id, tier, allowlist_json, denylist_json, require_approval_for_public, updated_at)
                 VALUES (1, ?, '[]', '[]', 1, ?)",
            )
            .bind(NetworkTier::LocalOnly.as_str())
            .bind(now)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    async fn ensure_default_providers(&self) -> Result<()> {
        let defaults = default_providers();
        for p in defaults {
            let exists = sqlx::query("SELECT id FROM providers WHERE id = ?")
                .bind(&p.id.0)
                .fetch_optional(&self.pool)
                .await?;
            if exists.is_none() {
                self.upsert_provider(&p).await?;
            }
        }
        Ok(())
    }

    pub async fn load_network_policy(&self) -> Result<NetworkPolicy> {
        let row = sqlx::query(
            "SELECT tier, allowlist_json, denylist_json, require_approval_for_public FROM network_policy WHERE id = 1",
        )
        .fetch_one(&self.pool)
        .await?;
        let tier: String = row.try_get("tier")?;
        let allowlist: Vec<String> = serde_json::from_str(&row.try_get::<String, _>("allowlist_json")?)?;
        let denylist: Vec<String> = serde_json::from_str(&row.try_get::<String, _>("denylist_json")?)?;
        let require: i64 = row.try_get("require_approval_for_public")?;
        Ok(NetworkPolicy {
            tier: NetworkTier::from_str(&tier)?,
            allowlist,
            denylist,
            require_approval_for_public: require != 0,
        })
    }

    pub async fn save_network_policy(&self, policy: &NetworkPolicy) -> Result<()> {
        sqlx::query(
            "UPDATE network_policy SET tier = ?, allowlist_json = ?, denylist_json = ?, require_approval_for_public = ?, updated_at = ? WHERE id = 1",
        )
        .bind(policy.tier.as_str())
        .bind(serde_json::to_string(&policy.allowlist)?)
        .bind(serde_json::to_string(&policy.denylist)?)
        .bind(if policy.require_approval_for_public { 1 } else { 0 })
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn create_session(&self, meta: &SessionMeta) -> Result<()> {
        sqlx::query(
            "INSERT INTO sessions (id, title, mode, workspace_root, network_tier, provider_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(meta.id.to_string())
        .bind(&meta.title)
        .bind(meta.mode.as_str())
        .bind(meta.workspace_root.to_string_lossy().to_string())
        .bind(meta.network_tier.as_str())
        .bind(meta.provider_id.as_ref().map(|p| p.0.clone()))
        .bind(meta.created_at.to_rfc3339())
        .bind(meta.updated_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionMeta>> {
        let rows = sqlx::query(
            "SELECT id, title, mode, workspace_root, network_tier, provider_id, created_at, updated_at FROM sessions ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_session).collect()
    }

    pub async fn get_session(&self, id: &SessionId) -> Result<Option<SessionMeta>> {
        let row = sqlx::query(
            "SELECT id, title, mode, workspace_root, network_tier, provider_id, created_at, updated_at FROM sessions WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_session).transpose()
    }

    pub async fn update_session_mode(&self, id: &SessionId, mode: Mode) -> Result<()> {
        sqlx::query("UPDATE sessions SET mode = ?, updated_at = ? WHERE id = ?")
            .bind(mode.as_str())
            .bind(Utc::now().to_rfc3339())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn upsert_provider(&self, p: &ProviderConfig) -> Result<()> {
        sqlx::query(
            "INSERT INTO providers (id, kind, display_name, base_url, model, api_key_env, enabled, extra_headers_json, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
               kind=excluded.kind,
               display_name=excluded.display_name,
               base_url=excluded.base_url,
               model=excluded.model,
               api_key_env=excluded.api_key_env,
               enabled=excluded.enabled,
               extra_headers_json=excluded.extra_headers_json,
               updated_at=excluded.updated_at",
        )
        .bind(&p.id.0)
        .bind(format!("{:?}", p.kind).to_ascii_lowercase())
        .bind(&p.display_name)
        .bind(&p.base_url)
        .bind(&p.model)
        .bind(&p.api_key_env)
        .bind(if p.enabled { 1 } else { 0 })
        .bind(serde_json::to_string(&p.extra_headers)?)
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_provider_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        sqlx::query("UPDATE providers SET enabled = ?, updated_at = ? WHERE id = ?")
            .bind(if enabled { 1 } else { 0 })
            .bind(Utc::now().to_rfc3339())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn list_providers(&self) -> Result<Vec<ProviderConfig>> {
        let rows = sqlx::query(
            "SELECT id, kind, display_name, base_url, model, api_key_env, enabled, extra_headers_json FROM providers ORDER BY id",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_provider).collect()
    }

    pub async fn get_provider(&self, id: &str) -> Result<Option<ProviderConfig>> {
        let row = sqlx::query(
            "SELECT id, kind, display_name, base_url, model, api_key_env, enabled, extra_headers_json FROM providers WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_provider).transpose()
    }

    pub async fn latest_audit_hash(&self) -> Result<Option<String>> {
        let row = sqlx::query("SELECT content_hash FROM audit_logs ORDER BY created_at DESC, id DESC LIMIT 1")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.try_get("content_hash")).transpose()?)
    }

    pub async fn append_audit(&self, mut event: AuditEvent) -> Result<AuditEvent> {
        let prev = self.latest_audit_hash().await?;
        event.prev_hash = prev.clone();
        event.content_hash =
            AuditEvent::hash_payload(event.kind, &event.summary, &event.detail, prev.as_deref());
        sqlx::query(
            "INSERT INTO audit_logs (id, session_id, kind, summary, detail_json, content_hash, prev_hash, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(event.id.to_string())
        .bind(event.session_id.as_ref().map(|s| s.to_string()))
        .bind(format!("{:?}", event.kind).to_ascii_lowercase())
        .bind(&event.summary)
        .bind(event.detail.to_string())
        .bind(&event.content_hash)
        .bind(&event.prev_hash)
        .bind(event.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(event)
    }

    pub async fn list_audit(
        &self,
        session_id: Option<&SessionId>,
        limit: u32,
    ) -> Result<Vec<AuditEvent>> {
        let limit = limit.clamp(1, 1000) as i64;
        let rows = if let Some(sid) = session_id {
            sqlx::query(
                "SELECT id, session_id, kind, summary, detail_json, content_hash, prev_hash, created_at
                 FROM audit_logs WHERE session_id = ? ORDER BY created_at DESC LIMIT ?",
            )
            .bind(sid.to_string())
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query(
                "SELECT id, session_id, kind, summary, detail_json, content_hash, prev_hash, created_at
                 FROM audit_logs ORDER BY created_at DESC LIMIT ?",
            )
            .bind(limit)
            .fetch_all(&self.pool)
            .await?
        };
        rows.into_iter().map(row_to_audit).collect()
    }

    pub async fn insert_checkpoint(&self, meta: &CheckpointMeta) -> Result<()> {
        sqlx::query(
            "INSERT INTO checkpoints (id, session_id, step_index, git_commit, label, created_at, audit_event_id)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(meta.id.to_string())
        .bind(meta.session_id.to_string())
        .bind(meta.step_index as i64)
        .bind(&meta.git_commit)
        .bind(&meta.label)
        .bind(meta.created_at.to_rfc3339())
        .bind(meta.audit_event_id.map(|u| u.to_string()))
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn next_checkpoint_step(&self, session_id: &SessionId) -> Result<u64> {
        let row = sqlx::query("SELECT COALESCE(MAX(step_index), -1) AS m FROM checkpoints WHERE session_id = ?")
            .bind(session_id.to_string())
            .fetch_one(&self.pool)
            .await?;
        let m: i64 = row.try_get("m")?;
        Ok((m + 1) as u64)
    }

    pub async fn list_checkpoints(&self, session_id: &SessionId) -> Result<Vec<CheckpointMeta>> {
        let rows = sqlx::query(
            "SELECT id, session_id, step_index, git_commit, label, created_at, audit_event_id FROM checkpoints WHERE session_id = ? ORDER BY step_index",
        )
        .bind(session_id.to_string())
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_checkpoint).collect()
    }

    pub async fn get_checkpoint(&self, id: &CheckpointId) -> Result<Option<CheckpointMeta>> {
        let row = sqlx::query(
            "SELECT id, session_id, step_index, git_commit, label, created_at, audit_event_id FROM checkpoints WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_checkpoint).transpose()
    }

    pub async fn register_tool(&self, tool: &ToolDefinition) -> Result<()> {
        sqlx::query(
            "INSERT INTO tool_registry (name, description, input_schema_json, categories_json, enabled, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(name) DO UPDATE SET
               description=excluded.description,
               input_schema_json=excluded.input_schema_json,
               categories_json=excluded.categories_json,
               enabled=excluded.enabled,
               updated_at=excluded.updated_at",
        )
        .bind(&tool.name)
        .bind(&tool.description)
        .bind(tool.input_schema.to_string())
        .bind(serde_json::to_string(&tool.categories)?)
        .bind(if tool.enabled { 1 } else { 0 })
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_tools(&self) -> Result<Vec<ToolDefinition>> {
        let rows = sqlx::query(
            "SELECT name, description, input_schema_json, categories_json, enabled FROM tool_registry ORDER BY name",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(row_to_tool).collect()
    }

    pub async fn get_tool(&self, name: &str) -> Result<Option<ToolDefinition>> {
        let row = sqlx::query(
            "SELECT name, description, input_schema_json, categories_json, enabled FROM tool_registry WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_tool).transpose()
    }

    pub async fn insert_approval(
        &self,
        id: Uuid,
        session_id: Option<&SessionId>,
        category: ApprovalCategory,
        summary: &str,
        detail: &Value,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO approvals (id, session_id, category, summary, detail_json, status, created_at)
             VALUES (?, ?, ?, ?, ?, 'pending', ?)",
        )
        .bind(id.to_string())
        .bind(session_id.map(|s| s.to_string()))
        .bind(category.as_str())
        .bind(summary)
        .bind(detail.to_string())
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn resolve_approval(&self, id: Uuid, decision: ApprovalDecision) -> Result<bool> {
        let status = match decision {
            ApprovalDecision::Approved => "approved",
            ApprovalDecision::Denied => "denied",
            ApprovalDecision::Deferred => "deferred",
        };
        let res = sqlx::query(
            "UPDATE approvals SET status = ?, resolved_at = ? WHERE id = ? AND status = 'pending'",
        )
        .bind(status)
        .bind(Utc::now().to_rfc3339())
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn list_pending_approvals(&self) -> Result<Vec<Value>> {
        let rows = sqlx::query(
            "SELECT id, session_id, category, summary, detail_json, created_at FROM approvals WHERE status = 'pending' ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::json!({
                "id": row.try_get::<String, _>("id")?,
                "session_id": row.try_get::<Option<String>, _>("session_id")?,
                "category": row.try_get::<String, _>("category")?,
                "summary": row.try_get::<String, _>("summary")?,
                "detail": serde_json::from_str::<Value>(&row.try_get::<String, _>("detail_json")?)?,
                "created_at": row.try_get::<String, _>("created_at")?,
            }));
        }
        Ok(out)
    }

    pub async fn upsert_rag_meta(
        &self,
        id: &str,
        workspace_root: &Path,
        index_path: &Path,
        chunk_count: i64,
        file_count: i64,
        status: &str,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO rag_index_meta (id, workspace_root, index_path, chunk_count, file_count, last_built_at, status, meta_json)
             VALUES (?, ?, ?, ?, ?, ?, ?, '{}')
             ON CONFLICT(id) DO UPDATE SET
               workspace_root=excluded.workspace_root,
               index_path=excluded.index_path,
               chunk_count=excluded.chunk_count,
               file_count=excluded.file_count,
               last_built_at=excluded.last_built_at,
               status=excluded.status",
        )
        .bind(id)
        .bind(workspace_root.to_string_lossy().to_string())
        .bind(index_path.to_string_lossy().to_string())
        .bind(chunk_count)
        .bind(file_count)
        .bind(Utc::now().to_rfc3339())
        .bind(status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_kv(&self, key: &str, value: &Value) -> Result<()> {
        sqlx::query(
            "INSERT INTO kv_state (key, value_json, updated_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, updated_at=excluded.updated_at",
        )
        .bind(key)
        .bind(value.to_string())
        .bind(Utc::now().to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_kv(&self, key: &str) -> Result<Option<Value>> {
        let row = sqlx::query("SELECT value_json FROM kv_state WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some(r) => Ok(Some(serde_json::from_str(&r.try_get::<String, _>("value_json")?)?)),
            None => Ok(None),
        }
    }

    /// Restore SQLite-tracked session mode / kv markers associated with a checkpoint step.
    pub async fn restore_session_marker(&self, session_id: &SessionId, step_index: u64) -> Result<()> {
        self.set_kv(
            &format!("session:{}:restored_step", session_id),
            &serde_json::json!({ "step_index": step_index, "at": Utc::now().to_rfc3339() }),
        )
        .await
    }
}

fn split_sql(sql: &str) -> Vec<&str> {
    sql.split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with("--") && !s.starts_with("PRAGMA"))
        .collect()
}

fn parse_rfc3339(s: &str) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(s)?.with_timezone(&Utc))
}

fn row_to_session(row: sqlx::sqlite::SqliteRow) -> Result<SessionMeta> {
    let mode = row.try_get::<String, _>("mode")?;
    let tier = row.try_get::<String, _>("network_tier")?;
    let provider: Option<String> = row.try_get("provider_id")?;
    Ok(SessionMeta {
        id: SessionId(Uuid::parse_str(&row.try_get::<String, _>("id")?)?),
        title: row.try_get("title")?,
        mode: if mode == "act" { Mode::Act } else { Mode::Plan },
        workspace_root: PathBuf::from(row.try_get::<String, _>("workspace_root")?),
        created_at: parse_rfc3339(&row.try_get::<String, _>("created_at")?)?,
        updated_at: parse_rfc3339(&row.try_get::<String, _>("updated_at")?)?,
        network_tier: NetworkTier::from_str(&tier)?,
        provider_id: provider.map(LlmProviderId::new),
    })
}

fn parse_kind(s: &str) -> LlmProviderKind {
    match s.to_ascii_lowercase().as_str() {
        "openai_compatible" | "openaicompatible" => LlmProviderKind::OpenaiCompatible,
        "ollama" => LlmProviderKind::Ollama,
        "llamacpp" | "llama_cpp" => LlmProviderKind::LlamaCpp,
        "lmstudio" | "lm_studio" => LlmProviderKind::LmStudio,
        "anthropic" => LlmProviderKind::Anthropic,
        "openai" => LlmProviderKind::Openai,
        "gemini" => LlmProviderKind::Gemini,
        "openrouter" => LlmProviderKind::OpenRouter,
        _ => LlmProviderKind::Custom,
    }
}

fn row_to_provider(row: sqlx::sqlite::SqliteRow) -> Result<ProviderConfig> {
    let headers: HashMap<String, String> =
        serde_json::from_str(&row.try_get::<String, _>("extra_headers_json")?)?;
    let enabled: i64 = row.try_get("enabled")?;
    Ok(ProviderConfig {
        id: LlmProviderId::new(row.try_get::<String, _>("id")?),
        kind: parse_kind(&row.try_get::<String, _>("kind")?),
        display_name: row.try_get("display_name")?,
        base_url: row.try_get("base_url")?,
        model: row.try_get("model")?,
        api_key_env: row.try_get("api_key_env")?,
        enabled: enabled != 0,
        extra_headers: headers,
    })
}

fn row_to_audit(row: sqlx::sqlite::SqliteRow) -> Result<AuditEvent> {
    let kind_s: String = row.try_get("kind")?;
    let kind = match kind_s.as_str() {
        "shell" => AuditEventKind::Shell,
        "filediff" | "file_diff" => AuditEventKind::FileDiff,
        "networkattempt" | "network_attempt" => AuditEventKind::NetworkAttempt,
        "delegation" => AuditEventKind::Delegation,
        "modechange" | "mode_change" => AuditEventKind::ModeChange,
        "checkpoint" => AuditEventKind::Checkpoint,
        "rollback" => AuditEventKind::Rollback,
        "approval" => AuditEventKind::Approval,
        "providercall" | "provider_call" => AuditEventKind::ProviderCall,
        "tool" => AuditEventKind::Tool,
        _ => AuditEventKind::Tool,
    };
    let sid: Option<String> = row.try_get("session_id")?;
    Ok(AuditEvent {
        id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
        session_id: sid.map(|s| SessionId(Uuid::parse_str(&s).unwrap_or_default())),
        kind,
        summary: row.try_get("summary")?,
        detail: serde_json::from_str(&row.try_get::<String, _>("detail_json")?)?,
        content_hash: row.try_get("content_hash")?,
        prev_hash: row.try_get("prev_hash")?,
        created_at: parse_rfc3339(&row.try_get::<String, _>("created_at")?)?,
    })
}

fn row_to_checkpoint(row: sqlx::sqlite::SqliteRow) -> Result<CheckpointMeta> {
    let audit: Option<String> = row.try_get("audit_event_id")?;
    Ok(CheckpointMeta {
        id: CheckpointId(Uuid::parse_str(&row.try_get::<String, _>("id")?)?),
        session_id: SessionId(Uuid::parse_str(&row.try_get::<String, _>("session_id")?)?),
        step_index: row.try_get::<i64, _>("step_index")? as u64,
        git_commit: row.try_get("git_commit")?,
        label: row.try_get("label")?,
        created_at: parse_rfc3339(&row.try_get::<String, _>("created_at")?)?,
        audit_event_id: audit.and_then(|s| Uuid::parse_str(&s).ok()),
    })
}

fn row_to_tool(row: sqlx::sqlite::SqliteRow) -> Result<ToolDefinition> {
    let enabled: i64 = row.try_get("enabled")?;
    Ok(ToolDefinition {
        name: row.try_get("name")?,
        description: row.try_get("description")?,
        input_schema: serde_json::from_str(&row.try_get::<String, _>("input_schema_json")?)?,
        categories: serde_json::from_str(&row.try_get::<String, _>("categories_json")?)?,
        enabled: enabled != 0,
    })
}

fn default_providers() -> Vec<ProviderConfig> {
    vec![
        ProviderConfig {
            id: LlmProviderId::new("ollama-local"),
            kind: LlmProviderKind::Ollama,
            display_name: "Ollama (local)".into(),
            base_url: "http://127.0.0.1:11434".into(),
            model: "llama3.2".into(),
            api_key_env: None,
            enabled: true,
            extra_headers: HashMap::new(),
        },
        ProviderConfig {
            id: LlmProviderId::new("llamacpp-local"),
            kind: LlmProviderKind::LlamaCpp,
            display_name: "llama.cpp (local)".into(),
            base_url: "http://127.0.0.1:8080".into(),
            model: "local".into(),
            api_key_env: None,
            enabled: true,
            extra_headers: HashMap::new(),
        },
        ProviderConfig {
            id: LlmProviderId::new("lmstudio-local"),
            kind: LlmProviderKind::LmStudio,
            display_name: "LM Studio (local)".into(),
            base_url: "http://127.0.0.1:1234/v1".into(),
            model: "local-model".into(),
            api_key_env: None,
            enabled: true,
            extra_headers: HashMap::new(),
        },
        ProviderConfig {
            id: LlmProviderId::new("openai-compatible-local"),
            kind: LlmProviderKind::OpenaiCompatible,
            display_name: "OpenAI-compatible (local)".into(),
            base_url: "http://127.0.0.1:8000/v1".into(),
            model: "local".into(),
            api_key_env: None,
            enabled: true,
            extra_headers: HashMap::new(),
        },
        ProviderConfig {
            id: LlmProviderId::new("anthropic"),
            kind: LlmProviderKind::Anthropic,
            display_name: "Anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            model: "claude-sonnet-4-20250514".into(),
            api_key_env: Some("ANTHROPIC_API_KEY".into()),
            enabled: false,
            extra_headers: HashMap::new(),
        },
        ProviderConfig {
            id: LlmProviderId::new("openai"),
            kind: LlmProviderKind::Openai,
            display_name: "OpenAI".into(),
            base_url: "https://api.openai.com/v1".into(),
            model: "gpt-4o".into(),
            api_key_env: Some("OPENAI_API_KEY".into()),
            enabled: false,
            extra_headers: HashMap::new(),
        },
        ProviderConfig {
            id: LlmProviderId::new("gemini"),
            kind: LlmProviderKind::Gemini,
            display_name: "Google Gemini".into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            model: "gemini-2.0-flash".into(),
            api_key_env: Some("GEMINI_API_KEY".into()),
            enabled: false,
            extra_headers: HashMap::new(),
        },
        ProviderConfig {
            id: LlmProviderId::new("openrouter"),
            kind: LlmProviderKind::OpenRouter,
            display_name: "OpenRouter".into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            model: "openai/gpt-4o-mini".into(),
            api_key_env: Some("OPENROUTER_API_KEY".into()),
            enabled: false,
            extra_headers: HashMap::new(),
        },
    ]
}