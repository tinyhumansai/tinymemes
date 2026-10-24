//! Client for the persistent catalog-usage interface.
use tinymemes_bus::{
    CONTENT_USAGE_INTERFACE, ContentUsageOutcome, MemeUsageRequest, RemoveMemeRequest,
    SlangUsageRequest,
};

/// A client carrying only an already-authorized callback proxy.
#[derive(Clone, Debug)]
pub struct ContentUsageClient(tinybus::module::HostCallbackProxy);
impl ContentUsageClient {
    /// Bind the exact persistent catalog interface.
    pub fn new(proxy: tinybus::module::HostCallbackProxy) -> tinybus::Result<Self> {
        if proxy.interface().as_str() != CONTENT_USAGE_INTERFACE {
            return Err(tinybus::Error::failed("catalog interface mismatch"));
        }
        Ok(Self(proxy))
    }
    /// Count matching stored slang entries using fresh native state.
    ///
    /// An unavailable acknowledgment can follow a committed increment; this
    /// operation has no idempotency token and a caller retry can count again.
    pub async fn record_slang_usage(
        &self,
        request: SlangUsageRequest,
    ) -> tinybus::Result<ContentUsageOutcome> {
        self.0
            .call_confidential("RecordSlangUsage", (request,))
            .await
    }
    /// Count the first stored exact media match using fresh native state.
    ///
    /// An unavailable acknowledgment can follow a committed increment; this
    /// operation has no idempotency token and must not be retried as a no-op.
    pub async fn record_meme_usage(
        &self,
        request: MemeUsageRequest,
    ) -> tinybus::Result<ContentUsageOutcome> {
        self.0
            .call_confidential("RecordMemeUsage", (request,))
            .await
    }
    /// Remove all stored entries with the exact region and title.
    pub async fn remove_meme(
        &self,
        request: RemoveMemeRequest,
    ) -> tinybus::Result<ContentUsageOutcome> {
        self.0.call_confidential("RemoveMeme", (request,)).await
    }
}
