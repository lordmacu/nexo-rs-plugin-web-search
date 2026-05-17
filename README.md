# nexo-plugin-web-search

Multi-provider web search (Brave / Tavily / DuckDuckGo /
Perplexity) tool plugin for
[`nexo-rs`](https://github.com/lordmacu/nexo-rs) agents.
Subprocess binary discovered + spawned by the daemon via
`[plugin.entrypoint]`; loaded out-of-tree so the daemon never
links the search machinery directly.

Phase 95 close-out of the canonical plugin extraction lineage
(browser → telegram → whatsapp → email → google → **web-search**).

## What it ships

One agent-callable tool — `web_search` — routed through the
canonical `tool.invoke` JSON-RPC path with `agent_id` and
`params.policy` (Phase 95 framework contract) per call:

| Tool | Purpose |
|------|---------|
| `web_search` | Search the web. Returns titles / URLs / snippets from a configured provider. Multi-instance: optional `instance:` arg picks a named search profile from `web-search.yaml`. Optional `provider:` arg overrides the router pick. |

Plus admin RPCs under `nexo/admin/web_search/`:
`bot_info`, `cache_stats`, `cache_clear`, `provider_status`,
`list_instances`.

## Install

```bash
cargo install nexo-plugin-web-search
```

The binary lands at `$HOME/.cargo/bin/nexo-plugin-web-search`.
The daemon's discovery walker probes it with `--print-manifest`
and auto-registers without any extra configuration.

## Configuration

Operator YAML at `<config_dir>/plugins/web-search.yaml`:

```yaml
instances:
  - id: default                     # required, unique
    # agent_id omitted → shared across all agents
    providers:
      brave:
        api_key_path: ./secrets/brave_api_key.txt
        timeout_ms:   8000
      tavily:
        api_key_path: ./secrets/tavily_api_key.txt
        timeout_ms:   10000
      duckduckgo:
        timeout_ms:   12000          # no API key needed
    cache:
      enabled: true
      path:    ./data/web_search_cache.db
      ttl_secs: 3600
    default_order: [brave, tavily, duckduckgo]
```

### Multi-instance × multi-agent (advanced)

```yaml
instances:
  - id: default                     # shared baseline
    providers: { brave: {...}, duckduckgo: {} }
    cache: { path: ./data/default.db }
    default_order: [brave, duckduckgo]
  - id: research                    # private profile for ana
    agent_id: ana
    providers: { perplexity: {...}, tavily: {...} }
    cache: { path: ./data/ana_research.db, ttl_secs: 1800 }
    default_order: [perplexity, tavily]
  - id: news                        # another private profile for ana
    agent_id: ana
    providers: { brave: {...} }
    cache: { enabled: false }       # ephemeral
    default_order: [brave]
```

Tool resolution per call for agent `ana`:
1. `args.instance` if operator-supplied via tool arg.
2. else `ana`'s first private instance (`research`).
3. else first shared instance (`default`).

## CLI

```text
nexo-plugin-web-search                  # JSON-RPC dispatch loop (daemon-spawned)
nexo-plugin-web-search --print-manifest # emit bundled manifest, exit 0
```

(Unlike the google plugin, web-search has no consent flow, so
no `--oauth-once` analog ships in v0.1.0.)

## Architecture

- **Single subprocess, many instances.** One process holds N
  `Arc<WebSearchRouter>` keyed by instance id. Each instance has
  its own provider set + cache + default_order. Instances
  without `agent_id` are shared; instances with `agent_id` are
  private to that agent.
- **Per-binding policy threading.** Phase 95's
  `tool.invoke.params.policy` carries the per-binding
  `WebSearchPolicy` slice: `enabled` gate (blocks calls before
  reaching the router), `default_count`, `provider` override,
  `expand_default`. Agnostic — every future subprocess tool
  needing per-binding gating reuses the same envelope.
- **Wraps published `nexo-web-search 0.1.2`.** Provider
  abstraction + sqlite cache + per-provider circuit breaker
  + fallback cascade come from the underlying lib.

## Manifest sections

- `[plugin]` id="web_search" v0.1.0 + min_nexo_version=">=0.1.19".
- `[plugin.entrypoint]` command="nexo-plugin-web-search".
- `[plugin.requires]` nexo_capabilities = ["broker"].
- `[plugin.capabilities.broker]` subscribe allowlist.
- `[plugin.tools]` expose=["web_search"] + extends.tools=["web_search"].
- `[plugin.admin]` nexo/admin/web_search/* method prefix.
- `[plugin.metrics]` prometheus=true broker scrape.
- `[plugin.config_schema]` shape="object" with `instances:[]` array.
- `[plugin.credentials_schema]` enabled=false.
- `[plugin.dashboard.layout]` workspace_walk subdir=web_search.

## Limitations (v0.1.0)

- `args.expand` accepted but no-op. v0.1.0 doesn't run the
  daemon-side link-extractor inside the subprocess; response
  includes `expand_supported: false` hint. v0.2.0 follow-up adds
  a daemon-side post-processor.
- Streaming progress (`WebSearchProgress`) not supported — the
  current `tool.invoke` contract is request/reply only.
- Cache clear admin RPC returns a placeholder error (clearing
  isn't exposed by `nexo-web-search 0.1.2`); follow-up.

## License

Dual-licensed under MIT OR Apache-2.0.

## Status

Phase 95 shipped 2026-05-17. Source:
[github.com/lordmacu/nexo-rs-plugin-web-search](https://github.com/lordmacu/nexo-rs-plugin-web-search).
