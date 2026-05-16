//! Bundled `nexo-plugin.toml` parses cleanly and declares the
//! expected sections.

use nexo_plugin_manifest::manifest::{ConfigShape, PluginManifest};

const MANIFEST: &str = include_str!("../nexo-plugin.toml");

#[test]
fn manifest_parses_as_v2() {
    let parsed: PluginManifest =
        toml::from_str(MANIFEST).expect("nexo-plugin.toml parses as PluginManifest");
    assert_eq!(parsed.manifest_version, 2);
    assert_eq!(parsed.plugin.id, "web_search");
    assert_eq!(parsed.plugin.version.to_string(), "0.1.0");
}

#[test]
fn extends_tools_declares_web_search() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let tools = &parsed.plugin.extends.tools;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0], "web_search");
}

#[test]
fn expose_list_matches_extends_tools() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let exposed = &parsed.plugin.tools.expose;
    assert_eq!(exposed.len(), 1);
    for name in &parsed.plugin.extends.tools {
        assert!(exposed.contains(name), "{name} missing from expose list");
    }
}

#[test]
fn admin_section_present_with_correct_prefix() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let admin = parsed
        .plugin
        .admin
        .as_ref()
        .expect("[plugin.admin] required");
    assert_eq!(admin.method_prefix, "nexo/admin/web_search/");
    assert_eq!(admin.broker_topic_prefix, "plugin.web_search.admin");
}

#[test]
fn config_schema_is_object_shape_with_instances_array() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let cs = parsed
        .plugin
        .config_schema
        .as_ref()
        .expect("[plugin.config_schema] required");
    assert_eq!(cs.shape, ConfigShape::Object);
    assert!(
        cs.schema.contains("\"instances\""),
        "schema must declare an instances array"
    );
    assert!(
        cs.schema.contains("\"agent_id\""),
        "schema must include agent_id field for multi-instance × multi-agent fanout"
    );
}

#[test]
fn credentials_schema_opted_out() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let cs = parsed
        .plugin
        .credentials_schema
        .as_ref()
        .expect("[plugin.credentials_schema] required (even when opting out)");
    assert!(
        !cs.enabled,
        "web_search plugin reads API key file refs directly; no RemoteCredentialStore"
    );
}

#[test]
fn dashboard_layout_workspace_walk_for_multi_instance() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let dash = parsed
        .plugin
        .dashboard
        .as_ref()
        .expect("[plugin.dashboard] required");
    let serialised = toml::to_string(dash).unwrap();
    assert!(
        serialised.contains("workspace_walk"),
        "dashboard.layout must be workspace_walk (multi-instance): {serialised}"
    );
    assert!(
        serialised.contains("brave_api_key.txt"),
        "auth_check candidates must include brave_api_key.txt: {serialised}"
    );
}

#[test]
fn entrypoint_command_matches_bin_name() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    assert_eq!(
        parsed.plugin.entrypoint.command.as_deref(),
        Some("nexo-plugin-web-search")
    );
}

#[test]
fn capabilities_broker_subscribes_to_outbound_and_admin() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let broker = parsed
        .plugin
        .capabilities
        .broker
        .as_ref()
        .expect("[plugin.capabilities.broker] required");
    assert!(broker
        .subscribe
        .iter()
        .any(|t| t == "plugin.outbound.web_search"));
    assert!(broker
        .subscribe
        .iter()
        .any(|t| t == "plugin.outbound.web_search.>"));
    assert!(broker
        .subscribe
        .iter()
        .any(|t| t == "plugin.web_search.admin.>"));
    assert!(broker
        .subscribe
        .iter()
        .any(|t| t == "plugin.web_search.metrics.scrape"));
}

#[test]
fn metrics_section_present() {
    let parsed: PluginManifest = toml::from_str(MANIFEST).unwrap();
    let metrics = parsed
        .plugin
        .metrics
        .as_ref()
        .expect("[plugin.metrics] required");
    let serialised = toml::to_string(metrics).unwrap();
    assert!(serialised.contains("prometheus"));
    assert!(serialised.contains("plugin.web_search"));
}
