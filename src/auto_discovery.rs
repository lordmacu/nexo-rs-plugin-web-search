//! Phase 81.33.b.real Stage 4 + Stage 5 — auto-discovery broker
//! handlers for admin RPC + Prometheus metrics scrape.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::plugin::WebSearchPlugin;
use crate::runtime_handle;

async fn current_plugin() -> Option<Arc<WebSearchPlugin>> {
    runtime_handle::runtime_handle()
        .read()
        .await
        .as_ref()
        .map(Arc::clone)
}

pub async fn admin_handle(request: &Value) -> Value {
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    let Some(plugin) = current_plugin().await else {
        return json!({
            "ok": false,
            "error": "web_search plugin not yet booted",
        });
    };

    match method {
        "nexo/admin/web_search/bot_info" => json!({
            "ok": true,
            "result": {
                "plugin": "web_search",
                "version": env!("CARGO_PKG_VERSION"),
                "configured_instances": plugin.instance_count(),
                "configured_agents": plugin.agent_count(),
                "shared_instances": plugin.shared_count(),
            },
        }),

        "nexo/admin/web_search/cache_stats" => {
            let instance = params.get("instance").and_then(|v| v.as_str());
            match plugin.admin_cache_stats(instance).await {
                Ok(v) => json!({ "ok": true, "result": v }),
                Err(e) => json!({ "ok": false, "error": format!("{e}") }),
            }
        }

        "nexo/admin/web_search/cache_clear" => {
            let instance = params.get("instance").and_then(|v| v.as_str());
            match plugin.admin_cache_clear(instance).await {
                Ok(v) => json!({ "ok": true, "result": v }),
                Err(e) => json!({ "ok": false, "error": format!("{e}") }),
            }
        }

        "nexo/admin/web_search/provider_status" => match plugin.admin_provider_status().await {
            Ok(v) => json!({ "ok": true, "result": v }),
            Err(e) => json!({ "ok": false, "error": format!("{e}") }),
        },

        "nexo/admin/web_search/list_instances" => match plugin.admin_list_instances().await {
            Ok(v) => json!({ "ok": true, "result": v }),
            Err(e) => json!({ "ok": false, "error": format!("{e}") }),
        },

        other => json!({
            "ok": false,
            "error": format!("unknown admin method: {other}"),
        }),
    }
}

/// `plugin.web_search.metrics.scrape` broker handler. Delegates to
/// `nexo_web_search::telemetry::render` which emits Prometheus
/// exposition format. The daemon's `/metrics` aggregator appends.
pub async fn metrics_scrape(_request: &Value) -> Value {
    let mut out = String::new();
    nexo_web_search::telemetry::render(&mut out);
    json!({ "ok": true, "body": out })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{
        CacheConfig, ProviderEntry, ProvidersConfig, WebSearchConfigFile, WebSearchInstance,
    };
    use serial_test::serial;

    async fn boot_one_shared() -> Arc<WebSearchPlugin> {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("brave.txt");
        std::fs::write(&key_path, "test-key").unwrap();
        let p = Arc::new(WebSearchPlugin::new());
        p.on_configure(WebSearchConfigFile {
            instances: vec![WebSearchInstance {
                id: "default".into(),
                agent_id: None,
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
            }],
        })
        .await
        .unwrap();
        std::mem::forget(dir);
        runtime_handle::set_runtime_handle(p.clone()).await;
        p
    }

    #[tokio::test]
    #[serial]
    async fn admin_bot_info_returns_metadata() {
        let _p = boot_one_shared().await;
        let r = admin_handle(&json!({
            "method": "nexo/admin/web_search/bot_info",
            "params": {},
        }))
        .await;
        assert_eq!(r["ok"], json!(true));
        assert_eq!(r["result"]["plugin"], json!("web_search"));
        assert_eq!(r["result"]["configured_instances"], json!(1));
        assert_eq!(r["result"]["shared_instances"], json!(1));
    }

    #[tokio::test]
    #[serial]
    async fn admin_list_instances_returns_full_map() {
        let _p = boot_one_shared().await;
        let r = admin_handle(&json!({
            "method": "nexo/admin/web_search/list_instances",
            "params": {},
        }))
        .await;
        assert_eq!(r["ok"], json!(true));
        let result = &r["result"];
        assert_eq!(result["instances"].as_array().unwrap()[0], json!("default"));
        assert_eq!(result["shared"].as_array().unwrap()[0], json!("default"));
    }

    #[tokio::test]
    #[serial]
    async fn admin_unknown_method_returns_err() {
        let _p = boot_one_shared().await;
        let r = admin_handle(&json!({
            "method": "nexo/admin/web_search/does_not_exist",
        }))
        .await;
        assert_eq!(r["ok"], json!(false));
    }

    #[tokio::test]
    #[serial]
    async fn metrics_scrape_returns_prometheus_body() {
        let _p = boot_one_shared().await;
        let r = metrics_scrape(&json!({})).await;
        assert_eq!(r["ok"], json!(true));
        // Body is empty when no calls happened yet, but the field
        // must exist and be a string.
        assert!(r["body"].is_string());
    }
}
