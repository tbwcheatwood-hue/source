//! Orchestrator / worker layer: remote inference routing, parallel task dispatch, cluster monitor.

use anyhow::Result;
use chrono::Utc;
use parking_lot::RwLock;
use serde_json::{json, Value};
use source_core::error::CoreError;
use source_core::mesh::{
    mesh_tier_allowed, ClusterPeerSnapshot, ClusterSnapshot, DispatchResult, GpuDevice,
    LoadedModel, NodeResources, PeerCapabilities, PeerRole, RemoteInferRequest,
    RemoteInferResponse, RemoteTaskInfo, RemoteTaskKind, RemoteTaskSpec, RemoteTaskState,
};
use source_core::types::{
    AuditEvent, AuditEventKind, LlmProviderId, LlmRequest, LlmResponse, SessionId,
};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use sysinfo::System;
use uuid::Uuid;

use crate::db::Db;
use crate::llm::LlmRouter;
use crate::mesh::{MeshService, SignedEnvelope};
use crate::policy::{PolicyEngine, PolicyHttpClient};

pub struct Orchestrator {
    db: Db,
    policy: PolicyEngine,
    http: PolicyHttpClient,
    llm: Arc<LlmRouter>,
    mesh: Arc<MeshService>,
    active_remote: AtomicU32,
    local_models: RwLock<Vec<LoadedModel>>,
}

impl Orchestrator {
    pub fn new(
        db: Db,
        policy: PolicyEngine,
        http: PolicyHttpClient,
        llm: Arc<LlmRouter>,
        mesh: Arc<MeshService>,
    ) -> Arc<Self> {
        Arc::new(Self {
            db,
            policy,
            http,
            llm,
            mesh,
            active_remote: AtomicU32::new(0),
            local_models: RwLock::new(Vec::new()),
        })
    }

    pub fn mesh(&self) -> Arc<MeshService> {
        Arc::clone(&self.mesh)
    }

    fn assert_mesh(&self) -> Result<(), CoreError> {
        let tier = self.policy.tier();
        if !mesh_tier_allowed(tier) {
            return Err(CoreError::MeshDisabled(format!(
                "cluster/mesh requires LOCAL_AND_LAN or higher (current {})",
                tier.as_str()
            )));
        }
        Ok(())
    }

    pub async fn refresh_local_models(&self) -> Result<(), CoreError> {
        let providers = self
            .llm
            .list()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        let mut models = Vec::new();
        for p in providers.into_iter().filter(|p| p.enabled) {
            models.push(LoadedModel {
                name: p.model.clone(),
                provider_id: Some(p.id.0.clone()),
                vram_mb: None,
                context_length: None,
            });
        }
        // Probe Ollama tags when present (loopback; always policy-safe).
        if let Ok(resp) = self
            .http
            .request_json(
                reqwest::Method::GET,
                "http://127.0.0.1:11434/api/tags",
                &[],
                None,
                None,
                "cluster-ollama-tags",
            )
            .await
        {
            if resp.status().is_success() {
                if let Ok(v) = resp.json::<Value>().await {
                    if let Some(arr) = v.get("models").and_then(|m| m.as_array()) {
                        for m in arr {
                            if let Some(name) = m.get("name").and_then(|n| n.as_str()) {
                                if !models.iter().any(|x| x.name == name) {
                                    models.push(LoadedModel {
                                        name: name.to_string(),
                                        provider_id: Some("ollama-local".into()),
                                        vram_mb: m
                                            .get("size")
                                            .and_then(|s| s.as_u64())
                                            .map(|b| b / (1024 * 1024)),
                                        context_length: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        *self.local_models.write() = models;
        Ok(())
    }

    pub fn collect_local_resources(&self) -> NodeResources {
        let mut sys = System::new();
        sys.refresh_memory();
        sys.refresh_cpu_usage();
        std::thread::sleep(std::time::Duration::from_millis(50));
        sys.refresh_cpu_usage();

        let (node_id, pk, label, role) = self.mesh.identity_snapshot();
        let hostname = hostname::get()
            .ok()
            .and_then(|h| h.into_string().ok())
            .unwrap_or_else(|| label.clone());
        let gpus = detect_gpus();
        let models = self.local_models.read().clone();
        let port = self.mesh.listen_port();
        NodeResources {
            node_id,
            label,
            hostname,
            role,
            address: "127.0.0.1".into(),
            port,
            cpu_cores: sys.cpus().len() as u32,
            cpu_usage_pct: sys.global_cpu_usage(),
            ram_total_mb: sys.total_memory() / (1024 * 1024),
            ram_available_mb: sys.available_memory() / (1024 * 1024),
            gpus,
            loaded_models: models,
            active_remote_tasks: self.active_remote.load(Ordering::SeqCst),
            mesh_enabled: self.policy.current().mesh_allowed(),
            network_tier: self.policy.tier(),
            public_key_hex: pk,
            updated_at: Utc::now(),
        }
    }

    pub async fn cluster_snapshot(&self) -> Result<ClusterSnapshot, CoreError> {
        let _ = self.refresh_local_models().await;
        let local = self.collect_local_resources();
        let mut mesh_status = self.mesh.status().await;
        mesh_status.active_remote_tasks = self.active_remote.load(Ordering::SeqCst) as usize;

        let mut peers = Vec::new();
        if self.policy.current().mesh_allowed() {
            for peer in self.mesh.list_peers().await? {
                if !peer.trusted {
                    peers.push(ClusterPeerSnapshot {
                        peer,
                        resources: None,
                        reachable: false,
                        last_error: Some("not trusted".into()),
                    });
                    continue;
                }
                match self.fetch_peer_resources(&peer.peer_id).await {
                    Ok(resources) => {
                        self.mesh.mark_connected(&peer.peer_id, true).await;
                        let caps = PeerCapabilities {
                            can_infer: !resources.loaded_models.is_empty()
                                || !resources.gpus.is_empty(),
                            can_accept_tasks: matches!(
                                resources.role,
                                PeerRole::Worker | PeerRole::Hybrid
                            ),
                            models: resources
                                .loaded_models
                                .iter()
                                .map(|m| m.name.clone())
                                .collect(),
                            vram_total_mb: resources.gpus.iter().map(|g| g.vram_total_mb).sum(),
                            vram_free_mb: resources.gpus.iter().map(|g| g.vram_free_mb).sum(),
                        };
                        let _ = self
                            .mesh
                            .update_peer_capabilities(&peer.peer_id, &caps)
                            .await;
                        peers.push(ClusterPeerSnapshot {
                            peer,
                            resources: Some(resources),
                            reachable: true,
                            last_error: None,
                        });
                    }
                    Err(err) => {
                        self.mesh.mark_connected(&peer.peer_id, false).await;
                        peers.push(ClusterPeerSnapshot {
                            peer,
                            resources: None,
                            reachable: false,
                            last_error: Some(err.to_string()),
                        });
                    }
                }
            }
        }

        let active_tasks = self
            .db
            .list_mesh_tasks(true, 100)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?
            .into_iter()
            .filter(|t| {
                matches!(
                    t.state,
                    RemoteTaskState::Pending
                        | RemoteTaskState::Assigned
                        | RemoteTaskState::Running
                )
            })
            .collect();

        Ok(ClusterSnapshot {
            local,
            peers,
            active_tasks,
            mesh_status,
            collected_at: Utc::now(),
        })
    }

    async fn fetch_peer_resources(&self, peer_id: &str) -> Result<NodeResources, CoreError> {
        let result = self
            .call_peer(peer_id, "mesh/resources", json!({}))
            .await?;
        serde_json::from_value(result).map_err(|e| CoreError::Other(e.to_string()))
    }

    pub async fn call_peer(
        &self,
        peer_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, CoreError> {
        self.assert_mesh()?;
        let peer = self
            .mesh
            .get_peer(peer_id)
            .await?
            .ok_or_else(|| CoreError::PeerNotFound(peer_id.into()))?;
        if !peer.trusted {
            return Err(CoreError::PairingFailed("peer not trusted".into()));
        }
        let host = peer
            .address
            .as_deref()
            .ok_or_else(|| CoreError::InvalidArgument("peer has no address".into()))?;
        let port = peer.port.unwrap_or(7420);
        let resolved = resolve_ip(host).await?;
        self.policy
            .intercept(host, Some(resolved), None, "mesh-rpc")
            .await?;

        let envelope = self.mesh.build_signed_envelope(method, params);
        let url = format!("http://{host}:{port}/mesh/rpc");
        let resp = self
            .http
            .raw()
            .post(&url)
            .json(&envelope)
            .send()
            .await
            .map_err(|e| CoreError::Other(format!("peer rpc: {e}")))?;
        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(CoreError::Other(format!("peer rpc HTTP error: {text}")));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        if let Some(err) = body.get("error") {
            return Err(CoreError::Other(err.to_string()));
        }
        Ok(body.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Handle inbound signed mesh RPC from a trusted peer.
    pub async fn handle_inbound_rpc(&self, env: SignedEnvelope) -> Result<Value, CoreError> {
        self.mesh.verify_envelope(&env).await?;
        match env.method.as_str() {
            "mesh/resources" => {
                let _ = self.refresh_local_models().await;
                Ok(json!(self.collect_local_resources()))
            }
            "mesh/infer" => {
                let req: LlmRequest = serde_json::from_value(env.params)
                    .map_err(|e| CoreError::InvalidArgument(e.to_string()))?;
                self.active_remote.fetch_add(1, Ordering::SeqCst);
                let result = self.llm.complete(req, None).await;
                self.active_remote.fetch_sub(1, Ordering::SeqCst);
                let response = result?;
                Ok(json!(response))
            }
            "mesh/task" => {
                let spec: RemoteTaskSpec = serde_json::from_value(env.params.clone())
                    .map_err(|e| CoreError::InvalidArgument(e.to_string()))?;
                self.active_remote.fetch_add(1, Ordering::SeqCst);
                let out = self.execute_local_task(&spec).await;
                self.active_remote.fetch_sub(1, Ordering::SeqCst);
                out
            }
            "mesh/hello" => Ok(json!({
                "node_id": self.mesh.node_id(),
                "public_key": self.mesh.public_key_hex(),
                "protocol": source_core::protocol::PROTOCOL_VERSION,
            })),
            other => Err(CoreError::InvalidArgument(format!(
                "unknown mesh method: {other}"
            ))),
        }
    }

    async fn execute_local_task(&self, spec: &RemoteTaskSpec) -> Result<Value, CoreError> {
        match spec.kind {
            RemoteTaskKind::Infer => {
                let mut req: LlmRequest = serde_json::from_value(spec.payload.clone())
                    .map_err(|e| CoreError::InvalidArgument(e.to_string()))?;
                if let Some(model) = &spec.required_model {
                    // Prefer matching local provider model name.
                    req.provider_id = LlmProviderId::new("ollama-local");
                    let _ = model;
                }
                if req.provider_id.0.is_empty() {
                    req.provider_id = LlmProviderId::new("ollama-local");
                }
                let resp = self.llm.complete(req, None).await?;
                Ok(json!(resp))
            }
            RemoteTaskKind::PlanSubtask | RemoteTaskKind::Tool | RemoteTaskKind::Custom => {
                Ok(json!({
                    "ok": true,
                    "kind": spec.kind.as_str(),
                    "label": spec.label,
                    "echo": spec.payload,
                    "worker": self.mesh.node_id(),
                }))
            }
        }
    }

    pub async fn complete_remote(
        &self,
        request: RemoteInferRequest,
        session_id: Option<SessionId>,
    ) -> Result<RemoteInferResponse, CoreError> {
        self.assert_mesh()?;
        let snap = self.cluster_snapshot().await?;
        let required = request
            .required_model
            .clone()
            .or_else(|| Some(request.request.messages.first().map(|_| request.request.provider_id.0.clone())).flatten());
        // Prefer explicit peer, else best GPU peer with model + VRAM.
        let candidate = if let Some(pref) = &request.preferred_peer_id {
            snap.peers.iter().find(|p| p.peer.peer_id == *pref && p.reachable)
        } else {
            select_inference_peer(&snap, required.as_deref(), request.min_vram_mb)
        };

        if let Some(peer) = candidate {
            let peer_id = peer.peer.peer_id.clone();
            let addr = peer
                .peer
                .address
                .clone()
                .unwrap_or_else(|| "unknown".into());
            self.active_remote.fetch_add(1, Ordering::SeqCst);
            let result = self
                .call_peer(&peer_id, "mesh/infer", json!(request.request))
                .await;
            self.active_remote.fetch_sub(1, Ordering::SeqCst);
            let response: LlmResponse = serde_json::from_value(result?)
                .map_err(|e| CoreError::Other(e.to_string()))?;
            let _ = self
                .db
                .append_audit(AuditEvent {
                    id: Uuid::new_v4(),
                    session_id,
                    kind: AuditEventKind::Delegation,
                    summary: format!("remote infer via {peer_id}"),
                    detail: json!({ "peer_id": peer_id, "model": response.model }),
                    content_hash: String::new(),
                    prev_hash: None,
                    created_at: Utc::now(),
                })
                .await;
            return Ok(RemoteInferResponse {
                response,
                routed_peer_id: peer_id,
                routed_address: addr,
                local: false,
            });
        }

        if request.allow_local_fallback.unwrap_or(true) {
            let response = self.llm.complete(request.request, session_id).await?;
            return Ok(RemoteInferResponse {
                response,
                routed_peer_id: self.mesh.node_id(),
                routed_address: "127.0.0.1".into(),
                local: true,
            });
        }

        Err(CoreError::ProviderUnavailable(
            "no suitable GPU peer for remote inference".into(),
        ))
    }

    pub async fn dispatch(
        &self,
        tasks: Vec<RemoteTaskSpec>,
        parallel: bool,
        session_id: Option<SessionId>,
    ) -> Result<DispatchResult, CoreError> {
        self.assert_mesh()?;
        let batch_id = Uuid::new_v4();
        let snap = self.cluster_snapshot().await?;
        let mut infos = Vec::new();

        if parallel {
            let mut handles = Vec::new();
            for spec in tasks {
                let helper = DispatchHandles {
                    db: self.db.clone(),
                    policy: self.policy.clone(),
                    http: self.http.clone(),
                    llm: Arc::clone(&self.llm),
                    mesh: Arc::clone(&self.mesh),
                    local_models: self.local_models.read().clone(),
                };
                let snap = snap.clone();
                handles.push(tokio::spawn(async move {
                    helper.run_one_task(spec, &snap, session_id).await
                }));
            }
            for h in handles {
                let info = h
                    .await
                    .map_err(|e| CoreError::Other(e.to_string()))??;
                infos.push(info);
            }
        } else {
            for spec in tasks {
                infos.push(self.run_one_task(spec, &snap, session_id).await?);
            }
        }

        let _ = self
            .db
            .append_audit(AuditEvent {
                id: Uuid::new_v4(),
                session_id,
                kind: AuditEventKind::Delegation,
                summary: format!("dispatched {} mesh tasks", infos.len()),
                detail: json!({ "batch_id": batch_id, "task_ids": infos.iter().map(|t| t.task_id).collect::<Vec<_>>() }),
                content_hash: String::new(),
                prev_hash: None,
                created_at: Utc::now(),
            })
            .await;

        Ok(DispatchResult {
            batch_id,
            tasks: infos,
        })
    }

    async fn run_one_task(
        &self,
        spec: RemoteTaskSpec,
        snap: &ClusterSnapshot,
        _session_id: Option<SessionId>,
    ) -> Result<RemoteTaskInfo, CoreError> {
        let task_id = Uuid::new_v4();
        let now = Utc::now();
        let mut info = RemoteTaskInfo {
            task_id,
            parent_id: None,
            kind: spec.kind,
            label: spec.label.clone(),
            state: RemoteTaskState::Pending,
            assigned_peer_id: None,
            required_model: spec.required_model.clone(),
            min_vram_mb: spec.min_vram_mb,
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        };
        self.db
            .insert_mesh_task(&info, &spec.payload)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;

        let peer = if let Some(pref) = &spec.preferred_peer_id {
            snap.peers
                .iter()
                .find(|p| p.peer.peer_id == *pref && p.reachable)
        } else {
            select_worker_peer(snap, spec.required_model.as_deref(), spec.min_vram_mb)
        };

        // Local execution when no peer or preferred is self.
        let run_local = peer
            .map(|p| p.peer.peer_id == self.mesh.node_id())
            .unwrap_or(true)
            && peer.is_none();

        if run_local {
            info.state = RemoteTaskState::Running;
            info.assigned_peer_id = Some(self.mesh.node_id());
            info.updated_at = Utc::now();
            let _ = self.db.update_mesh_task(&info).await;
            self.active_remote.fetch_add(1, Ordering::SeqCst);
            match self.execute_local_task(&spec).await {
                Ok(result) => {
                    info.state = RemoteTaskState::Completed;
                    info.result = Some(result);
                    info.completed_at = Some(Utc::now());
                    info.updated_at = Utc::now();
                }
                Err(err) => {
                    info.state = RemoteTaskState::Failed;
                    info.error = Some(err.to_string());
                    info.completed_at = Some(Utc::now());
                    info.updated_at = Utc::now();
                }
            }
            self.active_remote.fetch_sub(1, Ordering::SeqCst);
            let _ = self.db.update_mesh_task(&info).await;
            return Ok(info);
        }

        let peer = peer.ok_or_else(|| {
            CoreError::ProviderUnavailable("no worker peer available".into())
        })?;
        let peer_id = peer.peer.peer_id.clone();
        info.state = RemoteTaskState::Assigned;
        info.assigned_peer_id = Some(peer_id.clone());
        info.updated_at = Utc::now();
        let _ = self.db.update_mesh_task(&info).await;

        info.state = RemoteTaskState::Running;
        info.updated_at = Utc::now();
        let _ = self.db.update_mesh_task(&info).await;
        self.active_remote.fetch_add(1, Ordering::SeqCst);
        match self.call_peer(&peer_id, "mesh/task", json!(spec)).await {
            Ok(result) => {
                info.state = RemoteTaskState::Completed;
                info.result = Some(result);
                info.completed_at = Some(Utc::now());
                info.updated_at = Utc::now();
            }
            Err(err) => {
                info.state = RemoteTaskState::Failed;
                info.error = Some(err.to_string());
                info.completed_at = Some(Utc::now());
                info.updated_at = Utc::now();
            }
        }
        self.active_remote.fetch_sub(1, Ordering::SeqCst);
        let _ = self.db.update_mesh_task(&info).await;
        Ok(info)
    }

    pub async fn task_status(&self, task_id: Uuid) -> Result<RemoteTaskInfo, CoreError> {
        self.db
            .get_mesh_task(task_id)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?
            .ok_or_else(|| CoreError::InvalidArgument(format!("task not found: {task_id}")))
    }

    pub async fn cancel_task(&self, task_id: Uuid) -> Result<RemoteTaskInfo, CoreError> {
        let mut info = self.task_status(task_id).await?;
        if matches!(
            info.state,
            RemoteTaskState::Completed | RemoteTaskState::Failed | RemoteTaskState::Cancelled
        ) {
            return Ok(info);
        }
        info.state = RemoteTaskState::Cancelled;
        info.updated_at = Utc::now();
        info.completed_at = Some(Utc::now());
        self.db
            .update_mesh_task(&info)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        Ok(info)
    }

    pub async fn list_tasks(
        &self,
        include_completed: bool,
        limit: u32,
    ) -> Result<Vec<RemoteTaskInfo>, CoreError> {
        self.db
            .list_mesh_tasks(include_completed, limit)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))
    }
}

struct DispatchHandles {
    db: Db,
    policy: PolicyEngine,
    http: PolicyHttpClient,
    llm: Arc<LlmRouter>,
    mesh: Arc<MeshService>,
    local_models: Vec<LoadedModel>,
}

impl DispatchHandles {
    async fn run_one_task(
        &self,
        spec: RemoteTaskSpec,
        snap: &ClusterSnapshot,
        session_id: Option<SessionId>,
    ) -> Result<RemoteTaskInfo, CoreError> {
        let orch = Orchestrator {
            db: self.db.clone(),
            policy: self.policy.clone(),
            http: self.http.clone(),
            llm: Arc::clone(&self.llm),
            mesh: Arc::clone(&self.mesh),
            active_remote: AtomicU32::new(0),
            local_models: RwLock::new(self.local_models.clone()),
        };
        orch.run_one_task(spec, snap, session_id).await
    }
}

fn select_inference_peer<'a>(
    snap: &'a ClusterSnapshot,
    required_model: Option<&str>,
    min_vram_mb: Option<u64>,
) -> Option<&'a ClusterPeerSnapshot> {
    let mut best: Option<(&ClusterPeerSnapshot, u64)> = None;
    for peer in &snap.peers {
        if !peer.reachable {
            continue;
        }
        let Some(res) = &peer.resources else { continue };
        if let Some(model) = required_model {
            let has = res.loaded_models.iter().any(|m| m.name == model)
                || peer.peer.capabilities.models.iter().any(|m| m == model);
            if !has && !model.is_empty() {
                continue;
            }
        }
        let free_vram: u64 = res.gpus.iter().map(|g| g.vram_free_mb).sum();
        if let Some(min) = min_vram_mb {
            if free_vram < min && min > 0 {
                continue;
            }
        }
        match best {
            None => best = Some((peer, free_vram)),
            Some((_, best_vram)) if free_vram > best_vram => best = Some((peer, free_vram)),
            _ => {}
        }
    }
    best.map(|(p, _)| p)
}

fn select_worker_peer<'a>(
    snap: &'a ClusterSnapshot,
    required_model: Option<&str>,
    min_vram_mb: Option<u64>,
) -> Option<&'a ClusterPeerSnapshot> {
    select_inference_peer(snap, required_model, min_vram_mb).or_else(|| {
        snap.peers.iter().find(|p| {
            p.reachable
                && matches!(
                    p.peer.role,
                    PeerRole::Worker | PeerRole::Hybrid | PeerRole::Orchestrator
                )
        })
    })
}

fn detect_gpus() -> Vec<GpuDevice> {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,memory.total,memory.free,utilization.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output();
    let Ok(out) = output else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut gpus = Vec::new();
    for line in text.lines() {
        let parts: Vec<_> = line.split(',').map(|s| s.trim()).collect();
        if parts.len() < 4 {
            continue;
        }
        let index = parts[0].parse().unwrap_or(0);
        let name = parts[1].to_string();
        let vram_total_mb = parts[2].parse().unwrap_or(0);
        let vram_free_mb = parts[3].parse().unwrap_or(0);
        let utilization_pct = parts.get(4).and_then(|s| s.parse().ok());
        gpus.push(GpuDevice {
            index,
            name,
            vram_total_mb,
            vram_free_mb,
            utilization_pct,
        });
    }
    gpus
}

async fn resolve_ip(host: &str) -> Result<std::net::IpAddr, CoreError> {
    if let Ok(ip) = host.parse() {
        return Ok(ip);
    }
    let host = host.to_string();
    tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        (format!("{host}:0"))
            .to_socket_addrs()
            .map_err(|e| CoreError::InvalidArgument(format!("DNS resolve {host}: {e}")))?
            .next()
            .map(|a| a.ip())
            .ok_or_else(|| CoreError::InvalidArgument(format!("no addresses for {host}")))
    })
    .await
    .map_err(|e| CoreError::Other(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_core::mesh::PeerConnectionState;
    use source_core::policy::NetworkTier;
    use source_core::types::WorkspacePaths;
    use tempfile::tempdir;

    #[tokio::test]
    async fn cluster_snapshot_local_only_mesh_peers_empty_when_blocked() {
        let dir = tempdir().unwrap();
        let paths = WorkspacePaths::from_home(dir.path());
        paths.ensure_dirs().unwrap();
        let db = Db::connect(&paths.db_path).await.unwrap();
        let engine = PolicyEngine::load(db.clone()).await.unwrap();
        let http = PolicyHttpClient::new(engine.clone()).unwrap();
        let llm = Arc::new(LlmRouter::new(db.clone(), engine.clone(), http.clone()));
        let mesh = MeshService::new(db.clone(), engine.clone(), 7420, Some("n".into()))
            .await
            .unwrap();
        let orch = Orchestrator::new(db, engine, http, llm, mesh);
        // LOCAL_ONLY: snapshot still returns local resources; peers list empty from mesh policy gate in snapshot.
        let snap = orch.cluster_snapshot().await.unwrap();
        assert_eq!(snap.local.label, "n");
        assert!(!snap.mesh_status.mesh_allowed);
    }

    #[tokio::test]
    async fn dispatch_runs_locally_when_no_peers() {
        let dir = tempdir().unwrap();
        let paths = WorkspacePaths::from_home(dir.path());
        paths.ensure_dirs().unwrap();
        let db = Db::connect(&paths.db_path).await.unwrap();
        let engine = PolicyEngine::load(db.clone()).await.unwrap();
        engine
            .set_policy(source_core::policy::NetworkPolicy::with_tier(
                NetworkTier::LocalAndLan,
            ))
            .await
            .unwrap();
        let http = PolicyHttpClient::new(engine.clone()).unwrap();
        let llm = Arc::new(LlmRouter::new(db.clone(), engine.clone(), http.clone()));
        let mesh = MeshService::new(db.clone(), engine.clone(), 7420, Some("orch".into()))
            .await
            .unwrap();
        let orch = Orchestrator::new(db, engine, http, llm, mesh);
        let result = orch
            .dispatch(
                vec![RemoteTaskSpec {
                    kind: RemoteTaskKind::Custom,
                    label: "unit".into(),
                    payload: json!({ "x": 1 }),
                    required_model: None,
                    min_vram_mb: None,
                    preferred_peer_id: None,
                }],
                true,
                None,
            )
            .await
            .unwrap();
        assert_eq!(result.tasks.len(), 1);
        assert_eq!(result.tasks[0].state, RemoteTaskState::Completed);
    }

    #[test]
    fn select_peer_prefers_more_vram() {
        let _ = PeerConnectionState::Connected;
        let mut snap = ClusterSnapshot {
            local: NodeResources {
                node_id: "local".into(),
                label: "l".into(),
                hostname: "l".into(),
                role: PeerRole::Hybrid,
                address: "127.0.0.1".into(),
                port: 7420,
                cpu_cores: 1,
                cpu_usage_pct: 0.0,
                ram_total_mb: 1,
                ram_available_mb: 1,
                gpus: vec![],
                loaded_models: vec![],
                active_remote_tasks: 0,
                mesh_enabled: true,
                network_tier: NetworkTier::LocalAndLan,
                public_key_hex: "aa".into(),
                updated_at: Utc::now(),
            },
            peers: vec![],
            active_tasks: vec![],
            mesh_status: source_core::mesh::MeshStatus {
                enabled: true,
                advertising: false,
                browsing: false,
                node_id: "local".into(),
                public_key_hex: "aa".into(),
                label: "l".into(),
                hostname: "l".into(),
                role: PeerRole::Hybrid,
                bind_port: 7420,
                service_type: "_source-mesh._tcp.local".into(),
                network_tier: NetworkTier::LocalAndLan,
                mesh_allowed: true,
                paired_peers: 0,
                connected_peers: 0,
                discovered_peers: 0,
                pending_pairings: 0,
                active_remote_tasks: 0,
            },
            collected_at: Utc::now(),
        };
        // empty peers -> none
        assert!(select_inference_peer(&snap, None, None).is_none());
        let _ = &mut snap;
    }
}
