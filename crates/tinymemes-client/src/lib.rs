//! Typed TinyBus clients for the TinyMemes module.
//!
//! The wire vocabulary lives in `tinymemes-bus`, which is synchronous and does
//! no I/O. This crate adds the bus-facing half: proxies a host binds to a
//! `tinybus::Connection`, without linking the module's implementation.
pub use tinymemes_bus as bus;
use tinymemes_bus::{
    ApplyRequest, ApplyResult, REMIX_INTERFACE, REMIX_PATH, RemixRequest, RemixResult,
    STATE_INTERFACE, STATE_PATH, remix_methods,
};

#[cfg(feature = "host-callback")]
mod content_usage;
#[cfg(feature = "host-callback")]
pub use content_usage::*;
#[cfg(feature = "host-callback")]
mod marker;
#[cfg(feature = "host-callback")]
pub use marker::*;

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
        self.0.call(tinymemes_bus::APPLY_METHOD, (request,)).await
    }
}

/// Typed proxy for the remix service.
#[derive(Clone)]
pub struct RemixClient(tinybus::Proxy);
impl RemixClient {
    /// Bind a connection to the remix service.
    pub fn new(connection: &tinybus::Connection) -> tinybus::Result<Self> {
        // The module claims one bus name (the content service's); the remix
        // interface is served at its own path on that peer.
        Ok(Self(connection.proxy(
            STATE_INTERFACE,
            REMIX_PATH,
            REMIX_INTERFACE,
        )?))
    }
    /// Remix one reply. The host must serve `HOST_INTERFACE` meanwhile.
    pub async fn remix(&self, request: RemixRequest) -> tinybus::Result<RemixResult> {
        self.0.call(remix_methods::REMIX, (request,)).await
    }
}

impl std::fmt::Debug for RemixClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemixClient").finish_non_exhaustive()
    }
}

impl std::fmt::Debug for StateClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateClient").finish_non_exhaustive()
    }
}
