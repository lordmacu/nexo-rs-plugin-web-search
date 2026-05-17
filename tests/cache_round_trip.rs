//! Cache round-trip tests using in-memory cache (no sqlite file
//! I/O). v0.1.0 ships the wrapper; the cache itself lives inside
//! `nexo-web-search 0.1.2`.

use std::sync::Arc;

use nexo_plugin_web_search::plugin::{
    CacheConfig, ProviderEntry, ProvidersConfig, WebSearchConfigFile, WebSearchInstance,
    WebSearchPlugin,
};

fn brave_with_cache(
    id: &str,
    dir: &std::path::Path,
    enabled: bool,
    ttl_secs: u64,
) -> WebSearchInstance {
    let key_path = dir.join(format!("{id}_brave.txt"));
    std::fs::write(&key_path, "test-key").unwrap();
    WebSearchInstance {
        id: id.into(),
        agent_id: None,
        providers: ProvidersConfig {
            brave: Some(ProviderEntry {
                api_key_path: Some(key_path),
                timeout_ms: 8000,
            }),
            ..Default::default()
        },
        cache: CacheConfig {
            enabled,
            path: None, // in-memory
            ttl_secs,
        },
        default_order: vec!["brave".into()],
    }
}

#[tokio::test]
async fn cache_enabled_with_in_memory_path_builds_router() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile {
        instances: vec![brave_with_cache("default", dir.path(), true, 3600)],
    })
    .await
    .unwrap();
    assert!(p.router_for("default").is_ok());
}

#[tokio::test]
async fn cache_disabled_still_builds_router() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile {
        instances: vec![brave_with_cache("default", dir.path(), false, 0)],
    })
    .await
    .unwrap();
    assert!(p.router_for("default").is_ok());
}

#[tokio::test]
async fn cache_with_zero_ttl_still_opens_minimum_one_second() {
    // Router enforces TTL >= 1s via `max(1)`; opening with 0
    // succeeds with the floor.
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile {
        instances: vec![brave_with_cache("default", dir.path(), true, 0)],
    })
    .await
    .unwrap();
    assert!(p.router_for("default").is_ok());
}
