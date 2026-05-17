# Changelog

## 0.1.0 — 2026-05-17

Initial standalone-subprocess release. Phase 95 close-out of the
canonical plugin extraction lineage (browser, telegram, whatsapp,
email, google, web-search).

### Added

- `web_search` agent-callable tool wrapping `nexo-web-search 0.1.2`'s
  provider abstraction (Brave / Tavily / DuckDuckGo / Perplexity)
  + circuit breaker + sqlite cache.
- Multi-instance × multi-agent: operators declare N "search
  profiles" in `web-search.yaml::instances[]`. Tools accept
  optional `instance:` arg.
- Per-binding policy threading via the Phase 95
  `tool.invoke.params.policy` framework contract (requires
  nexo-core 0.2.0 + nexo-microapp-sdk 0.1.19).
- Admin RPCs: `bot_info`, `cache_stats`, `cache_clear`,
  `provider_status`, `list_instances`.
- Prometheus metrics via `plugin.web_search.metrics.scrape`
  broker handler.
- Subprocess CLI: `--print-manifest`.

### Architecture

Single subprocess holds `DashMap<instance_id, Arc<WebSearchRouter>>`
+ `DashMap<agent_id, Vec<instance_id>>` + `ArcSwap<Vec<shared_ids>>`.
Resolution precedence: `args.instance` → agent's private profile
→ first shared instance. Mirrors google plugin v0.2.1's
multi-account-per-agent pattern.

### Daemon-side requirements

- nexo-core 0.2.0 — breaking change drops the in-process
  `web_search_router` field + `WebSearchTool` from
  `AgentContext` / `AgentRuntime` / `AgentSpawnConfig` /
  `McpServerBootContext`. Tool now reaches agents via
  `RemoteToolHandler` → `tool.invoke` JSON-RPC over stdio.
- nexo-microapp-sdk 0.1.19 — additive
  `ToolInvocation.policy` field carries the per-binding policy
  slice resolved by the daemon's
  `EffectivePolicy::for_tool("web_search")`.

### Limitations

- `args.expand = true` accepted but no-op; v0.2.0 follow-up adds
  daemon-side link-extractor post-processor.
- Streaming progress not supported (tool.invoke is request/reply
  only today).
- `admin/cache_clear` returns placeholder error — `nexo-web-search
  0.1.2` doesn't expose a cache clear method.
