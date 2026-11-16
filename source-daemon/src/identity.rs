//! Local mesh node identity: Ed25519 keypair persisted in SQLite `node_identity`.

use anyhow::{anyhow, Context, Result};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use source_core::mesh::PeerRole;
use std::str::FromStr;
use zeroize::Zeroize;

use crate::db::Db;

#[derive(Clone)]
pub struct NodeIdentity {
    pub node_id: String,
    pub label: String,
    pub role: PeerRole,
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

impl NodeIdentity {
    pub async fn load_or_create(db: &Db, default_label: Option<String>) -> Result<Self> {
        if let Some(row) = db.load_node_identity().await? {
            let sk_bytes = hex::decode(&row.private_key_hex)
                .context("decode node private key")?;
            if sk_bytes.len() != 32 {
                return Err(anyhow!("node private key must be 32 bytes"));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&sk_bytes);
            let signing_key = SigningKey::from_bytes(&arr);
            arr.zeroize();
            let verifying_key = signing_key.verifying_key();
            let pub_hex = hex::encode(verifying_key.as_bytes());
            if pub_hex != row.public_key_hex {
                return Err(anyhow!("node identity public key mismatch"));
            }
            return Ok(Self {
                node_id: row.node_id,
                label: row.label,
                role: PeerRole::from_str(&row.role).unwrap_or(PeerRole::Hybrid),
                signing_key,
                verifying_key,
            });
        }

        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let node_id = uuid::Uuid::new_v4().to_string();
        let label = default_label.unwrap_or_else(|| {
            hostname::get()
                .ok()
                .and_then(|h| h.into_string().ok())
                .unwrap_or_else(|| "source-node".into())
        });
        let role = PeerRole::Hybrid;
        let private_key_hex = hex::encode(signing_key.to_bytes());
        let public_key_hex = hex::encode(verifying_key.as_bytes());
        db.save_node_identity(
            &node_id,
            &public_key_hex,
            &private_key_hex,
            &label,
            role.as_str(),
        )
        .await?;
        Ok(Self {
            node_id,
            label,
            role,
            signing_key,
            verifying_key,
        })
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.verifying_key.as_bytes())
    }

    pub fn fingerprint(&self) -> String {
        fingerprint_of_public_key(&self.public_key_hex())
    }

    pub fn verifying_key(&self) -> &VerifyingKey {
        &self.verifying_key
    }

    pub fn sign(&self, message: &[u8]) -> String {
        let sig = self.signing_key.sign(message);
        hex::encode(sig.to_bytes())
    }

    pub fn set_label(&mut self, label: String) {
        self.label = label;
    }

    pub fn set_role(&mut self, role: PeerRole) {
        self.role = role;
    }

    pub async fn persist_meta(&self, db: &Db) -> Result<()> {
        db.update_node_identity_meta(&self.node_id, &self.label, self.role.as_str())
            .await
    }
}

pub fn fingerprint_of_public_key(public_key_hex: &str) -> String {
    let digest = Sha256::digest(public_key_hex.as_bytes());
    hex::encode(&digest[..8])
}

pub fn parse_verifying_key(public_key_hex: &str) -> Result<VerifyingKey> {
    let bytes = hex::decode(public_key_hex.trim()).context("decode peer public key")?;
    if bytes.len() != 32 {
        return Err(anyhow!("ed25519 public key must be 32 bytes"));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    VerifyingKey::from_bytes(&arr).map_err(|e| anyhow!("invalid ed25519 public key: {e}"))
}

pub fn verify_signature(public_key_hex: &str, message: &[u8], signature_hex: &str) -> Result<()> {
    let vk = parse_verifying_key(public_key_hex)?;
    let sig_bytes = hex::decode(signature_hex.trim()).context("decode signature")?;
    if sig_bytes.len() != 64 {
        return Err(anyhow!("ed25519 signature must be 64 bytes"));
    }
    let mut arr = [0u8; 64];
    arr.copy_from_slice(&sig_bytes);
    let sig = Signature::from_bytes(&arr);
    vk.verify(message, &sig)
        .map_err(|e| anyhow!("signature verification failed: {e}"))
}

pub fn generate_pin() -> String {
    use rand::Rng;
    let n: u32 = rand::thread_rng().gen_range(0..1_000_000);
    format!("{n:06}")
}

/// Derive a 32-byte shared secret from PIN + both public keys (order-independent).
pub fn pin_shared_secret(pin: &str, local_pk: &str, remote_pk: &str) -> [u8; 32] {
    use hkdf::Hkdf;
    use sha2::Sha256;
    let (a, b) = if local_pk <= remote_pk {
        (local_pk, remote_pk)
    } else {
        (remote_pk, local_pk)
    };
    let ikm = format!("source-mesh-pin-v1|{pin}|{a}|{b}");
    let hk = Hkdf::<Sha256>::new(Some(b"source-mesh"), ikm.as_bytes());
    let mut out = [0u8; 32];
    hk.expand(b"pairing", &mut out).expect("hkdf expand");
    out
}

/// X25519 ephemeral DH for pairing, authenticated with PIN-derived secret.
pub fn x25519_ephemeral_keypair() -> (x25519_dalek::StaticSecret, x25519_dalek::PublicKey) {
    let secret = x25519_dalek::StaticSecret::random_from_rng(OsRng);
    let public = x25519_dalek::PublicKey::from(&secret);
    (secret, public)
}

pub fn x25519_shared(
    secret: &x25519_dalek::StaticSecret,
    peer_public_hex: &str,
) -> Result<[u8; 32]> {
    let bytes = hex::decode(peer_public_hex.trim()).context("decode x25519 public")?;
    if bytes.len() != 32 {
        return Err(anyhow!("x25519 public key must be 32 bytes"));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&bytes);
    let peer = x25519_dalek::PublicKey::from(arr);
    Ok(secret.diffie_hellman(&peer).to_bytes())
}

pub fn derive_session_secret(dh: &[u8; 32], pin_secret: &[u8; 32]) -> [u8; 32] {
    use hkdf::Hkdf;
    use sha2::Sha256;
    let hk = Hkdf::<Sha256>::new(Some(pin_secret), dh);
    let mut out = [0u8; 32];
    hk.expand(b"source-mesh-session-v1", &mut out)
        .expect("hkdf expand");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_is_six_digits() {
        let pin = generate_pin();
        assert_eq!(pin.len(), 6);
        assert!(pin.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn pin_secret_order_independent() {
        let a = pin_shared_secret("123456", "aa", "bb");
        let b = pin_shared_secret("123456", "bb", "aa");
        assert_eq!(a, b);
    }

    #[test]
    fn sign_verify_roundtrip() {
        let sk = SigningKey::generate(&mut OsRng);
        let pk_hex = hex::encode(sk.verifying_key().as_bytes());
        let msg = b"hello-mesh";
        let sig = hex::encode(sk.sign(msg).to_bytes());
        verify_signature(&pk_hex, msg, &sig).unwrap();
    }
}
