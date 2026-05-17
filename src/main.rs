//! Subprocess + CLI entrypoint for `nexo-plugin-web-search`
//! (Phase 95).

use std::sync::Arc;

use clap::Parser;
use nexo_broker::AnyBroker;
use nexo_microapp_sdk::plugin::{PluginAdapter, ToolInvocation, ToolInvocationError};
use serde_json::Value;

use nexo_plugin_web_search::auto_discovery;
use nexo_plugin_web_search::cli::{Cli, Command};
use nexo_plugin_web_search::env_config::web_search_config_from_env;
use nexo_plugin_web_search::plugin::{WebSearchConfigFile, WebSearchPlugin};
use nexo_plugin_web_search::runtime_handle;
use nexo_plugin_web_search::tools::tool_defs;

const MANIFEST: &str = include_str!("../nexo-plugin.toml");

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    // Phase 81.20.x F1 — Stage 8: short-circuit `--print-manifest`
    // BEFORE tracing + broker init so discovery walker just gets
    // manifest bytes on stdout.
    nexo_microapp_sdk::plugin::print_manifest_if_requested(MANIFEST);

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let _ = rustls::crypto::ring::default_provider().install_default();

    let cli = Cli::parse();
    match cli.command {
        Some(Command::PrintManifest) => {
            print!("{}", MANIFEST);
            return Ok(());
        }
        None => {}
    }

    let plugin = Arc::new(WebSearchPlugin::new());
    runtime_handle::set_runtime_handle(plugin.clone()).await;

    // Phase 95 — eager-load `$NEXO_CONFIG_DIR/plugins/web-search.yaml`
    // so the plugin is functional even when the daemon's
    // `plugin.configure` JSON-RPC races the first tool call.
    match web_search_config_from_env() {
        Ok(env_cfg) => {
            if let Some(initial) = env_cfg.initial {
                if let Err(e) = plugin.on_configure(initial).await {
                    tracing::warn!(
                        target = "nexo_plugin_web_search",
                        error = %e,
                        "env-based on_configure failed"
                    );
                } else if let Some(path) = env_cfg.config_path {
                    tracing::info!(
                        target = "nexo_plugin_web_search",
                        path = %path.display(),
                        "loaded initial web-search.yaml"
                    );
                }
            }
        }
        Err(e) => {
            tracing::warn!(
                target = "nexo_plugin_web_search",
                error = %e,
                "env config load failed; relying on plugin.configure"
            );
        }
    }

    if let Ok(broker_url) = std::env::var("NEXO_BROKER_URL") {
        if !broker_url.is_empty() {
            match boot_broker(&broker_url).await {
                Ok(broker) => spawn_auto_discovery_subscribers(broker),
                Err(e) => tracing::warn!(
                    target = "nexo_plugin_web_search",
                    error = %e,
                    "broker connect failed; admin RPCs disabled"
                ),
            }
        }
    }

    let plugin_for_configure = plugin.clone();
    let plugin_for_tool = plugin.clone();

    let adapter = PluginAdapter::new(MANIFEST)?
        .declare_tools(tool_defs())
        .on_configure(move |value: serde_yaml::Value| {
            let plugin = plugin_for_configure.clone();
            async move {
                let file: WebSearchConfigFile = serde_yaml::from_value(value)
                    .map_err(|e| format!("invalid web-search.yaml: {e}"))?;
                plugin
                    .on_configure(file)
                    .await
                    .map_err(|e| format!("on_configure: {e}"))
            }
        })
        .on_tool(move |invocation: ToolInvocation| {
            let plugin = plugin_for_tool.clone();
            async move { dispatch_tool(plugin, invocation).await }
        });

    tracing::info!(
        target = "nexo_plugin_web_search",
        "JSON-RPC dispatch loop ready"
    );
    adapter.run_stdio().await?;
    Ok(())
}

async fn boot_broker(broker_url: &str) -> anyhow::Result<AnyBroker> {
    let broker_inner = nexo_config::types::broker::BrokerInner {
        kind: if broker_url.starts_with("nats://") {
            nexo_config::types::broker::BrokerKind::Nats
        } else {
            nexo_config::types::broker::BrokerKind::Local
        },
        url: broker_url.to_string(),
        auth: nexo_config::types::broker::BrokerAuthConfig::default(),
        persistence: nexo_config::types::broker::BrokerPersistenceConfig::default(),
        limits: nexo_config::types::broker::BrokerLimitsConfig::default(),
        fallback: nexo_config::types::broker::BrokerFallbackConfig::default(),
    };
    AnyBroker::from_config(&broker_inner)
        .await
        .map_err(|e| anyhow::anyhow!("broker connect failed: {e}"))
}

async fn dispatch_tool(
    plugin: Arc<WebSearchPlugin>,
    inv: ToolInvocation,
) -> Result<Value, ToolInvocationError> {
    let agent_id = inv.agent_id.as_deref().ok_or_else(|| {
        ToolInvocationError::ArgumentInvalid(
            "tool.invoke is missing `agent_id` (daemon must include it)".into(),
        )
    })?;
    match plugin
        .invoke_outbound_tool(&inv.tool_name, inv.args, agent_id, inv.policy.as_ref())
        .await
    {
        Ok(v) => Ok(v),
        Err(e) => {
            let msg = format!("{e}");
            if msg.contains("unknown tool") {
                Err(ToolInvocationError::NotFound(msg))
            } else if msg.contains("disabled by policy") {
                Err(ToolInvocationError::Denied(msg))
            } else if msg.contains("no configured web_search instance")
                || msg.contains("not configured")
                || msg.contains("breaker open")
            {
                Err(ToolInvocationError::Unavailable(msg))
            } else if msg.contains("requires `query`")
                || msg.contains("web_search args:")
                || msg.contains("query is empty")
            {
                Err(ToolInvocationError::ArgumentInvalid(msg))
            } else {
                Err(ToolInvocationError::ExecutionFailed(msg))
            }
        }
    }
}

fn spawn_auto_discovery_subscribers(broker: AnyBroker) {
    spawn_one(
        broker.clone(),
        "plugin.web_search.admin.>",
        |_b, payload| async move { auto_discovery::admin_handle(&payload).await },
    );
    spawn_one(
        broker,
        "plugin.web_search.metrics.scrape",
        |_b, payload| async move { auto_discovery::metrics_scrape(&payload).await },
    );
}

fn spawn_one<F, Fut>(broker: AnyBroker, topic: &'static str, handler: F)
where
    F: Fn(AnyBroker, Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Value> + Send + 'static,
{
    use nexo_broker::{BrokerHandle, Event, Message};
    tokio::spawn(async move {
        let mut sub = match broker.subscribe(topic).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    target = "web_search.auto_discovery",
                    topic,
                    error = %e,
                    "subscribe failed; topic will not receive requests"
                );
                return;
            }
        };
        tracing::info!(target = "web_search.auto_discovery", topic, "subscriber up");
        while let Some(event) = sub.next().await {
            let Ok(msg) = serde_json::from_value::<Message>(event.payload) else {
                continue;
            };
            let Some(reply_to) = msg.reply_to.clone() else {
                continue;
            };
            let reply_payload = handler(broker.clone(), msg.payload.clone()).await;
            let reply_msg = Message::new(reply_to.clone(), reply_payload);
            let reply_event = Event::new(
                reply_to.clone(),
                "web_search",
                match serde_json::to_value(&reply_msg) {
                    Ok(v) => v,
                    Err(_) => continue,
                },
            );
            if let Err(e) = broker.publish(&reply_to, reply_event).await {
                tracing::warn!(
                    target = "web_search.auto_discovery",
                    topic,
                    reply_to = %reply_to,
                    error = %e,
                    "failed to publish reply"
                );
            }
        }
        tracing::debug!(
            target = "web_search.auto_discovery",
            topic,
            "subscriber stream ended"
        );
    });
}
