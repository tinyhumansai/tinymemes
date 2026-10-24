//! Operation-only catalog updates. Native catalogs and admission are not inputs.
use serde::{Deserialize, Serialize};

/// Persistent catalog interface.
pub const CONTENT_USAGE_INTERFACE: &str = "ai.tinyhumans.tinymemes.ContentUsage1";
/// Persistent catalog object path.
pub const CONTENT_USAGE_PATH: &str = "/ai/tinyhumans/tinymemes/ContentUsage";
/// Catalog document key. The catalog lives in its own collection,
/// [`crate::storage::CONTENT_COLLECTION`], apart from the remix markers.
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
    Storage(crate::MarkerStorageKind),
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

#[cfg(test)]
#[path = "content_usage_tests.rs"]
mod tests;
