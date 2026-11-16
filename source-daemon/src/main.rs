use anyhow::Result;
use clap::Parser;
use source_core::types::WorkspacePaths;
use source_daemon::api::router;
use source_daemon::state::AppState;
use std::net::SocketAddr;
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "source-daemon", about = "Source local daemon (Phase 1–4)")]
struct Args {
    /// Bind address for HTTP/WebSocket API
    #[arg(long, env = "SOURCE_BIND", default_value = "127.0.0.1:7420")]
    bind: String,

    /// Override Source data root (default: ~/.source)
    #[arg(long, env = "SOURCE_HOME")]
    source_home: Option<PathBuf>,

    /// Optional directory to serve as static web assets
    #[arg(long, env = "SOURCE_STATIC")]
    static_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();
    let home = args
        .source_home
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")));
    let mut paths = WorkspacePaths::from_home(home);
    if let Some(ref static_dir) = args.static_dir {
        paths.static_dir = Some(static_dir.clone());
    }

    tracing::info!(data_root = %paths.root.display(), bind = %args.bind, "starting source-daemon");

    let state = AppState::bootstrap(paths, args.bind.clone(), args.static_dir.clone()).await?;
    let app = router(state);
    let addr: SocketAddr = args.bind.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "source-daemon listening");
    axum::serve(listener, app).await?;
    Ok(())
}