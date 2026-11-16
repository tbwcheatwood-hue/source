//! Shadow Git (`.source-checkpoints`) + append-only SQLite audit + rollback API.

use anyhow::{bail, Context, Result};
use chrono::Utc;
use source_core::types::{
    AuditEvent, AuditEventKind, CheckpointId, CheckpointMeta, SessionId, WorkspacePaths,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

use crate::db::Db;

pub struct ShadowGit {
    db: Db,
    data_root: PathBuf,
}

impl ShadowGit {
    pub fn new(db: Db, paths: &WorkspacePaths) -> Self {
        Self {
            db,
            data_root: paths.checkpoints_dir.clone(),
        }
    }

    fn shadow_dir(workspace_root: &Path) -> PathBuf {
        workspace_root.join(".source-checkpoints")
    }

    fn git_env(shadow: &Path) -> Vec<(String, String)> {
        vec![
            ("GIT_DIR".into(), shadow.to_string_lossy().into_owned()),
            (
                "GIT_WORK_TREE".into(),
                shadow
                    .parent()
                    .unwrap_or(Path::new("."))
                    .to_string_lossy()
                    .into_owned(),
            ),
        ]
    }

    fn run_git(shadow: &Path, args: &[&str]) -> Result<String> {
        let mut cmd = Command::new("git");
        for (k, v) in Self::git_env(shadow) {
            cmd.env(k, v);
        }
        cmd.args(args);
        let output = cmd.output().context("spawn git")?;
        if !output.status.success() {
            bail!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub fn ensure_repo(&self, workspace_root: &Path) -> Result<PathBuf> {
        let shadow = Self::shadow_dir(workspace_root);
        if !shadow.exists() {
            std::fs::create_dir_all(&shadow)?;
            // Initialize a bare-like shadow repo whose work tree is the workspace.
            Command::new("git")
                .args(["init", "--bare"])
                .arg(&shadow)
                .output()
                .context("git init --bare")?;
            // Configure exclude: ignore the shadow dir itself and common noise.
            let info = shadow.join("info");
            std::fs::create_dir_all(&info)?;
            let exclude = info.join("exclude");
            std::fs::write(
                &exclude,
                ".source-checkpoints/\n.source/\n.git/\ntarget/\nnode_modules/\n",
            )?;
            Self::run_git(&shadow, &["config", "user.email", "source@localhost"])?;
            Self::run_git(&shadow, &["config", "user.name", "Source Shadow"])?;
            // Seed empty commit so rollback always has a base.
            Self::run_git(&shadow, &["add", "-A"])?;
            let _ = Self::run_git(
                &shadow,
                &[
                    "commit",
                    "--allow-empty",
                    "-m",
                    "source: initialize shadow checkpoints",
                ],
            );
        }
        let _ = &self.data_root; // retained for future per-session mirrors
        Ok(shadow)
    }

    pub async fn create_checkpoint(
        &self,
        session_id: SessionId,
        workspace_root: &Path,
        label: &str,
    ) -> Result<CheckpointMeta> {
        let shadow = self.ensure_repo(workspace_root)?;
        Self::run_git(&shadow, &["add", "-A"])?;
        let status = Self::run_git(&shadow, &["status", "--porcelain"])?;
        let msg = format!("source-checkpoint: {label}");
        if status.is_empty() {
            // Create an empty commit to mark the step even with no file changes.
            Self::run_git(
                &shadow,
                &["commit", "--allow-empty", "-m", &msg],
            )?;
        } else {
            Self::run_git(&shadow, &["commit", "-m", &msg])?;
        }
        let commit = Self::run_git(&shadow, &["rev-parse", "HEAD"])?;
        let step = self.db.next_checkpoint_step(&session_id).await?;

        let audit = self
            .db
            .append_audit(AuditEvent {
                id: Uuid::new_v4(),
                session_id: Some(session_id),
                kind: AuditEventKind::Checkpoint,
                summary: format!("checkpoint `{label}` @ {commit}"),
                detail: serde_json::json!({
                    "label": label,
                    "commit": commit,
                    "step_index": step,
                    "workspace_root": workspace_root,
                }),
                content_hash: String::new(),
                prev_hash: None,
                created_at: Utc::now(),
            })
            .await?;

        // Also record a file-diff summary for audit chain.
        let diff_stat = Self::run_git(&shadow, &["diff-tree", "--stat", "--no-commit-id", "-r", "HEAD"])
            .unwrap_or_default();
        if !diff_stat.is_empty() {
            let _ = self
                .db
                .append_audit(AuditEvent {
                    id: Uuid::new_v4(),
                    session_id: Some(session_id),
                    kind: AuditEventKind::FileDiff,
                    summary: format!("diff for checkpoint `{label}`"),
                    detail: serde_json::json!({ "stat": diff_stat }),
                    content_hash: String::new(),
                    prev_hash: None,
                    created_at: Utc::now(),
                })
                .await;
        }

        let meta = CheckpointMeta {
            id: CheckpointId::new(),
            session_id,
            step_index: step,
            git_commit: commit,
            label: label.to_string(),
            created_at: Utc::now(),
            audit_event_id: Some(audit.id),
        };
        self.db.insert_checkpoint(&meta).await?;
        Ok(meta)
    }

    pub async fn list(&self, session_id: &SessionId) -> Result<Vec<CheckpointMeta>> {
        self.db.list_checkpoints(session_id).await
    }

    /// Roll back workspace files to the checkpoint commit and mark SQLite session state.
    pub async fn rollback(
        &self,
        session_id: SessionId,
        checkpoint_id: CheckpointId,
        workspace_root: &Path,
    ) -> Result<CheckpointMeta> {
        let meta = self
            .db
            .get_checkpoint(&checkpoint_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("checkpoint not found"))?;
        if meta.session_id.0 != session_id.0 {
            bail!("checkpoint does not belong to session");
        }
        let shadow = self.ensure_repo(workspace_root)?;
        // Restore work tree to the recorded commit without moving HEAD permanently:
        // use read-tree + checkout-index style via `git restore --source`.
        // Fallback for older git: `git checkout <commit> -- .`
        if let Err(err) = Self::run_git(
            &shadow,
            &["restore", "--source", &meta.git_commit, "--worktree", "--", "."],
        ) {
            tracing::warn!(error = %err, "git restore failed; trying checkout");
            Self::run_git(&shadow, &["checkout", &meta.git_commit, "--", "."])?;
        }
        // Reset the shadow branch pointer to the checkpoint commit for consistency.
        Self::run_git(&shadow, &["reset", "--soft", &meta.git_commit])?;

        self.db
            .restore_session_marker(&session_id, meta.step_index)
            .await?;

        let _ = self
            .db
            .append_audit(AuditEvent {
                id: Uuid::new_v4(),
                session_id: Some(session_id),
                kind: AuditEventKind::Rollback,
                summary: format!(
                    "rollback to checkpoint {} (step {})",
                    meta.id, meta.step_index
                ),
                detail: serde_json::json!({
                    "checkpoint_id": meta.id,
                    "git_commit": meta.git_commit,
                    "step_index": meta.step_index,
                }),
                content_hash: String::new(),
                prev_hash: None,
                created_at: Utc::now(),
            })
            .await?;

        Ok(meta)
    }

    pub async fn audit_shell(
        &self,
        session_id: Option<SessionId>,
        command: &str,
        exit_code: i32,
        stdout_tail: &str,
    ) -> Result<AuditEvent> {
        self.db
            .append_audit(AuditEvent {
                id: Uuid::new_v4(),
                session_id,
                kind: AuditEventKind::Shell,
                summary: format!("shell exit={exit_code}: {command}"),
                detail: serde_json::json!({
                    "command": command,
                    "exit_code": exit_code,
                    "stdout_tail": stdout_tail,
                }),
                content_hash: String::new(),
                prev_hash: None,
                created_at: Utc::now(),
            })
            .await
    }

    pub async fn audit_delegation(
        &self,
        session_id: Option<SessionId>,
        target: &str,
        detail: serde_json::Value,
    ) -> Result<AuditEvent> {
        self.db
            .append_audit(AuditEvent {
                id: Uuid::new_v4(),
                session_id,
                kind: AuditEventKind::Delegation,
                summary: format!("delegation to {target}"),
                detail,
                content_hash: String::new(),
                prev_hash: None,
                created_at: Utc::now(),
            })
            .await
    }
}