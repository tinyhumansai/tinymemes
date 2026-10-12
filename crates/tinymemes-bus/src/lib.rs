//! Minimal content-only TinyMemes contract. No inference or storage implementation.
//!
//! Supplied snapshots and revision numbers are data. They confer no authority,
//! admission, durable CAS guarantee, research verification or source ownership.
mod content_usage;
pub use content_usage::*;
mod persistent;
pub use persistent::*;
mod intent;
mod records;
mod state;
mod tier;
pub use intent::Intent;
pub use records::*;
pub use state::*;
pub use tier::Tier;

/// Stateless content service name and interface.
pub const STATE_INTERFACE: &str = "ai.tinyhumans.tinymemes.ContentState";
/// Stateless content service object path.
pub const STATE_PATH: &str = "/ai/tinyhumans/tinymemes/ContentState";

/// Typed proxy for the compiled module, without linking TinyMemes implementation.
#[derive(Clone)]
pub struct StateClient(tinybus::Proxy);
impl StateClient {
    /// Bind a connection to the content service.
    pub fn new(connection: &tinybus::Connection) -> tinybus::Result<Self> {
        Ok(Self(connection.proxy(
            STATE_INTERFACE,
            STATE_PATH,
            STATE_INTERFACE,
        )?))
    }
    /// Transform supplied content. The nested result distinguishes content
    /// refusal from a bus/ABI transport failure. No state is persisted.
    pub async fn apply(&self, request: ApplyRequest) -> tinybus::Result<ApplyResult> {
        self.0.call("Apply", (request,)).await
    }
}

impl std::fmt::Debug for StateClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateClient").finish_non_exhaustive()
    }
}
