//! Subprocess-side env config loader.
//!
//! Daemon discovery walker spawns the binary with these env vars
//! (generic, no plugin-specific seeding code in the daemon —
//! agnostic per Wave 4 email precedent):
//!
//!   * `NEXO_BROKER_URL`            — broker URL.
//!   * `NEXO_BROKER_KIND`           — `nats` | `local` | `stdio_bridge`.
//!   * `NEXO_CONFIG_DIR`            — absolute path to the operator
//!                                     config dir (Phase 94 v0.2.1
//!                                     agnostic env seed). Plugin
//!                                     reads
//!                                     `$NEXO_CONFIG_DIR/plugins/web-search.yaml`
//!                                     here.
//!   * `NEXO_PLUGIN_WEB_SEARCH_CONFIG_PATH` — explicit override
//!                                     path for diagnostics.

use std::path::PathBuf;

use crate::plugin::WebSearchConfigFile;

#[derive(Debug, Default)]
pub struct WebSearchEnvConfig {
    pub broker_url: String,
    pub broker_kind: String,
    pub initial: Option<WebSearchConfigFile>,
    pub config_path: Option<PathBuf>,
}

pub fn web_search_config_from_env() -> anyhow::Result<WebSearchEnvConfig> {
    let broker_url = std::env::var("NEXO_BROKER_URL").unwrap_or_default();
    let broker_kind = std::env::var("NEXO_BROKER_KIND").unwrap_or_else(|_| {
        if broker_url.starts_with("nats://") {
            "nats".to_string()
        } else {
            "local".to_string()
        }
    });

    let candidate = resolve_config_path();
    let (initial, config_path) = match candidate.as_ref() {
        Some(path) if path.exists() => {
            let bytes = std::fs::read(path).map_err(|e| {
                anyhow::anyhow!("reading web-search config at {}: {e}", path.display())
            })?;
            let parsed: WebSearchConfigFile = serde_yaml::from_slice(&bytes).map_err(|e| {
                anyhow::anyhow!("parsing web-search.yaml at {}: {e}", path.display())
            })?;
            (Some(parsed), Some(path.clone()))
        }
        _ => (None, candidate),
    };

    Ok(WebSearchEnvConfig {
        broker_url,
        broker_kind,
        initial,
        config_path,
    })
}

fn resolve_config_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("NEXO_PLUGIN_WEB_SEARCH_CONFIG_PATH") {
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    if let Ok(cfg_dir) = std::env::var("NEXO_CONFIG_DIR") {
        if !cfg_dir.is_empty() {
            return Some(PathBuf::from(cfg_dir).join("plugins/web-search.yaml"));
        }
    }
    Some(PathBuf::from("./config/plugins/web-search.yaml"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    fn clean_env() {
        for k in [
            "NEXO_BROKER_URL",
            "NEXO_BROKER_KIND",
            "NEXO_CONFIG_DIR",
            "NEXO_PLUGIN_WEB_SEARCH_CONFIG_PATH",
        ] {
            unsafe {
                std::env::remove_var(k);
            }
        }
    }

    #[test]
    #[serial]
    fn defaults_when_no_file() {
        clean_env();
        unsafe {
            std::env::set_var("NEXO_CONFIG_DIR", "/nonexistent-test");
        }
        let cfg = web_search_config_from_env().unwrap();
        assert!(cfg.initial.is_none());
        assert_eq!(cfg.broker_kind, "local");
        clean_env();
    }

    #[test]
    #[serial]
    fn nats_url_implies_nats_kind() {
        clean_env();
        unsafe {
            std::env::set_var("NEXO_BROKER_URL", "nats://localhost:4222");
            std::env::set_var("NEXO_CONFIG_DIR", "/nonexistent-test");
        }
        let cfg = web_search_config_from_env().unwrap();
        assert_eq!(cfg.broker_kind, "nats");
        clean_env();
    }

    #[test]
    #[serial]
    fn reads_yaml_when_config_dir_points_at_real_file() {
        clean_env();
        let dir = tempfile::tempdir().unwrap();
        let plugins = dir.path().join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(
            plugins.join("web-search.yaml"),
            "instances:\n  - id: default\n    providers:\n      duckduckgo:\n        timeout_ms: 12000\n",
        )
        .unwrap();
        unsafe {
            std::env::set_var("NEXO_CONFIG_DIR", dir.path().to_string_lossy().into_owned());
        }
        let cfg = web_search_config_from_env().unwrap();
        let parsed = cfg.initial.expect("should load yaml");
        assert_eq!(parsed.instances.len(), 1);
        assert_eq!(parsed.instances[0].id, "default");
        clean_env();
    }
}
