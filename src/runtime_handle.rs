//! Process-wide handle to the active [`crate::plugin::WebSearchPlugin`].
//!
//! Mirrors google's `runtime_handle` pattern so auto-discovery
//! broker handlers reach the plugin's state without static
//! ownership at module-load time.

use std::sync::Arc;

use once_cell::sync::Lazy;
use tokio::sync::RwLock;

use crate::plugin::WebSearchPlugin;

static HANDLE: Lazy<RwLock<Option<Arc<WebSearchPlugin>>>> =
    Lazy::new(|| RwLock::new(None));

pub fn runtime_handle() -> &'static RwLock<Option<Arc<WebSearchPlugin>>> {
    &HANDLE
}

pub async fn set_runtime_handle(plugin: Arc<WebSearchPlugin>) {
    *HANDLE.write().await = Some(plugin);
}
