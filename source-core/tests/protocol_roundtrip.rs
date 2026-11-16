use source_core::protocol::{JsonRpcId, JsonRpcRequest, JsonRpcResponse, Method, PROTOCOL_VERSION};
use source_core::policy::NetworkTier;
use serde_json::json;

#[test]
fn method_names_roundtrip() {
    for m in [
        Method::Status,
        Method::SetNetworkTier,
        Method::Complete,
        Method::CreateCheckpoint,
        Method::InvokeTool,
        Method::MeshStart,
        Method::ClusterResources,
        Method::LlmCompleteRemote,
    ] {
        assert_eq!(Method::parse(m.as_str()), Some(m));
    }
    assert_eq!(PROTOCOL_VERSION, "1.2.0");
}

#[test]
fn jsonrpc_shapes() {
    let req = JsonRpcRequest::new(1, Method::GetNetworkTier.as_str(), json!({}));
    let s = serde_json::to_string(&req).unwrap();
    let back: JsonRpcRequest = serde_json::from_str(&s).unwrap();
    assert_eq!(back.method, "source/network/getTier");

    let resp = JsonRpcResponse::success(
        JsonRpcId::Number(1),
        json!({ "tier": NetworkTier::LocalOnly }),
    );
    let s = serde_json::to_string(&resp).unwrap();
    assert!(s.contains("LOCAL_ONLY") || s.contains("local_only") || s.contains("LocalOnly"));
}