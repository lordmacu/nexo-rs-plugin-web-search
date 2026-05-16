//! CLI subcommand wiring — `--print-manifest` only. Web-search
//! providers are API-key based with no consent flow, so no
//! `--oauth-once` analogue ships in v0.1.0.

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "nexo-plugin-web-search",
    version,
    about = "Multi-provider web search (Brave / Tavily / DuckDuckGo / Perplexity) tool plugin for Nexo agents.",
    long_about = "Subprocess binary loaded by the nexo-rs daemon via discovery. \
Without a subcommand, boots the long-lived JSON-RPC dispatch loop on stdin/stdout. \
Use --print-manifest to dump the bundled manifest (used by the daemon's discovery walker)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Emit the bundled `nexo-plugin.toml` and exit.
    #[command(name = "--print-manifest")]
    PrintManifest,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_print_manifest_subcommand() {
        let cli = Cli::try_parse_from(["nexo-plugin-web-search", "--print-manifest"]).unwrap();
        matches!(cli.command, Some(Command::PrintManifest));
    }

    #[test]
    fn no_subcommand_yields_none() {
        let cli = Cli::try_parse_from(["nexo-plugin-web-search"]).unwrap();
        assert!(cli.command.is_none());
    }
}
