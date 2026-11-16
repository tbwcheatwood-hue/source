use anyhow::Result;
use source_core::types::{Mode, WorkspacePaths};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::context::ContextCompactor;
use crate::db::Db;
use crate::llm::LlmRouter;
use crate::planact::PlanActMachine;
use crate::policy::{PolicyEngine, PolicyHttpClient};
use crate::shadow::ShadowGit;

#[derive(Clone)]
pub struct AppState {
    pub paths: WorkspacePaths,
    pub db: Db,
    pub policy: PolicyEngine,
    pub http: PolicyHttpClient,
    pub llm: Arc<LlmRouter>,
    pub context: Arc<ContextCompactor>,
    pub shadow: Arc<ShadowGit>,
    pub planact: Arc<PlanActMachine>,
    pub started: Instant,
    pub bind: String,
    pub static_dir: Option<PathBuf>,
    pub global_mode: Mode,
}

impl AppState {
    pub async fn bootstrap(paths: WorkspacePaths, bind: String, static_dir: Option<PathBuf>) -> Result<Self> {
        paths.ensure_dirs()?;
        let db = Db::connect(&paths.db_path).await?;
        let policy = PolicyEngine::load(db.clone()).await?;
        let http = PolicyHttpClient::new(policy.clone())?;
        let llm = Arc::new(LlmRouter::new(db.clone(), policy.clone(), http.clone()));
        let context = Arc::new(ContextCompactor::default());
        let shadow = Arc::new(ShadowGit::new(db.clone(), &paths));
        let planact = Arc::new(PlanActMachine::new(
            db.clone(),
            policy.clone(),
            shadow.clone(),
        ));
        planact.register_builtin_tools().await?;

        // Seed RAG index metadata row for the data root (Phase 2 metadata only).
        db.upsert_rag_meta(
            "default",
            &paths.root,
            &paths.cache_dir.join("rag"),
            0,
            0,
            "idle",
        )
        .await?;

        Ok(Self {
            paths,
            db,
            policy,
            http,
            llm,
            context,
            shadow,
            planact,
            started: Instant::now(),
            bind,
            static_dir,
            global_mode: Mode::Plan,
        })
    }
}