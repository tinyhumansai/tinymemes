//! Catalog mutations over this request's authenticated private document port.
use tinymemes_bus::{
    CONTENT_DOCUMENT_KEY, CONTENT_USAGE_INTERFACE, ContentMutationReply, ContentOperation,
    ContentState, ContentUsageFailure, MARKER_DOCUMENT_PATH, MemeSnapshot, MemeUsageRequest,
    RemoveMemeRequest, SlangSnapshot, SlangUsageRequest,
};
use tinystoragedrivers_bus::{DocumentClient, GetRequest, Precondition, PutRequest, StorageKind};

const MAX_ATTEMPTS: usize = 8;

/// The full catalog shape is private and deliberately stricter than classic
/// snapshot deserialization. Scores and research histories are data, not grants.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    format_version: u64,
    business_revision: String,
    slang: SlangSnapshot,
    memes: MemeSnapshot,
}
impl Catalog {
    /// Represent an absent catalog without writing a default document.
    fn empty() -> Self {
        Self {
            format_version: 1,
            business_revision: "0".into(),
            slang: SlangSnapshot::default(),
            memes: MemeSnapshot::default(),
        }
    }
    /// Accept only the exact owning envelope, preserving every field and duplicate.
    fn decode(original: serde_json::Value) -> Result<Self, ContentUsageFailure> {
        let catalog: Self = serde_json::from_value(original.clone())
            .map_err(|_| ContentUsageFailure::CorruptState)?;
        catalog.revision()?;
        if catalog.format_version != 1
            || serde_json::to_value(&catalog).map_err(|_| ContentUsageFailure::CorruptState)?
                != original
        {
            // Public record structs accept legacy/default fields; this owning
            // format must not silently discard unknown or normalize missing data.
            return Err(ContentUsageFailure::CorruptState);
        }
        Ok(catalog)
    }
    /// Parse the canonical decimal business revision independently of native Version.
    fn revision(&self) -> Result<u64, ContentUsageFailure> {
        let revision = self
            .business_revision
            .parse::<u64>()
            .map_err(|_| ContentUsageFailure::CorruptState)?;
        if revision.to_string() != self.business_revision {
            return Err(ContentUsageFailure::CorruptState);
        }
        Ok(revision)
    }
}

pub(crate) struct ContentUsageService {
    pub(crate) connection: tinybus::Connection,
}
/// Encode a content-free business refusal without backend source prose.
fn failure(error: ContentUsageFailure) -> tinybus::Result<serde_json::Value> {
    Ok(serde_json::to_value(Result::<(), _>::Err(error))?)
}
/// Decode deny-unknown business arguments into the existing owning reducer operation.
fn operation(
    member: &tinybus::MemberName,
    args: serde_json::Value,
) -> tinybus::Result<ContentOperation> {
    let invalid = |_| tinybus::Error::failed("invalid catalog request");
    match member.as_str() {
        "RecordSlangUsage" => {
            let (request,): (SlangUsageRequest,) = serde_json::from_value(args).map_err(invalid)?;
            Ok(ContentOperation::RecordSlangUsage {
                region: request.region,
                text: request.text,
            })
        }
        "RecordMemeUsage" => {
            let (request,): (MemeUsageRequest,) = serde_json::from_value(args).map_err(invalid)?;
            Ok(ContentOperation::RecordMemeUsage {
                url: request.media_url,
            })
        }
        "RemoveMeme" => {
            let (request,): (RemoveMemeRequest,) = serde_json::from_value(args).map_err(invalid)?;
            Ok(ContentOperation::RemoveMeme {
                region: request.region,
                title: request.title,
            })
        }
        _ => Err(tinybus::Error::failed("unknown catalog operation")),
    }
}
#[async_trait::async_trait]
impl tinybus::Interface for ContentUsageService {
    fn name(&self) -> tinybus::InterfaceName {
        CONTENT_USAGE_INTERFACE.try_into().unwrap()
    }
    fn members(&self) -> Vec<tinybus::MemberName> {
        ["RecordSlangUsage", "RecordMemeUsage", "RemoveMeme"]
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
        // Keep this call's authenticated sender across every await and retry.
        // No module configuration, first caller cache, or argument selects it.
        let caller = context
            .authenticated_sender()
            .filter(|_| context.is_confidential())
            .ok_or_else(|| tinybus::Error::failed("authenticated confidential caller required"))?
            .clone();
        let operation = operation(member, args)?;
        let documents = DocumentClient::new(self.connection.proxy(
            caller.as_str(),
            MARKER_DOCUMENT_PATH,
            tinystoragedrivers_bus::INTERFACE,
        )?)?;
        for _ in 0..MAX_ATTEMPTS {
            let document = match documents
                .get(GetRequest {
                    id: CONTENT_DOCUMENT_KEY.into(),
                })
                .await?
            {
                Ok(document) => document,
                Err(error) => return failure(ContentUsageFailure::Storage(error.kind)),
            };
            let (catalog, condition) = match document {
                Some(document) => {
                    if document.id != CONTENT_DOCUMENT_KEY {
                        return failure(ContentUsageFailure::CorruptState);
                    }
                    let catalog = match Catalog::decode(document.doc) {
                        Ok(catalog) => catalog,
                        Err(error) => return failure(error),
                    };
                    (catalog, Precondition::Version(document.version))
                }
                None => (Catalog::empty(), Precondition::Absent),
            };
            let revision = match catalog.revision() {
                Ok(revision) => revision,
                Err(error) => return failure(error),
            };
            // Pure owner policy runs only on freshly read stored catalog data.
            // Remix markers remain in their separate document and are untouched.
            let next = match tinymemes::state::apply(tinymemes_bus::ApplyRequest {
                state: ContentState {
                    revision,
                    slang: catalog.slang,
                    memes: catalog.memes,
                    remixed: Vec::new(),
                },
                expected_revision: revision,
                operation: operation.clone(),
            }) {
                Ok(next) => next,
                Err(error) => return failure(ContentUsageFailure::Content(error)),
            };
            let reply = ContentMutationReply {
                changed: next.revision != revision,
                business_revision: next.revision,
            };
            if !reply.changed {
                return Ok(serde_json::to_value(Result::<_, ContentUsageFailure>::Ok(
                    reply,
                ))?);
            }
            let doc = serde_json::to_value(Catalog {
                format_version: 1,
                business_revision: next.revision.to_string(),
                slang: next.slang,
                memes: next.memes,
            })?;
            match documents
                .put(PutRequest {
                    id: CONTENT_DOCUMENT_KEY.into(),
                    doc,
                    precondition: condition,
                })
                .await?
            {
                Ok(_) => {
                    return Ok(serde_json::to_value(Result::<_, ContentUsageFailure>::Ok(
                        reply,
                    ))?);
                }
                Err(error) if error.kind == StorageKind::Conflict => continue,
                // Native/outer acknowledgments may be lost after commit. Only
                // explicit conditional conflicts authorize a fresh-read retry.
                Err(error) => return failure(ContentUsageFailure::Storage(error.kind)),
            }
        }
        failure(ContentUsageFailure::ConflictExhausted)
    }
}
