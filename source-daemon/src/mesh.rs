//! LAN / Tailscale mesh: mDNS discovery, manual peer registration, Ed25519+PIN pairing.

use anyhow::{anyhow, Result};
use chrono::{Duration, Utc};
use dashmap::DashMap;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use source_core::error::CoreError;
use source_core::mesh::{
    mesh_tier_allowed, DiscoveredPeer, MeshPeerInfo, MeshStatus, PeerCapabilities,
    PeerConnectionState, PeerRole, PeerSource, MESH_SERVICE_TYPE, MESH_SERVICE_TYPE_BARE,
};
use source_core::policy::PeerKeyRecord;
use source_core::types::AuditEventKind;
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration as StdDuration;
use uuid::Uuid;

use crate::db::Db;
use crate::identity::{
    derive_session_secret, fingerprint_of_public_key, generate_pin, parse_verifying_key,
    pin_shared_secret, verify_signature, x25519_ephemeral_keypair, x25519_shared, NodeIdentity,
};
use crate::policy::PolicyEngine;

const PAIRING_TTL_SECS: i64 = 300;

#[derive(Clone)]
struct PendingPairing {
    pin: String,
    expires_at: chrono::DateTime<Utc>,
    label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairOffer {
    pub pairing_id: Uuid,
    pub node_id: String,
    pub public_key_hex: String,
    pub label: String,
    pub role: String,
    pub x25519_public_hex: String,
    pub expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairAccept {
    pub pairing_id: Uuid,
    pub pin: String,
    pub peer_id: String,
    pub public_key_hex: String,
    pub label: String,
    pub role: String,
    pub x25519_public_hex: String,
    /// HMAC-SHA256(pin_secret, local_pk || remote_pk || x25519_pub) hex
    pub pin_mac: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairComplete {
    pub ok: bool,
    pub peer_id: String,
    pub public_key_hex: String,
    pub label: String,
    pub role: String,
    pub session_secret_hex: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedEnvelope {
    pub from_node_id: String,
    pub from_public_key: String,
    pub timestamp_unix: i64,
    pub nonce: String,
    pub method: String,
    pub params: Value,
    pub signature_hex: String,
}

impl SignedEnvelope {
    pub fn signing_payload(&self) -> String {
        format!(
            "source-mesh-rpc-v1|{}|{}|{}|{}|{}|{}",
            self.from_node_id,
            self.from_public_key,
            self.timestamp_unix,
            self.nonce,
            self.method,
            self.params
        )
    }
}

pub struct MeshService {
    db: Db,
    policy: PolicyEngine,
    identity: Arc<RwLock<NodeIdentity>>,
    bind_port: u16,
    advertising: AtomicBool,
    browsing: AtomicBool,
    enabled: AtomicBool,
    discovered: DashMap<String, DiscoveredPeer>,
    pending: DashMap<Uuid, PendingPairing>,
    connected: DashMap<String, PeerConnectionState>,
    /// Ephemeral X25519 secrets keyed by pairing_id for in-flight pairing.
    x25519_secrets: DashMap<Uuid, x25519_dalek::StaticSecret>,
    mdns: RwLock<Option<ServiceDaemon>>,
    browse_stop: AtomicBool,
}

impl MeshService {
    pub async fn new(
        db: Db,
        policy: PolicyEngine,
        bind_port: u16,
        default_label: Option<String>,
    ) -> Result<Arc<Self>> {
        let identity = NodeIdentity::load_or_create(&db, default_label).await?;
        Ok(Arc::new(Self {
            db,
            policy,
            identity: Arc::new(RwLock::new(identity)),
            bind_port,
            advertising: AtomicBool::new(false),
            browsing: AtomicBool::new(false),
            enabled: AtomicBool::new(false),
            discovered: DashMap::new(),
            pending: DashMap::new(),
            connected: DashMap::new(),
            x25519_secrets: DashMap::new(),
            mdns: RwLock::new(None),
            browse_stop: AtomicBool::new(false),
        }))
    }

    pub fn identity_snapshot(&self) -> (String, String, String, PeerRole) {
        let id = self.identity.read();
        (
            id.node_id.clone(),
            id.public_key_hex(),
            id.label.clone(),
            id.role,
        )
    }

    pub fn node_id(&self) -> String {
        self.identity.read().node_id.clone()
    }

    pub fn listen_port(&self) -> u16 {
        self.bind_port
    }

    pub fn public_key_hex(&self) -> String {
        self.identity.read().public_key_hex()
    }

    pub fn sign_message(&self, message: &[u8]) -> String {
        self.identity.read().sign(message)
    }

    fn assert_mesh_allowed(&self) -> Result<(), CoreError> {
        let tier = self.policy.tier();
        if !mesh_tier_allowed(tier) {
            return Err(CoreError::MeshDisabled(format!(
                "network tier {} does not permit mesh (need LOCAL_AND_LAN or higher)",
                tier.as_str()
            )));
        }
        Ok(())
    }

    pub async fn start(
        self: &Arc<Self>,
        advertise: bool,
        browse: bool,
        role: Option<PeerRole>,
        label: Option<String>,
    ) -> Result<MeshStatus, CoreError> {
        self.assert_mesh_allowed()?;
        {
            let mut id = self.identity.write();
            if let Some(r) = role {
                id.set_role(r);
            }
            if let Some(l) = label {
                id.set_label(l);
            }
        }
        let persist = {
            let id = self.identity.read();
            (id.node_id.clone(), id.label.clone(), id.role)
        };
        // Drop identity guard before awaiting DB I/O (parking_lot guards are !Send).
        self.db
            .update_node_identity_meta(&persist.0, &persist.1, persist.2.as_str())
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;

        self.enabled.store(true, Ordering::SeqCst);

        if advertise {
            self.start_advertise()
                .map_err(|e| CoreError::Other(e.to_string()))?;
        }
        if browse {
            self.start_browse()
                .map_err(|e| CoreError::Other(e.to_string()))?;
        }

        self.audit(
            "mesh started",
            json!({
                "advertise": advertise,
                "browse": browse,
                "node_id": self.node_id(),
            }),
        )
        .await;
        Ok(self.status().await)
    }

    pub async fn stop(&self) -> Result<MeshStatus, CoreError> {
        self.browse_stop.store(true, Ordering::SeqCst);
        self.advertising.store(false, Ordering::SeqCst);
        self.browsing.store(false, Ordering::SeqCst);
        self.enabled.store(false, Ordering::SeqCst);
        if let Some(_daemon) = self.mdns.write().take() {
            // Dropping ServiceDaemon stops advertising/browsing.
        }
        self.audit("mesh stopped", json!({})).await;
        Ok(self.status().await)
    }

    fn ensure_mdns(&self) -> Result<ServiceDaemon> {
        let mut guard = self.mdns.write();
        if guard.is_none() {
            *guard = Some(ServiceDaemon::new().map_err(|e| anyhow!("mdns daemon: {e}"))?);
        }
        // ServiceDaemon is not Clone — we keep one and clone via re-get is hard.
        // Instead, create a fresh daemon if needed for operations.
        drop(guard);
        ServiceDaemon::new().map_err(|e| anyhow!("mdns daemon: {e}"))
    }

    fn start_advertise(&self) -> Result<()> {
        let daemon = ServiceDaemon::new().map_err(|e| anyhow!("mdns: {e}"))?;
        let id = self.identity.read();
        let instance = format!("source-{}", &id.node_id[..8.min(id.node_id.len())]);
        let host_name = format!("{instance}.local.");
        let mut props = HashMap::new();
        props.insert("node_id".into(), id.node_id.clone());
        props.insert("pk".into(), id.public_key_hex());
        props.insert("label".into(), id.label.clone());
        props.insert("role".into(), id.role.as_str().to_string());
        props.insert("proto".into(), source_core::protocol::PROTOCOL_VERSION.into());
        props.insert("fp".into(), id.fingerprint());

        // Prefer advertising on non-loopback IPv4 when available.
        let ip = std::net::IpAddr::V4(local_ipv4().unwrap_or(std::net::Ipv4Addr::LOCALHOST));
        let service = ServiceInfo::new(
            MESH_SERVICE_TYPE,
            &instance,
            &host_name,
            ip,
            self.bind_port,
            Some(props),
        )
        .map_err(|e| anyhow!("service info: {e}"))?;
        daemon
            .register(service)
            .map_err(|e| anyhow!("mdns register: {e}"))?;
        *self.mdns.write() = Some(daemon);
        self.advertising.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn start_browse(self: &Arc<Self>) -> Result<()> {
        let daemon = self.ensure_mdns()?;
        let receiver = daemon
            .browse(MESH_SERVICE_TYPE)
            .map_err(|e| anyhow!("mdns browse: {e}"))?;
        self.browse_stop.store(false, Ordering::SeqCst);
        self.browsing.store(true, Ordering::SeqCst);
        let this = Arc::clone(self);
        std::thread::Builder::new()
            .name("source-mdns-browse".into())
            .spawn(move || {
                while !this.browse_stop.load(Ordering::SeqCst) {
                    match receiver.recv_timeout(StdDuration::from_millis(500)) {
                        Ok(ServiceEvent::ServiceResolved(info)) => {
                            this.on_resolved(info);
                        }
                        Ok(ServiceEvent::ServiceRemoved(_, fullname)) => {
                            this.discovered.remove(&fullname);
                        }
                        Ok(_) => {}
                        Err(flume::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                }
                this.browsing.store(false, Ordering::SeqCst);
            })
            .map_err(|e| anyhow!("spawn browse: {e}"))?;
        // Keep a daemon alive for browse lifetime.
        if self.mdns.read().is_none() {
            *self.mdns.write() = Some(daemon);
        }
        Ok(())
    }

    fn on_resolved(&self, info: ServiceInfo) {
        let props = info.get_properties();
        let mut txt = HashMap::new();
        for prop in props.iter() {
            txt.insert(prop.key().to_string(), prop.val_str().to_string());
        }
        let peer_id = txt.get("node_id").cloned();
        // Skip self.
        if peer_id.as_deref() == Some(self.node_id().as_str()) {
            return;
        }
        let addresses: Vec<String> = info
            .get_addresses()
            .iter()
            .map(|a| a.to_string())
            .collect();
        let host = addresses
            .first()
            .cloned()
            .unwrap_or_else(|| info.get_hostname().trim_end_matches('.').to_string());
        let key = info.get_fullname().to_string();
        self.discovered.insert(
            key.clone(),
            DiscoveredPeer {
                instance_name: key,
                peer_id,
                host,
                port: info.get_port(),
                addresses,
                txt,
                discovered_at: Utc::now(),
            },
        );
    }

    pub async fn status(&self) -> MeshStatus {
        let (node_id, public_key_hex, label, role) = {
            let id = self.identity.read();
            (
                id.node_id.clone(),
                id.public_key_hex(),
                id.label.clone(),
                id.role,
            )
        };
        let peers = self.db.list_peer_keys().await.unwrap_or_default();
        let paired = peers.iter().filter(|p| p.trusted).count();
        let connected = self
            .connected
            .iter()
            .filter(|e| *e.value() == PeerConnectionState::Connected)
            .count();
        MeshStatus {
            enabled: self.enabled.load(Ordering::SeqCst),
            advertising: self.advertising.load(Ordering::SeqCst),
            browsing: self.browsing.load(Ordering::SeqCst),
            node_id,
            public_key_hex,
            label: label.clone(),
            hostname: hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or(label),
            role,
            bind_port: self.bind_port,
            service_type: MESH_SERVICE_TYPE_BARE.into(),
            network_tier: self.policy.tier(),
            mesh_allowed: self.policy.current().mesh_allowed(),
            paired_peers: paired,
            connected_peers: connected,
            discovered_peers: self.discovered.len(),
            pending_pairings: self.pending.len(),
            active_remote_tasks: 0,
        }
    }

    pub fn list_discovered(&self) -> Vec<DiscoveredPeer> {
        let mut v: Vec<_> = self.discovered.iter().map(|e| e.value().clone()).collect();
        v.sort_by(|a, b| a.instance_name.cmp(&b.instance_name));
        v
    }

    pub async fn list_peers(&self) -> Result<Vec<MeshPeerInfo>, CoreError> {
        let rows = self
            .db
            .list_peer_keys()
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        Ok(rows
            .into_iter()
            .map(|r| self.record_to_info(r))
            .collect())
    }

    fn record_to_info(&self, r: PeerKeyRecord) -> MeshPeerInfo {
        let caps: PeerCapabilities = r
            .capabilities_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let state = self
            .connected
            .get(&r.peer_id)
            .map(|s| *s)
            .unwrap_or(if r.trusted {
                PeerConnectionState::Paired
            } else {
                PeerConnectionState::Untrusted
            });
        MeshPeerInfo {
            peer_id: r.peer_id,
            public_key: r.public_key,
            label: r.label,
            trusted: r.trusted,
            address: r.address,
            port: r.port,
            source: r
                .source
                .as_deref()
                .and_then(|s| PeerSource::from_str(s).ok())
                .unwrap_or(PeerSource::Manual),
            role: r
                .role
                .as_deref()
                .and_then(|s| PeerRole::from_str(s).ok())
                .unwrap_or(PeerRole::Hybrid),
            state,
            fingerprint: r.fingerprint,
            capabilities: caps,
            last_seen_at: r.last_seen_at,
            paired_at: r.paired_at,
            created_at: r.created_at,
        }
    }

    /// Manual IP / Tailscale MagicDNS registration (does not trust until paired).
    pub async fn register_peer(
        &self,
        host: &str,
        port: u16,
        label: Option<String>,
        source: PeerSource,
        peer_id: Option<String>,
    ) -> Result<MeshPeerInfo, CoreError> {
        self.assert_mesh_allowed()?;
        let host = host.trim();
        if host.is_empty() {
            return Err(CoreError::InvalidArgument("host required".into()));
        }
        let resolved = resolve_host(host).await?;
        self.policy
            .intercept(host, Some(resolved), None, "mesh-register")
            .await?;

        let peer_id = peer_id.unwrap_or_else(|| format!("pending-{}", Uuid::new_v4()));
        let now = Utc::now();
        let record = PeerKeyRecord {
            peer_id: peer_id.clone(),
            public_key: String::new(),
            label: label.or_else(|| Some(host.to_string())),
            trusted: false,
            created_at: now,
            address: Some(host.to_string()),
            port: Some(port),
            fingerprint: None,
            source: Some(source.as_str().into()),
            role: Some(PeerRole::Hybrid.as_str().into()),
            capabilities_json: Some("{}".into()),
            last_seen_at: Some(now),
            paired_at: None,
        };
        self.db
            .upsert_peer_key(&record, None)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        self.connected
            .insert(peer_id.clone(), PeerConnectionState::Discovered);
        self.audit(
            "peer registered",
            json!({ "peer_id": peer_id, "host": host, "port": port, "source": source.as_str() }),
        )
        .await;
        let info = self
            .list_peers()
            .await?
            .into_iter()
            .find(|p| p.peer_id == peer_id)
            .ok_or_else(|| CoreError::PeerNotFound(peer_id))?;
        Ok(info)
    }

    pub async fn unregister_peer(&self, peer_id: &str) -> Result<(), CoreError> {
        self.db
            .delete_peer_key(peer_id)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        self.connected.remove(peer_id);
        self.audit("peer unregistered", json!({ "peer_id": peer_id }))
            .await;
        Ok(())
    }

    pub async fn begin_pairing(
        &self,
        label: Option<String>,
        ttl_secs: Option<u64>,
    ) -> Result<source_core::mesh::PairingSession, CoreError> {
        self.assert_mesh_allowed()?;
        let ttl = ttl_secs.unwrap_or(PAIRING_TTL_SECS as u64) as i64;
        let pairing_id = Uuid::new_v4();
        let pin = generate_pin();
        let expires_at = Utc::now() + Duration::seconds(ttl);
        let (node_id, public_key_hex, default_label, role) = {
            let id = self.identity.read();
            (
                id.node_id.clone(),
                id.public_key_hex(),
                id.label.clone(),
                id.role,
            )
        };
        self.pending.insert(
            pairing_id,
            PendingPairing {
                pin: pin.clone(),
                expires_at,
                label: label.clone().unwrap_or_else(|| default_label.clone()),
            },
        );
        // Prepare ephemeral DH for this pairing session.
        let (secret, public) = x25519_ephemeral_keypair();
        self.x25519_secrets.insert(pairing_id, secret);
        let _ = public; // offer embeds x25519 when remote completes pairing over HTTP

        self.audit(
            "pairing begun",
            json!({ "pairing_id": pairing_id, "expires_at": expires_at }),
        )
        .await;

        Ok(source_core::mesh::PairingSession {
            pairing_id,
            pin,
            node_id,
            public_key_hex,
            label: label.unwrap_or(default_label),
            expires_at,
            role: role.as_str().into(),
        })
    }

    pub fn get_pair_offer(&self, pairing_id: Uuid) -> Result<PairOffer, CoreError> {
        self.assert_mesh_allowed()?;
        let pending = self
            .pending
            .get(&pairing_id)
            .ok_or_else(|| CoreError::PairingFailed("unknown pairing_id".into()))?
            .clone();
        if Utc::now() > pending.expires_at {
            self.pending.remove(&pairing_id);
            return Err(CoreError::PairingFailed("pairing expired".into()));
        }
        let (secret, public) = if let Some(s) = self.x25519_secrets.get(&pairing_id) {
            let public = x25519_dalek::PublicKey::from(s.value());
            (None, public)
        } else {
            let (secret, public) = x25519_ephemeral_keypair();
            self.x25519_secrets.insert(pairing_id, secret);
            let public = public;
            (None::<()>, public)
        };
        let _ = secret;
        let id = self.identity.read();
        Ok(PairOffer {
            pairing_id,
            node_id: id.node_id.clone(),
            public_key_hex: id.public_key_hex(),
            label: pending.label,
            role: id.role.as_str().into(),
            x25519_public_hex: hex::encode(public.as_bytes()),
            expires_at: pending.expires_at,
        })
    }

    /// Local confirmation after out-of-band PIN check (same host UI path).
    pub async fn confirm_pairing(
        &self,
        pairing_id: Uuid,
        peer_id: &str,
        peer_public_key: &str,
        pin: &str,
        label: Option<String>,
        address: Option<String>,
        port: Option<u16>,
    ) -> Result<MeshPeerInfo, CoreError> {
        self.assert_mesh_allowed()?;
        let pending = self
            .pending
            .get(&pairing_id)
            .ok_or_else(|| CoreError::PairingFailed("unknown pairing_id".into()))?
            .clone();
        if Utc::now() > pending.expires_at {
            self.pending.remove(&pairing_id);
            return Err(CoreError::PairingFailed("pairing expired".into()));
        }
        if pending.pin != pin {
            return Err(CoreError::PairingFailed("PIN mismatch".into()));
        }
        parse_verifying_key(peer_public_key).map_err(|e| CoreError::PairingFailed(e.to_string()))?;

        let local_pk = self.public_key_hex();
        let pin_secret = pin_shared_secret(pin, &local_pk, peer_public_key);
        let session_secret = derive_session_secret(&pin_secret, &pin_secret);
        let fp = fingerprint_of_public_key(peer_public_key);
        let now = Utc::now();
        let record = PeerKeyRecord {
            peer_id: peer_id.to_string(),
            public_key: peer_public_key.to_string(),
            label: label.or(Some(pending.label)),
            trusted: true,
            created_at: now,
            address,
            port,
            fingerprint: Some(fp),
            source: Some(PeerSource::Manual.as_str().into()),
            role: Some(PeerRole::Hybrid.as_str().into()),
            capabilities_json: Some("{}".into()),
            last_seen_at: Some(now),
            paired_at: Some(now),
        };
        self.db
            .upsert_peer_key(&record, Some(&hex::encode(session_secret)))
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        self.pending.remove(&pairing_id);
        self.x25519_secrets.remove(&pairing_id);
        self.connected
            .insert(peer_id.to_string(), PeerConnectionState::Paired);
        self.audit(
            "peer paired",
            json!({ "peer_id": peer_id, "pairing_id": pairing_id }),
        )
        .await;
        self.list_peers()
            .await?
            .into_iter()
            .find(|p| p.peer_id == peer_id)
            .ok_or_else(|| CoreError::PeerNotFound(peer_id.into()))
    }

    /// Dial remote peer and complete PIN + X25519 + Ed25519 pairing (mTLS-style key exchange).
    pub async fn complete_pairing_remote(
        &self,
        host: &str,
        port: u16,
        pin: &str,
        label: Option<String>,
        http: &reqwest::Client,
    ) -> Result<MeshPeerInfo, CoreError> {
        self.assert_mesh_allowed()?;
        let host = host.trim();
        let resolved = resolve_host(host).await?;
        self.policy
            .intercept(host, Some(resolved), None, "mesh-pair")
            .await?;

        let base = format!("http://{host}:{port}");
        // Discover active pairing offer (most recent) via /mesh/pair/offer
        let offer_url = format!("{base}/mesh/pair/offer");
        let offer_resp = http
            .get(&offer_url)
            .send()
            .await
            .map_err(|e| CoreError::PairingFailed(format!("offer fetch: {e}")))?;
        if !offer_resp.status().is_success() {
            return Err(CoreError::PairingFailed(format!(
                "remote offer HTTP {}",
                offer_resp.status()
            )));
        }
        let offer: PairOffer = offer_resp
            .json()
            .await
            .map_err(|e| CoreError::PairingFailed(format!("offer json: {e}")))?;
        // PIN is shown only on the remote console/UI; initiator supplies it in PairAccept.
        if Utc::now() > offer.expires_at {
            return Err(CoreError::PairingFailed("remote pairing expired".into()));
        }

        let (eph_secret, eph_public) = x25519_ephemeral_keypair();
        let local = self.identity.read().clone_meta();
        let pin_secret = pin_shared_secret(pin, &local.1, &offer.public_key_hex);
        let pin_mac = {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            type HmacSha256 = Hmac<Sha256>;
            let mut mac = HmacSha256::new_from_slice(&pin_secret)
                .map_err(|e| CoreError::Other(e.to_string()))?;
            mac.update(local.1.as_bytes());
            mac.update(offer.public_key_hex.as_bytes());
            mac.update(eph_public.as_bytes());
            hex::encode(mac.finalize().into_bytes())
        };

        let accept = PairAccept {
            pairing_id: offer.pairing_id,
            pin: pin.to_string(),
            peer_id: local.0.clone(),
            public_key_hex: local.1.clone(),
            label: label.unwrap_or(local.2),
            role: local.3.as_str().into(),
            x25519_public_hex: hex::encode(eph_public.as_bytes()),
            pin_mac,
        };

        let accept_url = format!("{base}/mesh/pair/accept");
        let complete_resp = http
            .post(&accept_url)
            .json(&accept)
            .send()
            .await
            .map_err(|e| CoreError::PairingFailed(format!("accept post: {e}")))?;
        if !complete_resp.status().is_success() {
            let text = complete_resp.text().await.unwrap_or_default();
            return Err(CoreError::PairingFailed(format!(
                "remote accept failed: {text}"
            )));
        }
        let complete: PairComplete = complete_resp
            .json()
            .await
            .map_err(|e| CoreError::PairingFailed(format!("complete json: {e}")))?;

        let dh = x25519_shared(&eph_secret, &offer.x25519_public_hex)
            .map_err(|e| CoreError::PairingFailed(e.to_string()))?;
        let session = derive_session_secret(&dh, &pin_secret);
        // Remote returns its derived secret hex for confirmation.
        let remote_session = hex::decode(&complete.session_secret_hex)
            .map_err(|e| CoreError::PairingFailed(format!("session decode: {e}")))?;
        if remote_session.as_slice() != session.as_slice() {
            return Err(CoreError::PairingFailed(
                "session secret mismatch (PIN/mTLS key exchange failed)".into(),
            ));
        }

        let now = Utc::now();
        let record = PeerKeyRecord {
            peer_id: complete.peer_id.clone(),
            public_key: complete.public_key_hex.clone(),
            label: Some(complete.label.clone()),
            trusted: true,
            created_at: now,
            address: Some(host.to_string()),
            port: Some(port),
            fingerprint: Some(complete.fingerprint.clone()),
            source: Some(
                if host.contains(".ts.net") || host.ends_with(".tailscale.net") {
                    PeerSource::Tailscale
                } else {
                    PeerSource::Manual
                }
                .as_str()
                .into(),
            ),
            role: Some(complete.role.clone()),
            capabilities_json: Some("{}".into()),
            last_seen_at: Some(now),
            paired_at: Some(now),
        };
        self.db
            .upsert_peer_key(&record, Some(&hex::encode(session)))
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        self.connected
            .insert(complete.peer_id.clone(), PeerConnectionState::Connected);
        self.audit(
            "remote pairing complete",
            json!({ "peer_id": complete.peer_id, "host": host }),
        )
        .await;
        self.list_peers()
            .await?
            .into_iter()
            .find(|p| p.peer_id == complete.peer_id)
            .ok_or_else(|| CoreError::PeerNotFound(complete.peer_id))
    }

    /// Accept inbound pairing (HTTP handler).
    pub async fn accept_pair_offer(&self, accept: PairAccept) -> Result<PairComplete, CoreError> {
        self.assert_mesh_allowed()?;
        let pending = self
            .pending
            .get(&accept.pairing_id)
            .ok_or_else(|| CoreError::PairingFailed("unknown pairing_id".into()))?
            .clone();
        if Utc::now() > pending.expires_at {
            self.pending.remove(&accept.pairing_id);
            return Err(CoreError::PairingFailed("pairing expired".into()));
        }
        if pending.pin != accept.pin {
            return Err(CoreError::PairingFailed("PIN mismatch".into()));
        }
        parse_verifying_key(&accept.public_key_hex)
            .map_err(|e| CoreError::PairingFailed(e.to_string()))?;

        let local_pk = self.public_key_hex();
        let pin_secret = pin_shared_secret(&accept.pin, &local_pk, &accept.public_key_hex);
        {
            use hmac::{Hmac, Mac};
            use sha2::Sha256;
            type HmacSha256 = Hmac<Sha256>;
            let mut mac = HmacSha256::new_from_slice(&pin_secret)
                .map_err(|e| CoreError::Other(e.to_string()))?;
            mac.update(accept.public_key_hex.as_bytes());
            mac.update(local_pk.as_bytes());
            mac.update(
                &hex::decode(&accept.x25519_public_hex)
                    .map_err(|e| CoreError::PairingFailed(e.to_string()))?,
            );
            let expected = mac.finalize().into_bytes();
            let got = hex::decode(&accept.pin_mac)
                .map_err(|e| CoreError::PairingFailed(format!("pin_mac: {e}")))?;
            if got.as_slice() != expected.as_slice() {
                return Err(CoreError::PairingFailed("PIN MAC invalid".into()));
            }
        }

        let secret = self
            .x25519_secrets
            .get(&accept.pairing_id)
            .ok_or_else(|| CoreError::PairingFailed("missing ephemeral secret".into()))?;
        let dh = x25519_shared(secret.value(), &accept.x25519_public_hex)
            .map_err(|e| CoreError::PairingFailed(e.to_string()))?;
        drop(secret);
        let session = derive_session_secret(&dh, &pin_secret);

        let now = Utc::now();
        let fp = fingerprint_of_public_key(&accept.public_key_hex);
        let (local_node_id, local_pk, local_label, local_role, local_fp) = {
            let local = self.identity.read();
            (
                local.node_id.clone(),
                local.public_key_hex(),
                local.label.clone(),
                local.role,
                local.fingerprint(),
            )
        };
        let record = PeerKeyRecord {
            peer_id: accept.peer_id.clone(),
            public_key: accept.public_key_hex.clone(),
            label: Some(accept.label.clone()),
            trusted: true,
            created_at: now,
            address: None,
            port: None,
            fingerprint: Some(fp.clone()),
            source: Some(PeerSource::Manual.as_str().into()),
            role: Some(accept.role.clone()),
            capabilities_json: Some("{}".into()),
            last_seen_at: Some(now),
            paired_at: Some(now),
        };
        self.db
            .upsert_peer_key(&record, Some(&hex::encode(session)))
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        self.pending.remove(&accept.pairing_id);
        self.x25519_secrets.remove(&accept.pairing_id);
        self.connected
            .insert(accept.peer_id.clone(), PeerConnectionState::Connected);

        Ok(PairComplete {
            ok: true,
            peer_id: local_node_id,
            public_key_hex: local_pk,
            label: local_label,
            role: local_role.as_str().into(),
            session_secret_hex: hex::encode(session),
            fingerprint: local_fp,
        })
    }

    pub async fn reject_pairing(&self, pairing_id: Uuid) -> Result<(), CoreError> {
        self.pending.remove(&pairing_id);
        self.x25519_secrets.remove(&pairing_id);
        Ok(())
    }

    pub async fn set_trusted(&self, peer_id: &str, trusted: bool) -> Result<MeshPeerInfo, CoreError> {
        self.db
            .set_peer_trusted(peer_id, trusted)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))?;
        if !trusted {
            self.connected
                .insert(peer_id.to_string(), PeerConnectionState::Untrusted);
        }
        self.list_peers()
            .await?
            .into_iter()
            .find(|p| p.peer_id == peer_id)
            .ok_or_else(|| CoreError::PeerNotFound(peer_id.into()))
    }

    pub async fn get_peer(&self, peer_id: &str) -> Result<Option<PeerKeyRecord>, CoreError> {
        self.db
            .get_peer_key(peer_id)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))
    }

    pub async fn mark_connected(&self, peer_id: &str, connected: bool) {
        self.connected.insert(
            peer_id.to_string(),
            if connected {
                PeerConnectionState::Connected
            } else {
                PeerConnectionState::Offline
            },
        );
        let _ = self.db.touch_peer_seen(peer_id).await;
    }

    pub async fn update_peer_capabilities(
        &self,
        peer_id: &str,
        caps: &PeerCapabilities,
    ) -> Result<(), CoreError> {
        self.db
            .update_peer_capabilities(peer_id, caps)
            .await
            .map_err(|e| CoreError::Other(e.to_string()))
    }

    pub fn build_signed_envelope(&self, method: &str, params: Value) -> SignedEnvelope {
        let id = self.identity.read();
        let mut env = SignedEnvelope {
            from_node_id: id.node_id.clone(),
            from_public_key: id.public_key_hex(),
            timestamp_unix: Utc::now().timestamp(),
            nonce: Uuid::new_v4().to_string(),
            method: method.into(),
            params,
            signature_hex: String::new(),
        };
        let payload = env.signing_payload();
        env.signature_hex = id.sign(payload.as_bytes());
        env
    }

    pub async fn verify_envelope(&self, env: &SignedEnvelope) -> Result<(), CoreError> {
        self.assert_mesh_allowed()?;
        // Replay window: 5 minutes
        let now = Utc::now().timestamp();
        if (now - env.timestamp_unix).abs() > 300 {
            return Err(CoreError::PairingFailed("envelope timestamp out of window".into()));
        }
        let peer = self
            .get_peer(&env.from_node_id)
            .await?
            .ok_or_else(|| CoreError::PeerNotFound(env.from_node_id.clone()))?;
        if !peer.trusted {
            return Err(CoreError::PairingFailed("peer not trusted".into()));
        }
        if peer.public_key != env.from_public_key {
            return Err(CoreError::PairingFailed("public key mismatch".into()));
        }
        let payload = env.signing_payload();
        verify_signature(&env.from_public_key, payload.as_bytes(), &env.signature_hex)
            .map_err(|e| CoreError::PairingFailed(e.to_string()))?;
        Ok(())
    }

    async fn audit(&self, summary: &str, detail: Value) {
        let _ = self
            .db
            .append_audit(source_core::types::AuditEvent {
                id: Uuid::new_v4(),
                session_id: None,
                kind: AuditEventKind::Delegation,
                summary: summary.into(),
                detail,
                content_hash: String::new(),
                prev_hash: None,
                created_at: Utc::now(),
            })
            .await;
    }
}

trait IdentityMeta {
    fn clone_meta(&self) -> (String, String, String, PeerRole);
}

impl IdentityMeta for NodeIdentity {
    fn clone_meta(&self) -> (String, String, String, PeerRole) {
        (
            self.node_id.clone(),
            self.public_key_hex(),
            self.label.clone(),
            self.role,
        )
    }
}

fn local_ipv4() -> Option<std::net::Ipv4Addr> {
    // Best-effort: first non-loopback IPv4 from hostname resolution / interfaces via std.
    use std::net::UdpSocket;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() => Some(v4),
        _ => None,
    }
}

async fn resolve_host(host: &str) -> Result<IpAddr, CoreError> {
    if let Ok(ip) = host.parse::<IpAddr>() {
        return Ok(ip);
    }
    let host_owned = host.to_string();
    tokio::task::spawn_blocking(move || {
        use std::net::ToSocketAddrs;
        (format!("{host_owned}:0"))
            .to_socket_addrs()
            .map_err(|e| CoreError::InvalidArgument(format!("DNS resolve failed for {host_owned}: {e}")))?
            .next()
            .map(|a| a.ip())
            .ok_or_else(|| CoreError::InvalidArgument(format!("no addresses for {host_owned}")))
    })
    .await
    .map_err(|e| CoreError::Other(e.to_string()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use source_core::policy::NetworkTier;
    use tempfile::tempdir;

    struct MeshFixture {
        _dir: tempfile::TempDir,
        mesh: Arc<MeshService>,
    }

    async fn test_mesh() -> MeshFixture {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("t.db");
        let db = Db::connect(&db_path).await.unwrap();
        let mut policy = source_core::policy::NetworkPolicy::with_tier(NetworkTier::LocalAndLan);
        policy.require_approval_for_public = false;
        let engine = PolicyEngine::load(db.clone()).await.unwrap();
        engine.set_policy(policy).await.unwrap();
        let mesh = MeshService::new(db, engine, 7420, Some("test-node".into()))
            .await
            .unwrap();
        MeshFixture { _dir: dir, mesh }
    }

    #[tokio::test]
    async fn begin_and_confirm_pairing() {
        let fx = test_mesh().await;
        let session = fx.mesh.begin_pairing(None, Some(120)).await.unwrap();
        assert_eq!(session.pin.len(), 6);
        let peer_sk = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let peer_pk = hex::encode(peer_sk.verifying_key().as_bytes());
        let info = fx
            .mesh
            .confirm_pairing(
                session.pairing_id,
                "peer-abc",
                &peer_pk,
                &session.pin,
                Some("gpu-box".into()),
                Some("192.168.1.50".into()),
                Some(7420),
            )
            .await
            .unwrap();
        assert!(info.trusted);
        assert_eq!(info.peer_id, "peer-abc");
    }

    #[tokio::test]
    async fn mesh_blocked_on_local_only() {
        let dir = tempdir().unwrap();
        let db = Db::connect(&dir.path().join("t.db")).await.unwrap();
        let engine = PolicyEngine::load(db.clone()).await.unwrap();
        // default is LOCAL_ONLY
        let mesh = MeshService::new(db, engine, 7420, None).await.unwrap();
        let err = mesh
            .start(false, false, None, None)
            .await
            .err()
            .expect("should fail");
        assert!(matches!(err, CoreError::MeshDisabled(_)));
        let _keep = dir;
    }
}
