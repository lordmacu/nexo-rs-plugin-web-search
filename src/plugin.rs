//! Subprocess plugin runtime — multi-instance × multi-agent.
//!
//! Operators describe their search profiles in
//! `<config_dir>/plugins/web-search.yaml`:
//!
//! ```yaml
//! instances:
//!   - id: default                  # required, unique
//!     # agent_id omitted → shared across all agents
//!     providers:
//!       brave:
//!         api_key_path: ./secrets/brave_api_key.txt
//!         timeout_ms:   8000
//!       tavily:
//!         api_key_path: ./secrets/tavily_api_key.txt
//!       duckduckgo:
//!         timeout_ms:   12000
//!     cache:
//!       enabled: true
//!       path:    ./data/web_search_default.db
//!       ttl_secs: 3600
//!     default_order: [brave, tavily, duckduckgo]
//!   - id: research                 # private profile for ana
//!     agent_id: ana
//!     providers:
//!       perplexity:
//!         api_key_path: ./secrets/ana_perplexity.txt
//!     cache:
//!       enabled: true
//!       path: ./data/ana_research.db
//!       ttl_secs: 1800
//! ```
//!
//! Resolution for an agent's tool call:
//!   1. `args.instance` (operator-supplied via tool arg).
//!   2. First entry of `by_agent[agent_id]` (private profile).
//!   3. First shared instance (agent_id absent in YAML).
//!   4. Error: no instance configured.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use arc_swap::ArcSwap;
use dashmap::DashMap;
use nexo_web_search::{
    providers::duckduckgo::DuckDuckGoProvider, ProviderState, WebSearchArgs, WebSearchCache,
    WebSearchProvider, WebSearchResult, WebSearchRouter,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Top-level operator config (mirrors `web-search.yaml`).
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct WebSearchConfigFile {
    #[serde(default)]
    pub instances: Vec<WebSearchInstance>,
}

/// One search profile. `agent_id` absent → shared default;
/// `agent_id` present → private to that single agent.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchInstance {
    pub id: String,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub default_order: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub brave: Option<ProviderEntry>,
    #[serde(default)]
    pub tavily: Option<ProviderEntry>,
    #[serde(default)]
    pub perplexity: Option<ProviderEntry>,
    #[serde(default)]
    pub duckduckgo: Option<ProviderEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderEntry {
    #[serde(default)]
    pub api_key_path: Option<PathBuf>,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_timeout_ms() -> u64 {
    10_000
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CacheConfig {
    #[serde(default = "default_cache_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default = "default_cache_ttl")]
    pub ttl_secs: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: default_cache_enabled(),
            path: None,
            ttl_secs: default_cache_ttl(),
        }
    }
}

fn default_cache_enabled() -> bool {
    true
}
fn default_cache_ttl() -> u64 {
    600
}

/// Phase 95 FU#1 — per-instance bundle tracked separately so
/// admin RPCs can reach the cache + router directly (the lib's
/// `WebSearchRouter` doesn't expose its inner cache).
struct InstanceState {
    router: Arc<WebSearchRouter>,
    cache: Option<Arc<WebSearchCache>>,
}

/// Process-wide state.
pub struct WebSearchPlugin {
    /// instance_id → bundle (router + cache handle). Each instance
    /// has its own provider set + cache + default_order. One
    /// process, many profiles.
    instances: DashMap<String, Arc<InstanceState>>,
    /// agent_id → ordered list of private instance_ids. First
    /// entry is the agent's default profile.
    by_agent: DashMap<String, Vec<String>>,
    /// Instance ids without an `agent_id` binding (shared across
    /// all agents). Reload-safe via ArcSwap.
    shared: ArcSwap<Vec<String>>,
}

impl Default for WebSearchPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl WebSearchPlugin {
    pub fn new() -> Self {
        Self {
            instances: DashMap::new(),
            by_agent: DashMap::new(),
            shared: ArcSwap::from_pointee(Vec::new()),
        }
    }

    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    pub fn agent_count(&self) -> usize {
        self.by_agent.len()
    }

    pub fn shared_count(&self) -> usize {
        self.shared.load().len()
    }

    /// Replace state from a parsed `web-search.yaml`. Full-replace:
    /// instances absent from the new payload get dropped. Cache
    /// path uniqueness is validated; collisions are rejected
    /// because two instances writing the same sqlite file would
    /// corrupt each other.
    pub async fn on_configure(&self, file: WebSearchConfigFile) -> Result<()> {
        // Pre-flight: instance ids unique + cache paths unique.
        let mut seen_ids: std::collections::HashSet<&str> =
            std::collections::HashSet::new();
        let mut seen_cache_paths: std::collections::HashSet<&std::path::Path> =
            std::collections::HashSet::new();
        for inst in &file.instances {
            if !seen_ids.insert(inst.id.as_str()) {
                return Err(anyhow!(
                    "duplicate instance id `{}` in web-search.yaml",
                    inst.id
                ));
            }
            if inst.cache.enabled {
                if let Some(path) = inst.cache.path.as_deref() {
                    if !seen_cache_paths.insert(path) {
                        return Err(anyhow!(
                            "instance `{}` cache.path `{}` collides with another instance",
                            inst.id,
                            path.display()
                        ));
                    }
                }
            }
        }

        let next_instances: DashMap<String, Arc<InstanceState>> = DashMap::new();
        let next_by_agent: DashMap<String, Vec<String>> = DashMap::new();
        let mut next_shared: Vec<String> = Vec::new();

        for inst in &file.instances {
            let bundle = build_instance(inst).await.with_context(|| {
                format!("building router for instance `{}`", inst.id)
            })?;
            next_instances.insert(inst.id.clone(), Arc::new(bundle));
            match &inst.agent_id {
                Some(agent) => {
                    next_by_agent
                        .entry(agent.clone())
                        .or_default()
                        .push(inst.id.clone());
                }
                None => {
                    next_shared.push(inst.id.clone());
                }
            }
        }

        self.instances.clear();
        for (k, v) in next_instances.into_iter() {
            self.instances.insert(k, v);
        }
        self.by_agent.clear();
        for (k, v) in next_by_agent.into_iter() {
            self.by_agent.insert(k, v);
        }
        self.shared.store(Arc::new(next_shared));

        tracing::info!(
            target = "nexo_plugin_web_search",
            instances = self.instances.len(),
            agents = self.by_agent.len(),
            shared = self.shared.load().len(),
            "web_search plugin reconfigured"
        );
        Ok(())
    }

    /// Pick the instance for this call:
    ///   1. `args.instance` if operator-supplied.
    ///   2. agent's first private instance from `by_agent`.
    ///   3. plugin's first shared instance.
    pub fn resolve_instance(&self, args: &Value, agent_id: &str) -> Result<String> {
        if let Some(explicit) = args.get("instance").and_then(|v| v.as_str()) {
            return Ok(explicit.to_string());
        }
        if let Some(list) = self.by_agent.get(agent_id) {
            if let Some(first) = list.first() {
                return Ok(first.clone());
            }
        }
        let shared = self.shared.load();
        if let Some(first) = shared.first() {
            return Ok(first.clone());
        }
        Err(anyhow!(
            "agent `{agent_id}` has no configured web_search instance \
             (no private profile in `by_agent` and no shared instance \
             in `instances[].agent_id == None`)"
        ))
    }

    pub fn router_for(&self, instance_id: &str) -> Result<Arc<WebSearchRouter>> {
        self.instances
            .get(instance_id)
            .map(|r| Arc::clone(&r.value().router))
            .ok_or_else(|| anyhow!("instance `{instance_id}` is not configured"))
    }

    fn bundle_for(&self, instance_id: &str) -> Option<Arc<InstanceState>> {
        self.instances.get(instance_id).map(|r| Arc::clone(r.value()))
    }

    pub fn instances_for_agent(&self, agent_id: &str) -> Vec<String> {
        self.by_agent
            .get(agent_id)
            .map(|r| r.clone())
            .unwrap_or_default()
    }

    /// Dispatch a `tool.invoke` call from the daemon's
    /// `RemoteToolHandler`. Honours the Phase 95 per-binding
    /// policy slice (gate via `policy.enabled`, default count +
    /// provider override).
    pub async fn invoke_outbound_tool(
        &self,
        tool_name: &str,
        args: Value,
        agent_id: &str,
        policy: Option<&Value>,
    ) -> Result<Value> {
        if tool_name != "web_search" {
            return Err(anyhow!("unknown tool `{tool_name}`"));
        }
        // Per-binding policy gate.
        if let Some(p) = policy {
            if p.get("enabled").and_then(|v| v.as_bool()) == Some(false) {
                return Err(anyhow!(
                    "web_search disabled by policy on this binding"
                ));
            }
        }

        let instance_id = self.resolve_instance(&args, agent_id)?;
        let router = self.router_for(&instance_id)?;

        let mut search_args: WebSearchArgs = serde_json::from_value(args.clone())
            .map_err(|e| anyhow!("web_search args: {e}"))?;

        // Apply policy defaults when LLM omitted fields.
        if let Some(p) = policy {
            if search_args.count.is_none() {
                if let Some(c) = p.get("default_count").and_then(|v| v.as_u64()) {
                    search_args.count = Some(c.min(255) as u8);
                }
            }
            if !search_args.expand {
                if let Some(b) = p.get("expand_default").and_then(|v| v.as_bool()) {
                    search_args.expand = b;
                }
            }
        }

        // Provider precedence: explicit arg > policy.provider > router auto.
        let policy_provider = policy
            .and_then(|p| p.get("provider").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty() && *s != "auto")
            .map(|s| s.to_string());
        let provider_pick = search_args.provider.clone().or(policy_provider);

        let result: WebSearchResult = router
            .search(search_args.clone(), provider_pick.as_deref())
            .await
            .map_err(|e| anyhow!("web_search: {e}"))?;

        let mut value = serde_json::to_value(result)?;
        // Echo the resolved instance + expand-supported hint.
        if let Some(map) = value.as_object_mut() {
            map.insert("instance".into(), json!(instance_id));
            if search_args.expand {
                // v0.1.0 doesn't run link-extractor inside the
                // subprocess; signal the missing capability so
                // the LLM doesn't assume bodies were attached.
                map.insert("expand_supported".into(), json!(false));
            }
        }
        Ok(value)
    }

    // ── Admin handlers ──────────────────────────────────────────

    /// Phase 95 FU#1 (0.1.1) — per-instance entry counts via
    /// `WebSearchCache::stats()` (nexo-web-search 0.2.0).
    pub async fn admin_cache_stats(&self, instance: Option<&str>) -> Result<Value> {
        let mut stats: Vec<Value> = Vec::new();
        let ids: Vec<String> = match instance {
            Some(name) => vec![name.to_string()],
            None => self.instances.iter().map(|e| e.key().clone()).collect(),
        };
        for id in ids {
            let Some(bundle) = self.bundle_for(&id) else {
                stats.push(json!({
                    "instance": id,
                    "ok": false,
                    "error": "instance not configured",
                }));
                continue;
            };
            match bundle.cache.as_ref() {
                Some(cache) => match cache.stats().await {
                    Ok(s) => stats.push(json!({
                        "instance": id,
                        "ok": true,
                        "entries": s.entries,
                    })),
                    Err(e) => stats.push(json!({
                        "instance": id,
                        "ok": false,
                        "error": format!("{e}"),
                    })),
                },
                None => stats.push(json!({
                    "instance": id,
                    "ok": true,
                    "cache_enabled": false,
                })),
            }
        }
        Ok(json!({ "instances": stats }))
    }

    /// Phase 95 FU#1 (0.1.1) — flush cache via
    /// `WebSearchCache::clear()` (nexo-web-search 0.2.0). When
    /// `instance` is `None`, clears every configured instance's
    /// cache; reports per-instance row counts deleted.
    pub async fn admin_cache_clear(&self, instance: Option<&str>) -> Result<Value> {
        let mut cleared: Vec<Value> = Vec::new();
        let ids: Vec<String> = match instance {
            Some(name) => vec![name.to_string()],
            None => self.instances.iter().map(|e| e.key().clone()).collect(),
        };
        for id in ids {
            let Some(bundle) = self.bundle_for(&id) else {
                cleared.push(json!({
                    "instance": id,
                    "ok": false,
                    "error": "instance not configured",
                }));
                continue;
            };
            match bundle.cache.as_ref() {
                Some(cache) => match cache.clear().await {
                    Ok(rows) => cleared.push(json!({
                        "instance": id,
                        "ok": true,
                        "rows_deleted": rows,
                    })),
                    Err(e) => cleared.push(json!({
                        "instance": id,
                        "ok": false,
                        "error": format!("{e}"),
                    })),
                },
                None => cleared.push(json!({
                    "instance": id,
                    "ok": true,
                    "cache_enabled": false,
                    "rows_deleted": 0,
                })),
            }
        }
        Ok(json!({ "instances": cleared }))
    }

    /// Phase 95 FU#1 (0.1.1) — per-provider operational state via
    /// `WebSearchRouter::provider_states()` (nexo-web-search 0.2.0).
    /// Reports each instance's providers + their current circuit-
    /// breaker availability. `available: false` means the
    /// provider's breaker is open (recent failures); the router
    /// will skip it on the next call.
    pub async fn admin_provider_status(&self) -> Result<Value> {
        let mut out: Vec<Value> = Vec::with_capacity(self.instances.len());
        for entry in self.instances.iter() {
            let states: Vec<ProviderState> = entry.value().router.provider_states();
            out.push(json!({
                "instance": entry.key(),
                "providers": states,
            }));
        }
        Ok(json!({ "instances": out }))
    }

    pub async fn admin_list_instances(&self) -> Result<Value> {
        let mut instances: Vec<String> = self
            .instances
            .iter()
            .map(|e| e.key().clone())
            .collect();
        instances.sort();
        let mut by_agent: serde_json::Map<String, Value> =
            serde_json::Map::new();
        for entry in self.by_agent.iter() {
            by_agent.insert(entry.key().clone(), json!(entry.value()));
        }
        Ok(json!({
            "instances": instances,
            "by_agent": Value::Object(by_agent),
            "shared": *self.shared.load_full(),
        }))
    }
}

/// Build a single instance bundle (router + cache handle).
async fn build_instance(inst: &WebSearchInstance) -> Result<InstanceState> {
    let providers = build_providers(&inst.providers).await?;
    if providers.is_empty() {
        return Err(anyhow!(
            "instance `{}` declares no providers; at least one is required",
            inst.id
        ));
    }
    let cache = if inst.cache.enabled {
        let ttl = Duration::from_secs(inst.cache.ttl_secs.max(1));
        let cache_handle = match inst.cache.path.as_ref() {
            Some(path) => WebSearchCache::open(&path.to_string_lossy(), ttl)
                .await
                .map_err(|e| anyhow!("opening cache for `{}`: {e}", inst.id))?,
            None => WebSearchCache::open_memory(ttl)
                .await
                .map_err(|e| anyhow!("opening in-memory cache for `{}`: {e}", inst.id))?,
        };
        Some(Arc::new(cache_handle))
    } else {
        None
    };
    let router = Arc::new(WebSearchRouter::new(providers, cache.clone()));
    Ok(InstanceState { router, cache })
}

async fn build_providers(
    cfg: &ProvidersConfig,
) -> Result<Vec<Arc<dyn WebSearchProvider>>> {
    let mut out: Vec<Arc<dyn WebSearchProvider>> = Vec::new();

    if let Some(entry) = cfg.brave.as_ref() {
        if let Some(key) = read_api_key(entry).await? {
            out.push(Arc::new(
                nexo_web_search::providers::brave::BraveProvider::new(key, entry.timeout_ms),
            ));
        }
    }
    if let Some(entry) = cfg.tavily.as_ref() {
        if let Some(key) = read_api_key(entry).await? {
            out.push(Arc::new(
                nexo_web_search::providers::tavily::TavilyProvider::new(key, entry.timeout_ms),
            ));
        }
    }
    // Perplexity: feature-gated in the lib. When operator's YAML
    // includes a perplexity entry but the plugin was built without
    // the `perplexity` cargo feature, warn + skip rather than fail
    // — operators can rebuild with the feature on if they need it.
    #[cfg(feature = "perplexity")]
    if let Some(entry) = cfg.perplexity.as_ref() {
        if let Some(key) = read_api_key(entry).await? {
            out.push(Arc::new(
                nexo_web_search::providers::perplexity::PerplexityProvider::new(
                    key,
                    "sonar-pro".to_string(),
                    entry.timeout_ms,
                ),
            ));
        }
    }
    #[cfg(not(feature = "perplexity"))]
    if cfg.perplexity.is_some() {
        tracing::warn!(
            target = "nexo_plugin_web_search",
            "perplexity provider declared in config but plugin built without \
             `perplexity` feature; skipping. Rebuild with \
             `--features perplexity` to enable."
        );
    }
    if let Some(entry) = cfg.duckduckgo.as_ref() {
        out.push(Arc::new(DuckDuckGoProvider::new(entry.timeout_ms)));
    }

    Ok(out)
}

async fn read_api_key(entry: &ProviderEntry) -> Result<Option<String>> {
    let Some(path) = entry.api_key_path.as_ref() else {
        return Ok(None);
    };
    let raw = tokio::fs::read_to_string(path).await.with_context(|| {
        format!("reading API key file at {}", path.display())
    })?;
    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brave_only_instance(id: &str, agent: Option<&str>, dir: &std::path::Path) -> WebSearchInstance {
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
    async fn on_configure_empty_yields_empty_state() {
        let p = WebSearchPlugin::new();
        p.on_configure(WebSearchConfigFile::default()).await.unwrap();
        assert_eq!(p.instance_count(), 0);
        assert_eq!(p.agent_count(), 0);
        assert_eq!(p.shared_count(), 0);
    }

    #[tokio::test]
    async fn on_configure_single_shared_instance() {
        let dir = tempfile::tempdir().unwrap();
        let p = WebSearchPlugin::new();
        p.on_configure(WebSearchConfigFile {
            instances: vec![brave_only_instance("default", None, dir.path())],
        })
        .await
        .unwrap();
        assert_eq!(p.instance_count(), 1);
        assert_eq!(p.shared_count(), 1);
        assert_eq!(p.agent_count(), 0);
    }

    #[tokio::test]
    async fn on_configure_multi_private_for_one_agent() {
        let dir = tempfile::tempdir().unwrap();
        let p = WebSearchPlugin::new();
        p.on_configure(WebSearchConfigFile {
            instances: vec![
                brave_only_instance("research", Some("ana"), dir.path()),
                brave_only_instance("news", Some("ana"), dir.path()),
            ],
        })
        .await
        .unwrap();
        assert_eq!(p.instance_count(), 2);
        assert_eq!(p.shared_count(), 0);
        let ana_list = p.instances_for_agent("ana");
        assert_eq!(ana_list, vec!["research", "news"]);
    }

    #[tokio::test]
    async fn on_configure_mixed_shared_and_private() {
        let dir = tempfile::tempdir().unwrap();
        let p = WebSearchPlugin::new();
        p.on_configure(WebSearchConfigFile {
            instances: vec![
                brave_only_instance("default", None, dir.path()),
                brave_only_instance("ana_research", Some("ana"), dir.path()),
            ],
        })
        .await
        .unwrap();
        assert_eq!(p.instance_count(), 2);
        assert_eq!(p.shared_count(), 1);
        assert_eq!(p.agent_count(), 1);
    }

    #[tokio::test]
    async fn on_configure_full_replace_drops_old_instances() {
        let dir = tempfile::tempdir().unwrap();
        let p = WebSearchPlugin::new();
        p.on_configure(WebSearchConfigFile {
            instances: vec![
                brave_only_instance("a", Some("ana"), dir.path()),
                brave_only_instance("b", Some("bob"), dir.path()),
            ],
        })
        .await
        .unwrap();
        assert_eq!(p.instance_count(), 2);
        p.on_configure(WebSearchConfigFile {
            instances: vec![brave_only_instance("c", None, dir.path())],
        })
        .await
        .unwrap();
        assert_eq!(p.instance_count(), 1);
        assert!(p.router_for("a").is_err());
        assert!(p.router_for("c").is_ok());
        assert_eq!(p.agent_count(), 0);
        assert_eq!(p.shared_count(), 1);
    }

    #[tokio::test]
    async fn on_configure_rejects_duplicate_instance_ids() {
        let dir = tempfile::tempdir().unwrap();
        let p = WebSearchPlugin::new();
        let err = p
            .on_configure(WebSearchConfigFile {
                instances: vec![
                    brave_only_instance("default", None, dir.path()),
                    brave_only_instance("default", Some("ana"), dir.path()),
                ],
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("duplicate instance id"));
    }

    #[tokio::test]
    async fn on_configure_rejects_cache_path_collision() {
        let dir = tempfile::tempdir().unwrap();
        let shared_cache = dir.path().join("shared.db");
        let p = WebSearchPlugin::new();
        let mut a = brave_only_instance("a", None, dir.path());
        let mut b = brave_only_instance("b", None, dir.path());
        a.cache = CacheConfig {
            enabled: true,
            path: Some(shared_cache.clone()),
            ttl_secs: 600,
        };
        b.cache = CacheConfig {
            enabled: true,
            path: Some(shared_cache),
            ttl_secs: 600,
        };
        let err = p
            .on_configure(WebSearchConfigFile {
                instances: vec![a, b],
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("collides"));
    }

    #[tokio::test]
    async fn resolve_instance_picks_args_when_explicit() {
        let dir = tempfile::tempdir().unwrap();
        let p = WebSearchPlugin::new();
        p.on_configure(WebSearchConfigFile {
            instances: vec![
                brave_only_instance("default", None, dir.path()),
                brave_only_instance("research", Some("ana"), dir.path()),
            ],
        })
        .await
        .unwrap();
        let id = p
            .resolve_instance(&json!({"instance": "research"}), "bob")
            .unwrap();
        assert_eq!(id, "research");
    }

    #[tokio::test]
    async fn resolve_instance_falls_back_to_agent_private() {
        let dir = tempfile::tempdir().unwrap();
        let p = WebSearchPlugin::new();
        p.on_configure(WebSearchConfigFile {
            instances: vec![
                brave_only_instance("default", None, dir.path()),
                brave_only_instance("ana_research", Some("ana"), dir.path()),
            ],
        })
        .await
        .unwrap();
        let id = p.resolve_instance(&json!({}), "ana").unwrap();
        assert_eq!(id, "ana_research");
    }

    #[tokio::test]
    async fn resolve_instance_falls_back_to_shared_when_no_private() {
        let dir = tempfile::tempdir().unwrap();
        let p = WebSearchPlugin::new();
        p.on_configure(WebSearchConfigFile {
            instances: vec![brave_only_instance("default", None, dir.path())],
        })
        .await
        .unwrap();
        let id = p.resolve_instance(&json!({}), "bob").unwrap();
        assert_eq!(id, "default");
    }

    #[tokio::test]
    async fn resolve_instance_errors_when_nothing_configured() {
        let p = WebSearchPlugin::new();
        let err = p.resolve_instance(&json!({}), "bob").unwrap_err();
        assert!(err.to_string().contains("no configured web_search instance"));
    }

    #[tokio::test]
    async fn invoke_policy_disabled_returns_denied() {
        let dir = tempfile::tempdir().unwrap();
        let p = WebSearchPlugin::new();
        p.on_configure(WebSearchConfigFile {
            instances: vec![brave_only_instance("default", None, dir.path())],
        })
        .await
        .unwrap();
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
    async fn invoke_unknown_tool_errors() {
        let p = WebSearchPlugin::new();
        let err = p
            .invoke_outbound_tool("web_bogus", json!({"query": "x"}), "ana", None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("unknown tool"));
    }

    #[tokio::test]
    async fn admin_list_instances_returns_full_map() {
        let dir = tempfile::tempdir().unwrap();
        let p = WebSearchPlugin::new();
        p.on_configure(WebSearchConfigFile {
            instances: vec![
                brave_only_instance("default", None, dir.path()),
                brave_only_instance("ana_research", Some("ana"), dir.path()),
            ],
        })
        .await
        .unwrap();
        let listing = p.admin_list_instances().await.unwrap();
        let instances = listing["instances"].as_array().unwrap();
        assert_eq!(instances.len(), 2);
        assert_eq!(
            listing["shared"].as_array().unwrap()[0],
            json!("default")
        );
        assert_eq!(
            listing["by_agent"]["ana"].as_array().unwrap()[0],
            json!("ana_research")
        );
    }
}
