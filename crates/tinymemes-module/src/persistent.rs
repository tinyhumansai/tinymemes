//! Marker operations over the module's host-bound document port.
use std::collections::HashSet;
use std::sync::Arc;
use tinymemes_bus::storage::MARKERS_COLLECTION;
use tinymemes_bus::{
    MARKER_DOCUMENT_KEY, MARKER_INTERFACE, MarkRemixedReply, MarkerFailure, MarkerRequest,
    MarkerStorageKind,
};
use tinystoragedrivers_api::DocumentStore;
use tinystoragedrivers_types::{ErrorKind, Precondition, StorageError};

const MAX_ATTEMPTS: usize = 8;

/// The marker document contains no learned content or research claims.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Markers {
    format_version: u64,
    // Decimal text also preserves unsigned revisions through native BSON.
    business_revision: String,
    remixed: Vec<String>,
}
impl Markers {
    fn empty() -> Self {
        Self {
            format_version: 1,
            business_revision: "0".into(),
            remixed: Vec::new(),
        }
    }
    fn decode(value: serde_json::Value) -> Result<Self, MarkerFailure> {
        let markers: Self =
            serde_json::from_value(value).map_err(|_| MarkerFailure::CorruptState)?;
        markers.revision()?;
        if markers.format_version != 1
            || markers.remixed.iter().collect::<HashSet<_>>().len() != markers.remixed.len()
        {
            return Err(MarkerFailure::CorruptState);
        }
        Ok(markers)
    }
    fn revision(&self) -> Result<u64, MarkerFailure> {
        let revision = self
            .business_revision
            .parse::<u64>()
            .map_err(|_| MarkerFailure::CorruptState)?;
        if revision.to_string() != self.business_revision {
            return Err(MarkerFailure::CorruptState);
        }
        Ok(revision)
    }
}

/// The contract's mirror of a native storage class (same wire form).
pub(crate) fn marker_kind(kind: ErrorKind) -> MarkerStorageKind {
    match kind {
        ErrorKind::NotFound => MarkerStorageKind::NotFound,
        ErrorKind::Conflict => MarkerStorageKind::Conflict,
        ErrorKind::AlreadyExists => MarkerStorageKind::AlreadyExists,
        ErrorKind::Unsupported(_) => MarkerStorageKind::Unsupported,
        ErrorKind::InvalidInput => MarkerStorageKind::InvalidInput,
        ErrorKind::Serialization => MarkerStorageKind::Serialization,
        ErrorKind::Crypto => MarkerStorageKind::Crypto,
        ErrorKind::Unavailable => MarkerStorageKind::Unavailable,
        ErrorKind::Backend => MarkerStorageKind::Backend,
        _ => MarkerStorageKind::Backend,
    }
}

fn storage_failure(error: StorageError) -> MarkerFailure {
    MarkerFailure::Storage(marker_kind(error.kind()))
}

pub(crate) struct PersistentService {
    pub(crate) documents: Arc<dyn DocumentStore>,
}
fn failure(error: MarkerFailure) -> tinybus::Result<serde_json::Value> {
    Ok(serde_json::to_value(Result::<(), _>::Err(error))?)
}
#[async_trait::async_trait]
impl tinybus::Interface for PersistentService {
    fn name(&self) -> tinybus::InterfaceName {
        MARKER_INTERFACE.try_into().unwrap()
    }
    fn members(&self) -> Vec<tinybus::MemberName> {
        ["MarkRemixed", "WasRemixed"]
            .into_iter()
            .map(|name| name.try_into().unwrap())
            .collect()
    }
    fn requires_confidential(&self, _: &tinybus::MemberName) -> bool {
        true
    }
    async fn call(
        &self,
        _: &tinybus::MemberName,
        _: serde_json::Value,
    ) -> tinybus::Result<serde_json::Value> {
        Err(tinybus::Error::failed(
            "authenticated confidential caller required",
        ))
    }
    async fn call_with_context(
        &self,
        member: &tinybus::MemberName,
        args: serde_json::Value,
        context: &tinybus::CallContext,
    ) -> tinybus::Result<serde_json::Value> {
        // The host has already bound this module's storage handle. Continue to
        // require the authenticated confidential caller before accessing it.
        context
            .authenticated_sender()
            .filter(|_| context.is_confidential())
            .ok_or_else(|| tinybus::Error::failed("authenticated confidential caller required"))?;
        if !matches!(member.as_str(), "MarkRemixed" | "WasRemixed") {
            return Err(tinybus::Error::failed("unknown marker operation"));
        }
        let (request,): (MarkerRequest,) = serde_json::from_value(args)
            .map_err(|_| tinybus::Error::failed("invalid marker request"))?;
        for _ in 0..MAX_ATTEMPTS {
            let document = match self
                .documents
                .get(MARKERS_COLLECTION, MARKER_DOCUMENT_KEY)
                .await
            {
                Ok(document) => document,
                Err(error) => return failure(storage_failure(error)),
            };
            let (markers, condition) = match document {
                Some(document) => {
                    if document.id != MARKER_DOCUMENT_KEY {
                        return failure(MarkerFailure::CorruptState);
                    }
                    let markers = match Markers::decode(document.doc) {
                        Ok(markers) => markers,
                        Err(error) => return failure(error),
                    };
                    (markers, Precondition::Version(document.version))
                }
                None => (Markers::empty(), Precondition::Absent),
            };
            if member.as_str() == "WasRemixed" {
                return Ok(serde_json::to_value(Result::<_, MarkerFailure>::Ok(
                    markers.remixed.contains(&request.reply_id),
                ))?);
            }
            let revision = match markers.revision() {
                Ok(revision) => revision,
                Err(error) => return failure(error),
            };
            // The owner reducer receives only freshly read native marker state.
            let next = match tinymemes::state::apply(tinymemes_bus::ApplyRequest {
                state: tinymemes_bus::ContentState {
                    revision,
                    remixed: markers.remixed,
                    ..Default::default()
                },
                expected_revision: revision,
                operation: tinymemes_bus::ContentOperation::MarkRemixed {
                    reply_id: request.reply_id.clone(),
                },
            }) {
                Ok(next) => next,
                Err(error) => return failure(MarkerFailure::Content(error)),
            };
            let reply = MarkRemixedReply {
                changed: next.revision != revision,
                business_revision: next.revision,
            };
            if !reply.changed {
                return Ok(serde_json::to_value(Result::<_, MarkerFailure>::Ok(reply))?);
            }
            let doc = serde_json::to_value(Markers {
                format_version: 1,
                business_revision: next.revision.to_string(),
                remixed: next.remixed,
            })?;
            match self
                .documents
                .put(MARKERS_COLLECTION, MARKER_DOCUMENT_KEY, doc, condition)
                .await
            {
                Ok(_) => return Ok(serde_json::to_value(Result::<_, MarkerFailure>::Ok(reply))?),
                Err(error) if error.kind() == ErrorKind::Conflict => continue,
                Err(error) => return failure(storage_failure(error)),
            }
        }
        failure(MarkerFailure::ConflictExhausted)
    }
}

#[cfg(test)]
#[path = "persistent_tests.rs"]
mod tests;
