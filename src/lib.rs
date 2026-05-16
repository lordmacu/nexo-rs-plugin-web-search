//! `nexo-plugin-web-search` — multi-provider web search subprocess
//! plugin for Nexo agents.
//!
//! Phase 95 close-out of the canonical plugin extraction lineage
//! (browser, telegram, whatsapp, email, google, web-search).
//! Provides the `web_search` agent-callable tool wrapping the
//! published `nexo-web-search` crate's router + provider
//! abstraction (Brave / Tavily / DuckDuckGo / Perplexity).
//!
//! Multi-instance × multi-agent: operators declare N "search
//! profiles" in `web-search.yaml::instances[]`. Each instance
//! has its own provider set + cache + default_order. Instances
//! without `agent_id` are shared; instances with `agent_id` are
//! private to that agent. Tools accept optional `instance:` arg
//! (mirrors google's `account:` arg).

pub mod auto_discovery;
pub mod cli;
pub mod env_config;
pub mod plugin;
pub mod runtime_handle;
pub mod tools;

pub use plugin::{
    CacheConfig, ProviderEntry, ProvidersConfig, WebSearchConfigFile, WebSearchInstance,
    WebSearchPlugin,
};

pub use cli::{Cli, Command};
