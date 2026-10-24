//! Operation-only persistent marker contract. Native authority is not wire data.
use serde::{Deserialize, Serialize};

/// Persistent marker interface.
pub const MARKER_INTERFACE: &str = "ai.tinyhumans.tinymemes.RemixMarkers1";
/// Persistent marker object path.
pub const MARKER_PATH: &str = "/ai/tinyhumans/tinymemes/RemixMarkers";
/// Fixed private document callback path used by the module.
pub const MARKER_DOCUMENT_PATH: &str = "/ai/tinyhumans/tinymemes/Documents";
/// Fixed document key; callers cannot select documents or collections.
pub const MARKER_DOCUMENT_KEY: &str = "remix-state";

/// Business identifier, never an authority selector or persisted snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerRequest {
    /// Reply whose marker is read or recorded.
    pub reply_id: String,
}
/// Result of a committed marker operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkRemixedReply {
    /// Whether this operation added a marker.
    pub changed: bool,
    /// Owning content revision, distinct from the native document version.
    pub business_revision: u64,
}
/// Content-free class of a storage refusal. Mirrors the storage contract's
/// kinds by name so this crate links no storage crate; the module maps them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerStorageKind {
    /// Required resource absent.
    NotFound,
    /// Atomic condition failed.
    Conflict,
    /// Unique resource exists.
    AlreadyExists,
    /// Native operation unsupported.
    Unsupported,
    /// Native input rejected.
    InvalidInput,
    /// Serialization failed.
    Serialization,
    /// Cryptography failed.
    Crypto,
    /// Service unavailable.
    Unavailable,
    /// Driver failed.
    Backend,
    /// Native lease guard no longer holds.
    Fenced,
    /// Scope generation retired or unissued, or scope reset pending.
    ScopeGenerationRejected,
}

/// Content-free failures. Native source chains never cross the ABI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum MarkerFailure {
    /// Native storage refused the operation.
    Storage(MarkerStorageKind),
    /// Persisted state cannot be decoded or validated.
    CorruptState,
    /// Owning content operation refused.
    Content(crate::ContentError),
    /// Bounded native conflict retries were exhausted.
    ConflictExhausted,
    /// Persistent implementation is unavailable.
    Unavailable,
}
/// Persistent operation outcome, separate from transport admission.
pub type MarkerOutcome<T> = Result<T, MarkerFailure>;

#[cfg(test)]
#[path = "persistent_tests.rs"]
mod tests;
