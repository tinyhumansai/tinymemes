//! Load the separately built cdylib using only the minimal contract.
use std::sync::Arc;

use tinybus::{Connection, broker::Broker, module::ModuleHost, transport::memory::MemoryBus};
use tinymemes_bus::{ApplyRequest, ContentError, ContentOperation, ContentState};
use tinymemes_client::StateClient;
use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_busproxy::{PerCallerEndpoint, StorageServer};
use tinystoragedrivers_testkit::MemoryBackend;
use tinystoragedrivers_types::Scope;

/// Load a real cdylib and exercise its typed content operations and refusals.
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args_os()
        .nth(1)
        .ok_or("expected compiled module path")?;
    let bus = MemoryBus::new();
    let broker = Broker::new();
    broker.clone().spawn(bus.clone());
    let scope = Scope::new("tenant:compiled-state-example")?;
    let storage = MemoryBackend::new().for_scope(&scope)?;
    let storage_host = Connection::connect(bus.connect().await?).await?;
    let endpoint = PerCallerEndpoint::new(move |_sender| Some(StorageServer::new(storage.clone())));
    let _served_storage =
        tinystoragedrivers_busproxy::serve(&storage_host, Arc::new(endpoint)).await?;
    let host = ModuleHost::new(broker);
    host.load_file(std::path::Path::new(&path))?;
    let caller = Connection::connect(bus.connect().await?).await?;
    let client = StateClient::new(&caller)?;
    let request = |id: &str| ApplyRequest {
        state: ContentState::default(),
        expected_revision: 0,
        operation: ContentOperation::MarkRemixed {
            reply_id: id.into(),
        },
    };
    let first = client
        .apply(request("first"))
        .await?
        .map_err(|_| "unexpected content refusal")?;
    let second = client
        .apply(request("second"))
        .await?
        .map_err(|_| "unexpected content refusal")?;
    assert_eq!(first.remixed, ["first"]);
    assert_eq!(second.remixed, ["second"]);
    assert_eq!(first.revision, 1);
    let mut conflict = request("third");
    conflict.expected_revision = 1;
    assert_eq!(
        client.apply(conflict).await?,
        Err(ContentError::RevisionConflict {
            expected: 1,
            actual: 0
        })
    );
    // Exercise every typed operation through the separately loaded module.
    let mut state = ContentState {
        slang: tinymemes_bus::SlangSnapshot {
            terms: vec![tinymemes_bus::LearnedTerm {
                region: "IN".into(),
                term: "yaar".into(),
                meaning: "friend".into(),
                min_tier: tinymemes_bus::Tier::Light,
                intents: vec![],
                sources: vec![],
                seen: 0,
                used: 0,
                origin: tinymemes_bus::Origin::Curated,
                verified: None,
            }],
            searches: vec![],
        },
        memes: tinymemes_bus::MemeSnapshot {
            memes: vec![tinymemes_bus::LearnedMeme {
                region: "IN".into(),
                title: "Reaction".into(),
                meaning: "reaction".into(),
                url: "media".into(),
                page_url: "page".into(),
                intents: vec![tinymemes_bus::Intent::Banter],
                tags: vec![],
                rating: "g".into(),
                verified: 0.9,
                used: 0,
                added_at: 7,
            }],
            searches: vec![],
        },
        ..Default::default()
    };
    for operation in [
        ContentOperation::RecordSlangUsage {
            region: "IN".into(),
            text: "yaar yaar".into(),
        },
        ContentOperation::RecordMemeUsage {
            url: "media".into(),
        },
    ] {
        state = client
            .apply(ApplyRequest {
                expected_revision: state.revision,
                state,
                operation,
            })
            .await?
            .map_err(|_| "unexpected usage refusal")?;
    }
    assert_eq!(state.slang.terms[0].used, 1);
    assert_eq!(state.memes.memes[0].used, 1);
    assert_eq!(state.revision, 2);
    state = client
        .apply(ApplyRequest {
            expected_revision: state.revision,
            state,
            operation: ContentOperation::RemoveMeme {
                region: "IN".into(),
                title: "Reaction".into(),
            },
        })
        .await?
        .map_err(|_| "unexpected removal refusal")?;
    assert!(state.memes.memes.is_empty());
    assert_eq!(state.revision, 3);
    state.slang.terms[0].used = u32::MAX;
    assert_eq!(
        client
            .apply(ApplyRequest {
                expected_revision: state.revision,
                state,
                operation: ContentOperation::RecordSlangUsage {
                    region: "IN".into(),
                    text: "yaar".into()
                },
            })
            .await?,
        Err(ContentError::CounterExhausted)
    );
    println!(
        "compiled TinyBus content module: all operations, isolated values and typed refusals passed"
    );
    Ok(())
}
