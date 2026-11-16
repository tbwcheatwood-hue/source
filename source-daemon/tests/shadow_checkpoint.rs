use source_core::types::{SessionId, SessionMeta, WorkspacePaths};
use source_core::policy::NetworkTier;
use source_core::types::Mode;
use source_daemon::db::Db;
use source_daemon::shadow::ShadowGit;
use chrono::Utc;
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn checkpoint_and_rollback_restores_file() {
    let dir = tempdir().unwrap();
    let workspace = dir.path().join("ws");
    fs::create_dir_all(&workspace).unwrap();
    let file = workspace.join("note.txt");
    fs::write(&file, "v1").unwrap();

    let paths = WorkspacePaths {
        root: dir.path().join(".source"),
        db_path: dir.path().join(".source/source.db"),
        checkpoints_dir: dir.path().join(".source/checkpoints"),
        logs_dir: dir.path().join(".source/logs"),
        cache_dir: dir.path().join(".source/cache"),
        static_dir: None,
    };
    paths.ensure_dirs().unwrap();
    let db = Db::connect(&paths.db_path).await.unwrap();
    let session_id = SessionId::new();
    let now = Utc::now();
    db.create_session(&SessionMeta {
        id: session_id,
        title: "t".into(),
        mode: Mode::Act,
        workspace_root: workspace.clone(),
        created_at: now,
        updated_at: now,
        network_tier: NetworkTier::LocalOnly,
        provider_id: None,
    })
    .await
    .unwrap();

    let shadow = ShadowGit::new(db.clone(), &paths);
    let cp = shadow
        .create_checkpoint(session_id, &workspace, "before-edit")
        .await
        .unwrap();
    assert!(!cp.git_commit.is_empty());

    fs::write(&file, "v2-changed").unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "v2-changed");

    shadow
        .rollback(session_id, cp.id, &workspace)
        .await
        .unwrap();
    assert_eq!(fs::read_to_string(&file).unwrap(), "v1");

    let audits = db.list_audit(Some(&session_id), 50).await.unwrap();
    assert!(audits.iter().any(|a| matches!(
        a.kind,
        source_core::types::AuditEventKind::Checkpoint | source_core::types::AuditEventKind::Rollback
    )));
}