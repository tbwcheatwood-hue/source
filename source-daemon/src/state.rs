use anyhow::Result;
use source_core::types::{Mode, WorkspacePaths};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::analyzer::TreeSitterAnalyzer;
use crate::browser::HeadlessBrowser;
use crate::context::ContextCompactor;
use crate::db::Db;
use crate::llm::LlmRouter;
use crate::mcp_host::McpHost;
use crate::planact::PlanActMachine;
use crate::policy::{PolicyEngine, PolicyHttpClient};
use crate::pty_runner::PtyRunner;
use crate::rag::OfflineRag;
use crate::shadow::ShadowGit;
use crate::templates::TemplateManager;

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
    pub analyzer: Arc<TreeSitterAnalyzer>,
    pub pty: Arc<PtyRunner>,
    pub rag: Arc<OfflineRag>,
    pub templates: Arc<TemplateManager>,
    pub browser: Arc<HeadlessBrowser>,
    pub mcp: Arc<McpHost>,
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
        let analyzer = Arc::new(TreeSitterAnalyzer::new(ContextCompactor::default()));
        let shadow = Arc::new(ShadowGit::new(db.clone(), &paths));
        let pty = Arc::new(PtyRunner::new());
        let rag = Arc::new(
            OfflineRag::open(paths.offline_docs_dir.clone(), paths.rag_dir.clone()).await?,
        );
        // Auto-index offline docs if any files exist.
        if paths.offline_docs_dir.exists()
            && std::fs::read_dir(&paths.offline_docs_dir)?
                .next()
                .is_some()
        {
            match rag.index_docs(None, false).await {
                Ok(status) => tracing::info!(
                    chunks = status.chunk_count,
                    files = status.file_count,
                    "offline RAG indexed"
                ),
                Err(err) => tracing::warn!(error = %err, "offline RAG index skipped"),
            }
        }

        let bundled = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
        let templates = Arc::new(TemplateManager::new(
            paths.templates_dir.clone(),
            paths.vendor_dir.clone(),
            bundled,
        ));
        templates.ensure_seeded()?;

        let browser = Arc::new(HeadlessBrowser::new(policy.clone()));
        let mcp = Arc::new(McpHost::start(
            paths.extensions_dir.clone(),
            policy.clone(),
        )?);

        let planact = Arc::new(PlanActMachine::new(
            db.clone(),
            policy.clone(),
            shadow.clone(),
            pty.clone(),
            rag.clone(),
            templates.clone(),
            browser.clone(),
            analyzer.clone(),
            mcp.clone(),
        ));
        planact.register_builtin_tools().await?;

        // Seed / refresh RAG index metadata.
        let rag_status = rag.status().await?;
        db.upsert_rag_meta(
            "offline-docs",
            &paths.offline_docs_dir,
            &paths.rag_dir,
            rag_status.chunk_count,
            rag_status.file_count,
            &rag_status.status,
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
            analyzer,
            pty,
            rag,
            templates,
            browser,
            mcp,
            started: Instant::now(),
            bind,
            static_dir,
            global_mode: Mode::Plan,
        })
    }
}
