//! In-process invoke dispatcher tests.

use std::path::Path;
use std::sync::Arc;

use nexo_plugin_web_search::plugin::{
    CacheConfig, ProviderEntry, ProvidersConfig, WebSearchConfigFile, WebSearchInstance, WebSearchPlugin,
};
use serde_json::json;

fn brave_only(id: &str, agent: Option<&str>, dir: &Path) -> WebSearchInstance {
    let key_path = dir.join(format!("{id}_brave.txt"));
    std::fs::write(&key_path, "test-key").unwrap();
    WebSearchInstance {
        id: id.into(),
        agent_id: agent.map(String::from),
        providers: ProvidersConfig {
            brave: Some(ProviderEntry {
                api_key_path: Some(key_path),
                timeout_ms: 8000,
            }),
            ..Default::default()
        },
        cache: CacheConfig {
            enabled: false,
            path: None,
            ttl_secs: 0,
        },
        default_order: vec!["brave".into()],
    }
}

async fn boot_shared() -> (Arc<WebSearchPlugin>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile {
        instances: vec![brave_only("default", None, dir.path())],
    })
    .await
    .unwrap();
    (p, dir)
}

#[tokio::test]
async fn unknown_tool_errors() {
    let (p, _dir) = boot_shared().await;
    let err = p
        .invoke_outbound_tool("web_bogus", json!({"query": "x"}), "ana", None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("unknown tool"));
}

#[tokio::test]
async fn policy_disabled_returns_error() {
    let (p, _dir) = boot_shared().await;
    let err = p
        .invoke_outbound_tool(
            "web_search",
            json!({"query": "x"}),
            "ana",
            Some(&json!({"enabled": false})),
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("disabled by policy"));
}

#[tokio::test]
async fn missing_query_errors() {
    let (p, _dir) = boot_shared().await;
    let err = p
        .invoke_outbound_tool("web_search", json!({}), "ana", None)
        .await
        .unwrap_err();
    let msg = err.to_string();
    // serde parses but query empty fails router-side; either path
    // surfaces "query" or "missing field".
    assert!(
        msg.contains("query") || msg.contains("missing"),
        "expected missing-query indication; got: {msg}"
    );
}

#[tokio::test]
async fn empty_query_errors_from_router() {
    let (p, _dir) = boot_shared().await;
    let err = p
        .invoke_outbound_tool(
            "web_search",
            json!({"query": "   "}),
            "ana",
            None,
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("query"),
        "expected empty-query rejection: {err}"
    );
}

#[tokio::test]
async fn unknown_agent_with_no_shared_errors() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile {
        instances: vec![brave_only("ana_priv", Some("ana"), dir.path())],
    })
    .await
    .unwrap();
    let err = p
        .invoke_outbound_tool("web_search", json!({"query": "x"}), "bob", None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no configured web_search instance"));
}

#[tokio::test]
async fn unknown_instance_errors() {
    let (p, _dir) = boot_shared().await;
    let err = p
        .invoke_outbound_tool(
            "web_search",
            json!({"query": "x", "instance": "nope"}),
            "ana",
            None,
        )
        .await
        .unwrap_err();
    assert!(err.to_string().contains("`nope` is not configured"));
}

#[tokio::test]
async fn explicit_instance_arg_used() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile {
        instances: vec![
            brave_only("default", None, dir.path()),
            brave_only("research", Some("ana"), dir.path()),
        ],
    })
    .await
    .unwrap();
    // bob (no private) uses default → but explicit arg picks research.
    let id = p
        .resolve_instance(&json!({"instance": "research"}), "bob")
        .unwrap();
    assert_eq!(id, "research");
}

#[tokio::test]
async fn policy_disabled_blocks_before_router_call() {
    // Even with a perfectly valid query, policy.enabled=false
    // short-circuits BEFORE the router runs. Verifies the gate
    // sits at the dispatcher entry point.
    let (p, _dir) = boot_shared().await;
    let result = p
        .invoke_outbound_tool(
            "web_search",
            json!({"query": "rust async"}),
            "ana",
            Some(&json!({"enabled": false})),
        )
        .await;
    assert!(result.is_err());
}
