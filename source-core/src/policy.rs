use chrono::{DateTime, Utc};
use ipnet::IpNet;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use uuid::Uuid;

use crate::error::{CoreError, CoreResult};

/// Three-tier network policy enforced in the daemon HTTP/WS client and tool-runner gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum NetworkTier {
    /// Only 127.0.0.1 / ::1
    #[default]
    LocalOnly,
    /// RFC 1918 + Tailscale/WireGuard CGNAT 100.64.0.0/10
    LocalAndLan,
    /// Allowlist + approval hooks
    UnrestrictedConfigurable,
}

impl NetworkTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            NetworkTier::LocalOnly => "LOCAL_ONLY",
            NetworkTier::LocalAndLan => "LOCAL_AND_LAN",
            NetworkTier::UnrestrictedConfigurable => "UNRESTRICTED_CONFIGURABLE",
        }
    }
}

impl std::fmt::Display for NetworkTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl FromStr for NetworkTier {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_uppercase().as_str() {
            "LOCAL_ONLY" | "LOCALONLY" | "LOCAL" => Ok(Self::LocalOnly),
            "LOCAL_AND_LAN" | "LOCALANDLAN" | "LAN" => Ok(Self::LocalAndLan),
            "UNRESTRICTED_CONFIGURABLE" | "UNRESTRICTED" | "OPEN" => {
                Ok(Self::UnrestrictedConfigurable)
            }
            other => Err(CoreError::InvalidArgument(format!(
                "unknown network tier: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DestinationClass {
    Loopback,
    PrivateLan,
    Cgnat,
    Public,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NetworkDecision {
    Allow { class: DestinationClass },
    Deny { reason: String, class: DestinationClass },
    NeedsApproval { class: DestinationClass, host: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalCategory {
    FileEdits,
    Shell,
    Network,
    Mcp,
    CrossNode,
}

impl ApprovalCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FileEdits => "file_edits",
            Self::Shell => "shell",
            Self::Network => "network",
            Self::Mcp => "mcp",
            Self::CrossNode => "cross_node",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Denied,
    Deferred,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ApprovalRequest {
    pub id: Uuid,
    pub session_id: Option<Uuid>,
    pub category: ApprovalCategory,
    pub summary: String,
    pub detail: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolPermission {
    pub tool_name: String,
    pub categories: Vec<ApprovalCategory>,
    pub auto_approve_in_act: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PeerKeyRecord {
    pub peer_id: String,
    pub public_key: String,
    pub label: Option<String>,
    pub trusted: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NetworkPolicy {
    pub tier: NetworkTier,
    /// Hosts/CIDRs allowed when tier is UNRESTRICTED_CONFIGURABLE.
    pub allowlist: Vec<String>,
    /// Hosts always denied regardless of tier (except loopback in LOCAL_ONLY checks still apply).
    pub denylist: Vec<String>,
    /// When true in UNRESTRICTED_CONFIGURABLE, public destinations require approval.
    pub require_approval_for_public: bool,
}

impl Default for NetworkPolicy {
    fn default() -> Self {
        Self {
            tier: NetworkTier::LocalOnly,
            allowlist: Vec::new(),
            denylist: Vec::new(),
            require_approval_for_public: true,
        }
    }
}

impl NetworkPolicy {
    pub fn with_tier(tier: NetworkTier) -> Self {
        Self {
            tier,
            ..Default::default()
        }
    }

    pub fn classify_ip(ip: IpAddr) -> DestinationClass {
        if ip.is_loopback() {
            return DestinationClass::Loopback;
        }
        match ip {
            IpAddr::V4(v4) => {
                let octets = v4.octets();
                // RFC 1918
                if octets[0] == 10
                    || (octets[0] == 172 && (16..=31).contains(&octets[1]))
                    || (octets[0] == 192 && octets[1] == 168)
                {
                    return DestinationClass::PrivateLan;
                }
                // CGNAT / Tailscale / WireGuard shared space 100.64.0.0/10
                if octets[0] == 100 && (64..=127).contains(&octets[1]) {
                    return DestinationClass::Cgnat;
                }
                // link-local
                if octets[0] == 169 && octets[1] == 254 {
                    return DestinationClass::PrivateLan;
                }
                DestinationClass::Public
            }
            IpAddr::V6(v6) => {
                if (v6.segments()[0] & 0xfe00) == 0xfc00 {
                    // Unique local addresses fc00::/7
                    DestinationClass::PrivateLan
                } else if v6.segments()[0] & 0xffc0 == 0xfe80 {
                    DestinationClass::PrivateLan
                } else {
                    DestinationClass::Public
                }
            }
        }
    }

    fn host_matches_pattern(host: &str, pattern: &str) -> bool {
        let host = host.trim().to_ascii_lowercase();
        let pattern = pattern.trim().to_ascii_lowercase();
        if pattern.is_empty() {
            return false;
        }
        if let Ok(net) = pattern.parse::<IpNet>() {
            if let Ok(ip) = host.parse::<IpAddr>() {
                return net.contains(&ip);
            }
            return false;
        }
        if let Some(suffix) = pattern.strip_prefix("*.") {
            return host == suffix || host.ends_with(&format!(".{suffix}"));
        }
        host == pattern
    }

    fn in_list(&self, host: &str, list: &[String]) -> bool {
        list.iter().any(|p| Self::host_matches_pattern(host, p))
    }

    /// Evaluate a destination host/IP. DNS is caller's responsibility; pass resolved IP when known.
    pub fn evaluate(&self, host: &str, ip: Option<IpAddr>) -> NetworkDecision {
        let host_trim = host.trim();
        if host_trim.is_empty() {
            return NetworkDecision::Deny {
                reason: "empty host".into(),
                class: DestinationClass::Unknown,
            };
        }

        if self.in_list(host_trim, &self.denylist) {
            return NetworkDecision::Deny {
                reason: format!("host `{host_trim}` is on denylist"),
                class: ip.map(Self::classify_ip).unwrap_or(DestinationClass::Unknown),
            };
        }

        // Prefer explicit IP classification when provided; also try parsing host as IP.
        let resolved = ip.or_else(|| host_trim.parse::<IpAddr>().ok());
        let class = resolved
            .map(Self::classify_ip)
            .unwrap_or(DestinationClass::Unknown);

        match self.tier {
            NetworkTier::LocalOnly => {
                if class == DestinationClass::Loopback
                    || host_trim.eq_ignore_ascii_case("localhost")
                {
                    NetworkDecision::Allow { class: DestinationClass::Loopback }
                } else {
                    NetworkDecision::Deny {
                        reason: format!(
                            "LOCAL_ONLY allows only loopback; refused `{host_trim}` ({class:?})"
                        ),
                        class,
                    }
                }
            }
            NetworkTier::LocalAndLan => match class {
                DestinationClass::Loopback
                | DestinationClass::PrivateLan
                | DestinationClass::Cgnat => NetworkDecision::Allow { class },
                DestinationClass::Public | DestinationClass::Unknown => {
                    if host_trim.eq_ignore_ascii_case("localhost") {
                        NetworkDecision::Allow {
                            class: DestinationClass::Loopback,
                        }
                    } else {
                        NetworkDecision::Deny {
                            reason: format!(
                                "LOCAL_AND_LAN refuses public/unknown host `{host_trim}`"
                            ),
                            class,
                        }
                    }
                }
            },
            NetworkTier::UnrestrictedConfigurable => {
                if self.in_list(host_trim, &self.allowlist) {
                    return NetworkDecision::Allow { class };
                }
                match class {
                    DestinationClass::Loopback
                    | DestinationClass::PrivateLan
                    | DestinationClass::Cgnat => NetworkDecision::Allow { class },
                    DestinationClass::Public | DestinationClass::Unknown => {
                        if self.require_approval_for_public {
                            NetworkDecision::NeedsApproval {
                                class,
                                host: host_trim.to_string(),
                            }
                        } else if self.allowlist.is_empty() {
                            // Empty allowlist with approval disabled = open
                            NetworkDecision::Allow { class }
                        } else {
                            NetworkDecision::Deny {
                                reason: format!(
                                    "`{host_trim}` not on allowlist for UNRESTRICTED_CONFIGURABLE"
                                ),
                                class,
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn evaluate_socket(&self, addr: SocketAddr) -> NetworkDecision {
        self.evaluate(&addr.ip().to_string(), Some(addr.ip()))
    }

    pub fn evaluate_url(&self, raw: &str) -> CoreResult<NetworkDecision> {
        let parsed = url::Url::parse(raw)
            .map_err(|e| CoreError::InvalidArgument(format!("invalid url `{raw}`: {e}")))?;
        let host = parsed
            .host_str()
            .ok_or_else(|| CoreError::InvalidArgument(format!("url missing host: {raw}")))?;
        Ok(self.evaluate(host, host.parse::<IpAddr>().ok()))
    }

    pub fn assert_allowed(&self, host: &str, ip: Option<IpAddr>) -> CoreResult<()> {
        match self.evaluate(host, ip) {
            NetworkDecision::Allow { .. } => Ok(()),
            NetworkDecision::Deny { reason, .. } => Err(CoreError::NetworkBlocked(reason)),
            NetworkDecision::NeedsApproval { host, .. } => {
                Err(CoreError::ApprovalRequired(format!("network:{host}")))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn local_only_allows_loopback() {
        let p = NetworkPolicy::with_tier(NetworkTier::LocalOnly);
        assert!(matches!(
            p.evaluate("127.0.0.1", Some(IpAddr::V4(Ipv4Addr::LOCALHOST))),
            NetworkDecision::Allow { .. }
        ));
        assert!(matches!(
            p.evaluate("192.168.1.1", Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)))),
            NetworkDecision::Deny { .. }
        ));
    }

    #[test]
    fn lan_allows_rfc1918_and_tailscale() {
        let p = NetworkPolicy::with_tier(NetworkTier::LocalAndLan);
        assert!(matches!(
            p.evaluate("10.0.0.5", Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)))),
            NetworkDecision::Allow {
                class: DestinationClass::PrivateLan
            }
        ));
        assert!(matches!(
            p.evaluate("100.64.1.2", Some(IpAddr::V4(Ipv4Addr::new(100, 64, 1, 2)))),
            NetworkDecision::Allow {
                class: DestinationClass::Cgnat
            }
        ));
        assert!(matches!(
            p.evaluate("8.8.8.8", Some(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)))),
            NetworkDecision::Deny { .. }
        ));
    }

    #[test]
    fn unrestricted_needs_approval_for_public() {
        let p = NetworkPolicy::with_tier(NetworkTier::UnrestrictedConfigurable);
        assert!(matches!(
            p.evaluate("api.openai.com", None),
            NetworkDecision::NeedsApproval { .. }
        ));
    }

    #[test]
    fn allowlist_bypasses_approval() {
        let mut p = NetworkPolicy::with_tier(NetworkTier::UnrestrictedConfigurable);
        p.allowlist.push("api.openai.com".into());
        assert!(matches!(
            p.evaluate("api.openai.com", None),
            NetworkDecision::Allow { .. }
        ));
    }
}