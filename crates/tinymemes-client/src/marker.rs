//! Client for the persistent remix-marker interface.
use tinymemes_bus::{MARKER_INTERFACE, MarkRemixedReply, MarkerOutcome, MarkerRequest};

/// A client constructed only from the host-selected callback capability.
#[derive(Clone, Debug)]
pub struct RemixMarkerClient(tinybus::module::HostCallbackProxy);
impl RemixMarkerClient {
    /// Bind an already-authorized, exact-interface callback proxy.
    pub fn new(proxy: tinybus::module::HostCallbackProxy) -> tinybus::Result<Self> {
        if proxy.interface().as_str() != MARKER_INTERFACE {
            return Err(tinybus::Error::failed("marker interface mismatch"));
        }
        Ok(Self(proxy))
    }
    /// Record a marker using fresh native state inside the compiled module.
    pub async fn mark_remixed(
        &self,
        request: MarkerRequest,
    ) -> tinybus::Result<MarkerOutcome<MarkRemixedReply>> {
        self.0.call_confidential("MarkRemixed", (request,)).await
    }
    /// Read a marker using fresh native state inside the compiled module.
    pub async fn was_remixed(
        &self,
        request: MarkerRequest,
    ) -> tinybus::Result<MarkerOutcome<bool>> {
        self.0.call_confidential("WasRemixed", (request,)).await
    }
}
