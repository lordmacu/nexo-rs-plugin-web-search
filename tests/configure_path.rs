//! plugin.configure round-trip — multi-instance × multi-agent.

use std::path::Path;
use std::sync::Arc;

use nexo_plugin_web_search::plugin::{
    CacheConfig, ProviderEntry, ProvidersConfig, WebSearchConfigFile, WebSearchInstance, WebSearchPlugin,
};

fn brave_only(id: &str, agent: Option<&str>, dir: &Path) -> WebSearchInstance {
    let key_path = dir.join(format!("{id}_brave.txt"));
    std::fs::write(&key_path, "test-brave-key").unwrap();
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

#[tokio::test]
async fn empty_instances_yields_empty_state() {
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile::default()).await.unwrap();
    assert_eq!(p.instance_count(), 0);
    assert_eq!(p.agent_count(), 0);
    assert_eq!(p.shared_count(), 0);
}

#[tokio::test]
async fn single_shared_instance() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile {
        instances: vec![brave_only("default", None, dir.path())],
    })
    .await
    .unwrap();
    assert_eq!(p.instance_count(), 1);
    assert_eq!(p.shared_count(), 1);
    assert!(p.router_for("default").is_ok());
}

#[tokio::test]
async fn single_private_instance() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile {
        instances: vec![brave_only("ana", Some("ana"), dir.path())],
    })
    .await
    .unwrap();
    assert_eq!(p.instance_count(), 1);
    assert_eq!(p.agent_count(), 1);
    assert_eq!(p.shared_count(), 0);
    let listing = p.instances_for_agent("ana");
    assert_eq!(listing, vec!["ana"]);
}

#[tokio::test]
async fn multi_private_one_agent() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile {
        instances: vec![
            brave_only("research", Some("ana"), dir.path()),
            brave_only("news", Some("ana"), dir.path()),
        ],
    })
    .await
    .unwrap();
    assert_eq!(p.instance_count(), 2);
    let listing = p.instances_for_agent("ana");
    assert_eq!(listing, vec!["research", "news"]);
    assert_eq!(
        p.resolve_instance(&serde_json::json!({}), "ana").unwrap(),
        "research",
        "default = first declared"
    );
}

#[tokio::test]
async fn mixed_shared_and_private() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile {
        instances: vec![
            brave_only("default", None, dir.path()),
            brave_only("ana_research", Some("ana"), dir.path()),
        ],
    })
    .await
    .unwrap();
    assert_eq!(p.shared_count(), 1);
    assert_eq!(p.agent_count(), 1);
    // ana → private wins.
    assert_eq!(
        p.resolve_instance(&serde_json::json!({}), "ana").unwrap(),
        "ana_research"
    );
    // bob (no private) → shared.
    assert_eq!(
        p.resolve_instance(&serde_json::json!({}), "bob").unwrap(),
        "default"
    );
}

#[tokio::test]
async fn full_replace_drops_old_state() {
    let dir = tempfile::tempdir().unwrap();
    let p = Arc::new(WebSearchPlugin::new());
    p.on_configure(WebSearchConfigFile {
        instances: vec![
            brave_only("a", Some("ana"), dir.path()),
            brave_only("b", Some("bob"), dir.path()),
            brave_only("c", None, dir.path()),
        ],
    })
    .await
    .unwrap();
    assert_eq!(p.instance_count(), 3);
    p.on_configure(WebSearchConfigFile {
        instances: vec![brave_only("solo", None, dir.path())],
    })
    .await
    .unwrap();
    assert_eq!(p.instance_count(), 1);
    assert_eq!(p.shared_count(), 1);
    assert_eq!(p.agent_count(), 0);
    assert!(p.router_for("a").is_err());
    assert!(p.router_for("solo").is_ok());
}
