//! Subprocess + CLI entrypoint for `nexo-plugin-web-search`
//! (Phase 95).
//!
//! Dispatch:
//!   * `nexo-plugin-web-search --print-manifest` → echo bundled
//!     manifest.
//!   * no subcommand → long-lived JSON-RPC dispatch loop against
//!     stdin/stdout (daemon-spawned mode).
//!
//! Configuration delivered via daemon's `plugin.configure`
//! JSON-RPC (Phase 93) AND/OR
//! `$NEXO_CONFIG_DIR/plugins/web-search.yaml` discovered at boot
//! (Phase 94 v0.2.1 generic env seed).

fn main() -> anyhow::Result<()> {
    // Implementation lands in Phase 5 step 22. Stub keeps the bin
    // compilable while scaffolding.
    eprintln!("nexo-plugin-web-search scaffold; runtime wiring pending");
    Ok(())
}
