use source_core::policy::{NetworkDecision, NetworkPolicy, NetworkTier};
use source_daemon::db::Db;
use source_daemon::policy::PolicyEngine;
use std::net::{IpAddr, Ipv4Addr};
use tempfile::tempdir;

#[tokio::test]
async fn policy_persists_and_intercepts() {
    let dir = tempdir().unwrap();
    let db = Db::connect(&dir.path().join("t.db")).await.unwrap();
    let engine = PolicyEngine::load(db).await.unwrap();
    assert_eq!(engine.tier(), NetworkTier::LocalOnly);

    let mut policy = NetworkPolicy::with_tier(NetworkTier::LocalAndLan);
    engine.set_policy(policy.clone()).await.unwrap();
    assert_eq!(engine.tier(), NetworkTier::LocalAndLan);

    let allow = engine
        .intercept(
            "10.0.0.2",
            Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))),
            None,
            "test",
        )
        .await
        .unwrap();
    assert!(matches!(allow, NetworkDecision::Allow { .. }));

    let blocked = engine
        .intercept(
            "1.1.1.1",
            Some(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))),
            None,
            "test",
        )
        .await;
    assert!(blocked.is_err());

    policy.tier = NetworkTier::UnrestrictedConfigurable;
    policy.allowlist.push("api.example.com".into());
    engine.set_policy(policy).await.unwrap();
    let ok = engine
        .intercept("api.example.com", None, None, "test")
        .await
        .unwrap();
    assert!(matches!(ok, NetworkDecision::Allow { .. }));
}