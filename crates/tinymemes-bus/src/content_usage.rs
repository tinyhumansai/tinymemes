//! Operation-only catalog updates. Native catalogs and admission are not inputs.
use serde::{Deserialize, Serialize};

/// Persistent catalog interface.
pub const CONTENT_USAGE_INTERFACE: &str = "ai.tinyhumans.tinymemes.ContentUsage1";
/// Persistent catalog object path.
pub const CONTENT_USAGE_PATH: &str = "/ai/tinyhumans/tinymemes/ContentUsage";
/// Private catalog document key, separate from remix markers.
pub const CONTENT_DOCUMENT_KEY: &str = "content-state";

/// Usage text and its business region, not an authority selector.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlangUsageRequest {
    /// Region to match inside the stored catalog.
    pub region: String,
    /// Text in which existing terms are counted.
    pub text: String,
}
/// An existing media identifier, never a storage or callback endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemeUsageRequest {
    /// Exact media URL to match inside the stored catalog.
    pub media_url: String,
}
/// Business identifiers selecting stored entries for removal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoveMemeRequest {
    /// Region to match inside the stored catalog.
    pub region: String,
    /// Exact title to remove in that region.
    pub title: String,
}
/// Result of a native catalog mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentMutationReply {
    /// Whether the owning reducer changed the catalog.
    pub changed: bool,
    /// Business journal revision, distinct from native document Version.
    pub business_revision: u64,
}
/// Content-free failures; backend source chains remain host-local.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum ContentUsageFailure {
    /// Native storage refused the operation.
    Storage(tinystoragedrivers_bus::StorageKind),
    /// Persisted catalog is malformed or uses an unsupported format.
    CorruptState,
    /// The existing owning reducer refused the operation.
    Content(crate::ContentError),
    /// Eight conditional write attempts conflicted.
    ConflictExhausted,
    /// The persistent implementation is unavailable.
    Unavailable,
}
/// Business outcome, separate from transport admission.
pub type ContentUsageOutcome = Result<ContentMutationReply, ContentUsageFailure>;

/// A client carrying only an already-authorized callback proxy.
#[cfg(feature = "host-callback")]
#[derive(Clone, Debug)]
pub struct ContentUsageClient(tinybus::module::HostCallbackProxy);
#[cfg(feature = "host-callback")]
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

#[cfg(test)]
#[path = "content_usage_tests.rs"]
mod tests;
