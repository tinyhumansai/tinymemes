//! Pure storage declarations and legacy value conversions for TinyMemes.
//!
//! The engine asks the host to load and save one JSON text per [`IndexKind`]
//! (`LoadIndex` / `SaveIndex`). The host keeps those texts in `memes.state`, one
//! document per kind, as the *parsed*
//! text (v1 kept the text as a string in `{"json": "<text>"}` in
//! `tinymemes_documents`). [`state_body`] and [`state_text`] are the two
//! directions of that mapping. The module performs its own migration when it
//! opens its already-bound storage handle.
use crate::IndexKind;
use serde_json::{Value, json};
use tinystoragedrivers_types::CollectionSpec;

/// The v2 collection holding the learned-state documents.
pub const STATE_COLLECTION: &str = "memes.state";
/// Owner-owned collection for delivered-remix markers.
pub const MARKERS_COLLECTION: &str = "memes.remix_markers";
/// Owner-owned collection for the learned slang and meme catalog document
/// the content-usage operations update.
pub const CONTENT_COLLECTION: &str = "memes.content";
/// Legacy host-owned collection written through the previous marker API.
pub const LEGACY_MARKERS_COLLECTION: &str = "remix_markers";
/// The v1 collection the host kept them in, one `{"json": "<text>"}` body each.
pub const LEGACY_STATE_COLLECTION: &str = "tinymemes_documents";
/// Shared owner-local migration progress collection.
pub const MIGRATIONS_COLLECTION: &str = "tsd.migrations";
/// Shared leases collection used to serialize this owner's open migration.
pub const LEASES_COLLECTION: &str = "tsd.leases";
/// Stable owner name in the migration progress key.
pub const OWNER: &str = "memes";
/// Stable migration step id and progress-key component.
pub const STATE_STEP: &str = "state_v2";
/// Stable migration step id for the previous marker collection.
pub const MARKERS_STEP: &str = "markers_v1";
/// The target schema version written by the state migration.
pub const STATE_VERSION: u32 = 2;

/// Body field holding the remixed ids (a body must be an object, the ring is an
/// array).
const IDS: &str = "ids";
/// Body field holding text that is not the shape its kind expects, kept
/// verbatim so a migration never destroys it. The module treats such a
/// document as a fresh start, as it did before.
const UNPARSED: &str = "unparsed";

/// Declare the collections used by the memes owner and its state migration.
///
/// This is a pure declaration for the host to grant and ensure. It does not
/// compose a cross-owner migration registry or open a backend.
#[must_use]
pub fn collections() -> Vec<CollectionSpec> {
    vec![
        CollectionSpec::new(STATE_COLLECTION),
        CollectionSpec::new(MARKERS_COLLECTION),
        CollectionSpec::new(CONTENT_COLLECTION),
        CollectionSpec::new(LEGACY_MARKERS_COLLECTION),
        CollectionSpec::new(LEGACY_STATE_COLLECTION),
        CollectionSpec::new(MIGRATIONS_COLLECTION),
        CollectionSpec::new(LEASES_COLLECTION),
    ]
}

/// The document id of `kind` (unchanged from v1; a plain word is its own
/// [`tinystoragedrivers_types::Key`]).
#[must_use]
pub fn state_id(kind: IndexKind) -> &'static str {
    match kind {
        IndexKind::Slang => "slang",
        IndexKind::Memes => "memes",
        IndexKind::Remixed => "remixed",
    }
}

/// The v2 body for the text the module saved for `kind`.
///
/// A JSON object is the body; the remixed ring (an array of ids) becomes
/// `{"ids": [...]}`. Anything else, including an object whose own `unparsed` field is a string,
/// is kept as `{"unparsed": "<text>"}`.
#[must_use]
pub fn state_body(kind: IndexKind, text: &str) -> Value {
    body_for(state_id(kind), text)
}

/// The text the module loads back from a v2 body: the inverse of [`state_body`].
#[must_use]
pub fn state_text(kind: IndexKind, body: &Value) -> String {
    if let Some(text) = body.get(UNPARSED).and_then(Value::as_str) {
        return text.to_owned();
    }
    match (kind, body.get(IDS)) {
        (IndexKind::Remixed, Some(ids)) => ids.to_string(),
        _ => body.to_string(),
    }
}

fn body_for(id: &str, text: &str) -> Value {
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Array(ids)) if id == state_id(IndexKind::Remixed) => json!({ IDS: ids }),
        // An object carrying a string `unparsed` field would be read back as the
        // fallback marker by `state_text`, so it is wrapped like any other text.
        Ok(Value::Object(fields))
            if id != state_id(IndexKind::Remixed)
                && !fields.get(UNPARSED).is_some_and(Value::is_string) =>
        {
            Value::Object(fields)
        }
        _ => json!({ UNPARSED: text }),
    }
}

/// Convert one v1 (`{"json": "<text>"}`) body to the v2 body.
///
/// The caller preserves the source id and expiry while writing the target.
#[must_use]
pub fn migrate_state_body(id: &str, legacy: &Value) -> Value {
    match legacy.get("json").and_then(Value::as_str) {
        Some(text) => body_for(id, text),
        None => json!({ UNPARSED: legacy.to_string() }),
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod tests;
