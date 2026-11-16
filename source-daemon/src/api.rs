//! JSON-RPC over HTTP POST `/rpc` and WebSocket `/ws`, plus health and static hooks.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use source_core::error::CoreError;
use source_core::mesh::{DEFAULT_MESH_PORT, PeerSource, RemoteInferRequest};
use source_core::policy::{NetworkPolicy, NetworkTier};
use source_core::protocol::{
    AppVerifyParams, BeginPairingParams, BrowserInspectParams, BrowserNavigateParams,
    BuildRepoMapParams, ClusterDispatchParams, CompactConversationParams, CompactTerminalParams,
    CompletePairingParams, ConfirmPairingParams, CreateCheckpointParams, CreateSessionParams,
    EvaluateNetworkParams, InvokeToolParams, JsonRpcError, JsonRpcId, JsonRpcRequest,
    JsonRpcResponse, ListAuditParams, ListTasksParams, McpCallParams, MeshStartParams, Method,
    ListModelsParams, NetworkTierResult, Notification, PeerIdParams, ProbeProviderParams,
    ProviderIdParams, PtyIdParams, PtyOpenParams, PtyReadParams, PtyResizeParams, PtyWriteParams,
    RagIndexParams, RagQueryParams, RegisterPeerParams, RejectPairingParams,
    RequestApprovalParams, ResolveApprovalParams, RollbackParams, SearchSymbolsParams,
    SessionIdParams, SetModeParams, SetNetworkTierParams, SetSessionProviderParams, TaskIdParams,
    TemplateScaffoldParams, TrustPeerParams, PROTOCOL_VERSION,
};
use crate::analyzer::SymbolSearchQuery;
use crate::mesh::{PairAccept, SignedEnvelope};
use source_core::types::{
    DaemonStatus, LlmProviderId, LlmRequest, Mode, ProviderConfig, ProviderStatus, SessionId,
    SessionMeta, ToolDefinition,
};
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use uuid::Uuid;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let mut app = Router::new()
        .route("/health", get(health))
        .route("/rpc", post(rpc_http))
        .route("/ws", get(ws_upgrade))
        .route("/api/status", get(status_rest))
        .route("/api/network/tier", get(network_tier_rest))
        .route("/api/cluster/resources", get(cluster_resources_rest))
        .route("/mesh/pair/offer", get(mesh_pair_offer))
        .route("/mesh/pair/accept", post(mesh_pair_accept))
        .route("/mesh/rpc", post(mesh_rpc))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    if let Some(dir) = state.static_dir.clone() {
        if dir.exists() {
            app = app.fallback_service(ServeDir::new(dir));
        }
    }

    app
}

async fn health() -> impl IntoResponse {
    Json(json!({ "ok": true, "service": "source-daemon", "protocol": PROTOCOL_VERSION }))
}

async fn status_rest(State(state): State<AppState>) -> impl IntoResponse {
    match build_status(&state).await {
        Ok(s) => (StatusCode::OK, Json(json!(s))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn network_tier_rest(State(state): State<AppState>) -> impl IntoResponse {
    let p = state.policy.current();
    Json(NetworkTierResult {
        tier: p.tier,
        allowlist: p.allowlist,
        denylist: p.denylist,
        require_approval_for_public: p.require_approval_for_public,
    })
}

async fn cluster_resources_rest(State(state): State<AppState>) -> impl IntoResponse {
    match state.orchestrator.cluster_snapshot().await {
        Ok(snap) => (StatusCode::OK, Json(json!(snap))).into_response(),
        Err(e) => (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn mesh_pair_offer(State(state): State<AppState>) -> impl IntoResponse {
    // Return the most recently created pending pairing offer.
    let status = state.mesh.status().await;
    if !status.mesh_allowed {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "mesh disabled by network tier" })),
        )
            .into_response();
    }
    match newest_pair_offer(&state).await {
        Ok(offer) => (StatusCode::OK, Json(json!(offer))).into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn newest_pair_offer(state: &AppState) -> Result<crate::mesh::PairOffer, CoreError> {
    // begin_pairing stores pending; we keep last pairing_id in kv for HTTP discovery.
    if let Ok(Some(v)) = state.db.get_kv("mesh:last_pairing_id").await {
        if let Some(id) = v.as_str().and_then(|s| uuid::Uuid::parse_str(s).ok()) {
            return state.mesh.get_pair_offer(id);
        }
    }
    Err(CoreError::PairingFailed(
        "no active pairing session; call source/mesh/beginPairing first".into(),
    ))
}

async fn mesh_pair_accept(
    State(state): State<AppState>,
    Json(accept): Json<PairAccept>,
) -> impl IntoResponse {
    match state.mesh.accept_pair_offer(accept).await {
        Ok(complete) => (StatusCode::OK, Json(json!(complete))).into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn mesh_rpc(
    State(state): State<AppState>,
    Json(env): Json<SignedEnvelope>,
) -> impl IntoResponse {
    match state.orchestrator.handle_inbound_rpc(env).await {
        Ok(result) => (StatusCode::OK, Json(json!({ "result": result }))).into_response(),
        Err(e) => (
            StatusCode::FORBIDDEN,
            Json(json!({ "error": e.to_string() })),
        )
            .into_response(),
    }
}

async fn rpc_http(
    State(state): State<AppState>,
    Json(req): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    let resp = dispatch(&state, req).await;
    Json(resp)
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();
    // Greeting notification
    let hello = Notification::new(
        "source/hello",
        json!({
            "protocol": PROTOCOL_VERSION,
            "tier": state.policy.tier().as_str(),
        }),
    );
    if let Ok(text) = serde_json::to_string(&hello) {
        let _ = sink.send(Message::Text(text.into())).await;
    }

    while let Some(Ok(msg)) = stream.next().await {
        match msg {
            Message::Text(text) => {
                match serde_json::from_str::<JsonRpcRequest>(&text) {
                    Ok(req) => {
                        let resp = dispatch(&state, req).await;
                        if let Ok(out) = serde_json::to_string(&resp) {
                            if sink.send(Message::Text(out.into())).await.is_err() {
                                break;
                            }
                        }
                    }
                    Err(err) => {
                        let resp = JsonRpcResponse::failure(
                            JsonRpcId::Null,
                            JsonRpcError::invalid_params(format!("invalid request: {err}")),
                        );
                        if let Ok(out) = serde_json::to_string(&resp) {
                            let _ = sink.send(Message::Text(out.into())).await;
                        }
                    }
                }
            }
            Message::Ping(p) => {
                let _ = sink.send(Message::Pong(p)).await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

async fn build_status(state: &AppState) -> anyhow::Result<DaemonStatus> {
    let providers = state.db.list_providers().await?;
    let provider_status: Vec<ProviderStatus> = providers
        .into_iter()
        .map(|p| ProviderStatus {
            id: p.id,
            kind: p.kind,
            display_name: p.display_name,
            enabled: p.enabled,
            reachable: None,
            base_url: p.base_url,
        })
        .collect();
    Ok(DaemonStatus {
        version: env!("CARGO_PKG_VERSION").into(),
        protocol_version: PROTOCOL_VERSION.into(),
        mode: state.global_mode,
        network_tier: state.policy.tier(),
        data_root: state.paths.root.clone(),
        active_session: None,
        uptime_secs: state.started.elapsed().as_secs(),
        providers: provider_status,
    })
}

fn core_err(err: CoreError) -> JsonRpcError {
    let code = match &err {
        CoreError::NetworkBlocked(_) => -32001,
        CoreError::ApprovalRequired(_) => -32002,
        CoreError::ApprovalDenied(_) => -32003,
        CoreError::InvalidModeTransition { .. } => -32004,
        CoreError::ProviderUnavailable(_) => -32005,
        CoreError::SessionNotFound(_) => -32006,
        CoreError::CheckpointNotFound(_) => -32007,
        CoreError::MeshDisabled(_) => -32008,
        CoreError::PeerNotFound(_) => -32009,
        CoreError::PairingFailed(_) => -32010,
        CoreError::InvalidArgument(_) => -32602,
        _ => -32603,
    };
    JsonRpcError::application(code, err.to_string(), None)
}

async fn dispatch(state: &AppState, req: JsonRpcRequest) -> JsonRpcResponse {
    let id = req.id.clone();
    if req.jsonrpc != "2.0" {
        return JsonRpcResponse::failure(
            id,
            JsonRpcError::invalid_params("jsonrpc must be \"2.0\""),
        );
    }
    let Some(method) = Method::parse(&req.method) else {
        return JsonRpcResponse::failure(id, JsonRpcError::method_not_found(&req.method));
    };

    let result = handle_method(state, method, req.params).await;
    match result {
        Ok(value) => JsonRpcResponse::success(id, value),
        Err(err) => JsonRpcResponse::failure(id, err),
    }
}

async fn handle_method(
    state: &AppState,
    method: Method,
    params: Value,
) -> Result<Value, JsonRpcError> {
    match method {
        Method::Status => {
            let s = build_status(state)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(s))
        }
        Method::GetNetworkTier => {
            let p = state.policy.current();
            Ok(json!(NetworkTierResult {
                tier: p.tier,
                allowlist: p.allowlist,
                denylist: p.denylist,
                require_approval_for_public: p.require_approval_for_public,
            }))
        }
        Method::SetNetworkTier => {
            let p: SetNetworkTierParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let mut policy = state.policy.current();
            policy.tier = p.tier;
            if let Some(a) = p.allowlist {
                policy.allowlist = a;
            }
            if let Some(d) = p.denylist {
                policy.denylist = d;
            }
            if let Some(r) = p.require_approval_for_public {
                policy.require_approval_for_public = r;
            }
            let saved = state
                .policy
                .set_policy(policy)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(NetworkTierResult {
                tier: saved.tier,
                allowlist: saved.allowlist,
                denylist: saved.denylist,
                require_approval_for_public: saved.require_approval_for_public,
            }))
        }
        Method::EvaluateNetwork => {
            let p: EvaluateNetworkParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let ip = p.ip.as_deref().and_then(|s| s.parse::<IpAddr>().ok());
            let decision = state.policy.evaluate(&p.host, ip);
            Ok(json!(decision))
        }
        Method::CreateSession => {
            let p: CreateSessionParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let now = Utc::now();
            let selected = state
                .llm
                .get_selected_provider()
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            let meta = SessionMeta {
                id: SessionId::new(),
                title: p.title,
                mode: p.mode.unwrap_or(Mode::Plan),
                workspace_root: PathBuf::from(p.workspace_root),
                created_at: now,
                updated_at: now,
                network_tier: state.policy.tier(),
                provider_id: selected.provider_id.map(LlmProviderId::new),
            };
            state
                .db
                .create_session(&meta)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(meta))
        }
        Method::ListSessions => {
            let list = state
                .db
                .list_sessions()
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(list))
        }
        Method::GetSession => {
            let p: SessionIdParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let s = state
                .db
                .get_session(&p.session_id)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?
                .ok_or_else(|| {
                    core_err(CoreError::SessionNotFound(p.session_id.to_string()))
                })?;
            Ok(json!(s))
        }
        Method::SetMode => {
            let p: SetModeParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let mode = state
                .planact
                .set_mode(p.session_id, p.mode)
                .await
                .map_err(core_err)?;
            Ok(json!({ "session_id": p.session_id, "mode": mode }))
        }
        Method::ListProviders => {
            let list = state
                .llm
                .list()
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(list))
        }
        Method::UpsertProvider => {
            let p: ProviderConfig = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let saved = state
                .llm
                .upsert(p)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(saved))
        }
        Method::EnableProvider => {
            let p: ProviderIdParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            state
                .llm
                .set_enabled(&p.provider_id, true)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!({ "provider_id": p.provider_id, "enabled": true }))
        }
        Method::DisableProvider => {
            let p: ProviderIdParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            state
                .llm
                .set_enabled(&p.provider_id, false)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!({ "provider_id": p.provider_id, "enabled": false }))
        }
        Method::ProbeProvider => {
            let p: ProbeProviderParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let result = state
                .llm
                .probe(p, None)
                .await
                .map_err(core_err)?;
            Ok(json!(result))
        }
        Method::ListModels => {
            let p: ListModelsParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let result = state
                .llm
                .list_models(&p.provider_id)
                .await
                .map_err(core_err)?;
            Ok(json!(result))
        }
        Method::GetSelectedProvider => {
            let result = state
                .llm
                .get_selected_provider()
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(result))
        }
        Method::SetSelectedProvider => {
            let p: ProviderIdParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let result = state
                .llm
                .set_selected_provider(&p.provider_id)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(result))
        }
        Method::SetSessionProvider => {
            let p: SetSessionProviderParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let provider = state
                .db
                .get_provider(&p.provider_id)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?
                .ok_or_else(|| {
                    core_err(CoreError::ProviderUnavailable(p.provider_id.clone()))
                })?;
            state
                .db
                .update_session_provider(&p.session_id, Some(&provider.id.0))
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            let session = state
                .db
                .get_session(&p.session_id)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?
                .ok_or_else(|| {
                    core_err(CoreError::SessionNotFound(p.session_id.to_string()))
                })?;
            Ok(json!(session))
        }
        Method::Complete => {
            let mut req: LlmRequest = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let resolved = state
                .llm
                .resolve_provider_id(&req.provider_id.0, None)
                .await
                .map_err(core_err)?;
            req.provider_id = LlmProviderId::new(resolved);
            let resp = state
                .llm
                .complete(req, None)
                .await
                .map_err(core_err)?;
            Ok(json!(resp))
        }
        Method::BuildRepoMap => {
            let p: BuildRepoMapParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let map = state
                .context
                .build_repo_map(PathBuf::from(&p.root).as_path(), p.max_symbols)
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            // Persist RAG metadata for this workspace.
            let _ = state
                .db
                .upsert_rag_meta(
                    &format!("ws-{}", blake_short(&p.root)),
                    PathBuf::from(&p.root).as_path(),
                    state.paths.cache_dir.join("rag").as_path(),
                    map.symbols.len() as i64,
                    map.file_count as i64,
                    "ready",
                )
                .await;
            Ok(json!(map))
        }
        Method::CompactTerminal => {
            let p: CompactTerminalParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let slice = state
                .context
                .compact_terminal(&p.log, p.max_chars, p.exit_code);
            Ok(json!(slice))
        }
        Method::CompactConversation => {
            let p: CompactConversationParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let result = state
                .context
                .compact_conversation(p.turns, p.used_tokens, p.budget);
            Ok(json!(result))
        }
        Method::CreateCheckpoint => {
            let p: CreateCheckpointParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let meta = state
                .shadow
                .create_checkpoint(p.session_id, PathBuf::from(&p.workspace_root).as_path(), &p.label)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(meta))
        }
        Method::ListCheckpoints => {
            let p: SessionIdParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let list = state
                .shadow
                .list(&p.session_id)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(list))
        }
        Method::Rollback => {
            let p: RollbackParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let meta = state
                .shadow
                .rollback(
                    p.session_id,
                    p.checkpoint_id,
                    PathBuf::from(&p.workspace_root).as_path(),
                )
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(meta))
        }
        Method::RequestApproval => {
            let p: RequestApprovalParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let id = Uuid::new_v4();
            state
                .db
                .insert_approval(id, p.session_id.as_ref(), p.category, &p.summary, &p.detail)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!({ "approval_id": id, "status": "pending" }))
        }
        Method::ResolveApproval => {
            let p: ResolveApprovalParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            state
                .planact
                .resolve_approval(p.approval_id, p.decision)
                .await
                .map_err(core_err)?;
            Ok(json!({ "approval_id": p.approval_id, "decision": p.decision }))
        }
        Method::ListPendingApprovals => {
            let list = state
                .db
                .list_pending_approvals()
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(list))
        }
        Method::RegisterTool => {
            let tool: ToolDefinition = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            state
                .db
                .register_tool(&tool)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(tool))
        }
        Method::ListTools => {
            let list = state
                .db
                .list_tools()
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(list))
        }
        Method::InvokeTool => {
            let p: InvokeToolParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let result = state
                .planact
                .invoke_tool(p.session_id, p.call)
                .await
                .map_err(core_err)?;
            Ok(json!(result))
        }
        Method::ListAuditEvents => {
            let p: ListAuditParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let list = state
                .db
                .list_audit(p.session_id.as_ref(), p.limit.unwrap_or(100))
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(list))
        }
        Method::SearchSymbols => {
            let p: SearchSymbolsParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let hits = state
                .analyzer
                .search(
                    PathBuf::from(&p.root).as_path(),
                    SymbolSearchQuery {
                        query: p.query,
                        kind: p.kind,
                        language: p.language,
                        path_glob: p.path_glob,
                        max_results: p.max_results.unwrap_or(50),
                    },
                )
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(hits))
        }
        Method::PtyOpen => {
            let p: PtyOpenParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let info = state
                .pty
                .open(
                    p.cwd.map(PathBuf::from),
                    p.shell,
                    p.cols,
                    p.rows,
                    p.env,
                )
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(info))
        }
        Method::PtyWrite => {
            let p: PtyWriteParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            state
                .pty
                .write(&p.pty_id, &p.data)
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!({ "ok": true }))
        }
        Method::PtyRead => {
            let p: PtyReadParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let r = state
                .pty
                .read(&p.pty_id, p.max_bytes, p.wait_ms)
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(r))
        }
        Method::PtyResize => {
            let p: PtyResizeParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let info = state
                .pty
                .resize(&p.pty_id, p.cols, p.rows)
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(info))
        }
        Method::PtyClose => {
            let p: PtyIdParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let info = state
                .pty
                .close(&p.pty_id)
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(info))
        }
        Method::PtyList => Ok(json!(state.pty.list())),
        Method::RagIndex => {
            let p: RagIndexParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let status = state
                .rag
                .index_docs(p.docs_root.map(PathBuf::from), p.force.unwrap_or(false))
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            let _ = state
                .db
                .upsert_rag_meta(
                    "offline-docs",
                    &status.docs_root,
                    &status.index_path,
                    status.chunk_count,
                    status.file_count,
                    &status.status,
                )
                .await;
            Ok(json!(status))
        }
        Method::RagQuery => {
            let p: RagQueryParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let hits = state
                .rag
                .query(
                    &p.query,
                    p.top_k.unwrap_or(8),
                    p.bm25_weight.unwrap_or(0.5),
                    p.vector_weight.unwrap_or(0.5),
                )
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(hits))
        }
        Method::RagStatus => {
            let status = state
                .rag
                .status()
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(status))
        }
        Method::TemplateList => {
            let list = state
                .templates
                .list()
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(list))
        }
        Method::TemplateScaffold => {
            let p: TemplateScaffoldParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let dest = state
                .templates
                .scaffold(&p.template_id, PathBuf::from(&p.dest).as_path(), p.name.as_deref())
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!({ "dest": dest }))
        }
        Method::CacheVendorStatus => {
            let status = state
                .templates
                .vendor_status()
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(json!(status))
        }
        Method::McpList => Ok(json!(state.mcp.list_filtered())),
        Method::McpReload => {
            let list = state
                .mcp
                .reload()
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            // Re-register MCP tools into tool registry
            for t in state.mcp.as_tool_definitions() {
                let _ = state.db.register_tool(&t).await;
            }
            Ok(json!(list))
        }
        Method::McpCall => {
            let p: McpCallParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let result = state
                .mcp
                .call_tool(&p.server_id, &p.tool_name, p.arguments)
                .await
                .map_err(|e| JsonRpcError::internal(e.to_string()))?;
            Ok(result)
        }
        Method::BrowserNavigate => {
            let p: BrowserNavigateParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let r = state
                .browser
                .navigate(&p.url, p.session_key.as_deref(), p.wait_ms, None)
                .await
                .map_err(core_err)?;
            Ok(json!(r))
        }
        Method::BrowserInspect => {
            let p: BrowserInspectParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let r = state
                .browser
                .inspect(p.session_key.as_deref(), p.selector.as_deref(), p.script.as_deref())
                .await
                .map_err(core_err)?;
            Ok(json!(r))
        }
        Method::AppVerify => {
            let p: AppVerifyParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let r = state
                .browser
                .verify_app(
                    &p.launch_command,
                    p.cwd.as_deref(),
                    &p.url,
                    p.ready_selector.as_deref(),
                    p.assert_text.as_deref(),
                    p.timeout_ms,
                    None,
                )
                .await
                .map_err(core_err)?;
            Ok(json!(r))
        }
        Method::MeshStart => {
            let p: MeshStartParams = serde_json::from_value(params.clone())
                .unwrap_or(MeshStartParams {
                    advertise: Some(true),
                    browse: Some(true),
                    role: None,
                    label: None,
                });
            let status = state
                .mesh
                .start(
                    p.advertise.unwrap_or(true),
                    p.browse.unwrap_or(true),
                    p.role,
                    p.label,
                )
                .await
                .map_err(core_err)?;
            Ok(json!(status))
        }
        Method::MeshStop => {
            let status = state.mesh.stop().await.map_err(core_err)?;
            Ok(json!(status))
        }
        Method::MeshStatus => Ok(json!(state.mesh.status().await)),
        Method::MeshDiscover => Ok(json!(state.mesh.list_discovered())),
        Method::MeshRegisterPeer => {
            let p: RegisterPeerParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let source = p.source.unwrap_or_else(|| {
                if p.host.contains(".ts.net") || p.host.contains("tailscale") {
                    PeerSource::Tailscale
                } else {
                    PeerSource::Manual
                }
            });
            let info = state
                .mesh
                .register_peer(
                    &p.host,
                    p.port.unwrap_or(DEFAULT_MESH_PORT),
                    p.label,
                    source,
                    p.peer_id,
                )
                .await
                .map_err(core_err)?;
            Ok(json!(info))
        }
        Method::MeshUnregisterPeer => {
            let p: PeerIdParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            state
                .mesh
                .unregister_peer(&p.peer_id)
                .await
                .map_err(core_err)?;
            Ok(json!({ "ok": true, "peer_id": p.peer_id }))
        }
        Method::MeshListPeers => {
            let list = state.mesh.list_peers().await.map_err(core_err)?;
            Ok(json!(list))
        }
        Method::MeshBeginPairing => {
            let p: BeginPairingParams = serde_json::from_value(params.clone())
                .unwrap_or(BeginPairingParams {
                    label: None,
                    ttl_secs: None,
                });
            let session = state
                .mesh
                .begin_pairing(p.label, p.ttl_secs)
                .await
                .map_err(core_err)?;
            let _ = state
                .db
                .set_kv(
                    "mesh:last_pairing_id",
                    &json!(session.pairing_id.to_string()),
                )
                .await;
            Ok(json!(session))
        }
        Method::MeshConfirmPairing => {
            let p: ConfirmPairingParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let info = state
                .mesh
                .confirm_pairing(
                    p.pairing_id,
                    &p.peer_id,
                    &p.peer_public_key,
                    &p.pin,
                    p.label,
                    p.address,
                    p.port,
                )
                .await
                .map_err(core_err)?;
            Ok(json!(info))
        }
        Method::MeshCompletePairing => {
            let p: CompletePairingParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let info = state
                .mesh
                .complete_pairing_remote(
                    &p.host,
                    p.port.unwrap_or(DEFAULT_MESH_PORT),
                    &p.pin,
                    p.label,
                    state.http.raw(),
                )
                .await
                .map_err(core_err)?;
            Ok(json!(info))
        }
        Method::MeshRejectPairing => {
            let p: RejectPairingParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            state
                .mesh
                .reject_pairing(p.pairing_id)
                .await
                .map_err(core_err)?;
            Ok(json!({ "ok": true }))
        }
        Method::MeshTrustPeer => {
            let p: TrustPeerParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let info = state
                .mesh
                .set_trusted(&p.peer_id, p.trusted)
                .await
                .map_err(core_err)?;
            Ok(json!(info))
        }
        Method::MeshRevokePeer => {
            let p: PeerIdParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let info = state
                .mesh
                .set_trusted(&p.peer_id, false)
                .await
                .map_err(core_err)?;
            Ok(json!(info))
        }
        Method::ClusterResources => {
            let snap = state
                .orchestrator
                .cluster_snapshot()
                .await
                .map_err(core_err)?;
            Ok(json!(snap))
        }
        Method::ClusterDispatch => {
            let p: ClusterDispatchParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let result = state
                .orchestrator
                .dispatch(p.tasks, p.parallel.unwrap_or(true), p.session_id)
                .await
                .map_err(core_err)?;
            Ok(json!(result))
        }
        Method::ClusterTaskStatus => {
            let p: TaskIdParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let info = state
                .orchestrator
                .task_status(p.task_id)
                .await
                .map_err(core_err)?;
            Ok(json!(info))
        }
        Method::ClusterCancelTask => {
            let p: TaskIdParams = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let info = state
                .orchestrator
                .cancel_task(p.task_id)
                .await
                .map_err(core_err)?;
            Ok(json!(info))
        }
        Method::ClusterListTasks => {
            let p: ListTasksParams = serde_json::from_value(params.clone()).unwrap_or(
                ListTasksParams {
                    include_completed: Some(true),
                    limit: Some(100),
                },
            );
            let list = state
                .orchestrator
                .list_tasks(
                    p.include_completed.unwrap_or(true),
                    p.limit.unwrap_or(100),
                )
                .await
                .map_err(core_err)?;
            Ok(json!(list))
        }
        Method::LlmCompleteRemote => {
            let p: RemoteInferRequest = serde_json::from_value(params)
                .map_err(|e| JsonRpcError::invalid_params(e.to_string()))?;
            let resp = state
                .orchestrator
                .complete_remote(p, None)
                .await
                .map_err(core_err)?;
            Ok(json!(resp))
        }
    }
}

fn blake_short(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(s.as_bytes());
    hex::encode(&h[..8])
}

#[allow(dead_code)]
fn parse_tier(s: &str) -> Result<NetworkTier, JsonRpcError> {
    NetworkTier::from_str(s).map_err(|e| JsonRpcError::invalid_params(e.to_string()))
}

#[allow(dead_code)]
fn empty_policy() -> NetworkPolicy {
    NetworkPolicy::default()
}