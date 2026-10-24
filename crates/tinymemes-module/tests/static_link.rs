//! The `static-link` build exports `linked_module()` and the host admits it
//! in-process, attesting it as host code (the executable's digest).

use std::sync::Arc;

use sha2::{Digest, Sha256};
use tinybus::Connection;
use tinybus::broker::Broker;
use tinybus::module::{ModuleHost, ModuleState};
use tinybus::transport::memory::MemoryBus;
use tinymemes_bus::{ApplyRequest, ContentOperation, ContentState, RemixRequest, STATE_INTERFACE};
use tinymemes_client::{RemixClient, StateClient};
use tinymemes_module::{BUS_NAME, linked_module};
use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_busproxy::{PerCallerEndpoint, StorageServer};
use tinystoragedrivers_testkit::MemoryBackend;
use tinystoragedrivers_types::Scope;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linked_module_is_admitted_and_attested_as_the_host_executable() {
    let bus = MemoryBus::new();
    let broker = Broker::new();
    broker.clone().spawn(bus.clone());
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:static-link-test").unwrap())
        .unwrap();
    let storage_host = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let endpoint = PerCallerEndpoint::new(move |_sender| Some(StorageServer::new(storage.clone())));
    let _served_storage = tinystoragedrivers_busproxy::serve(&storage_host, Arc::new(endpoint))
        .await
        .unwrap();
    let host = ModuleHost::new(broker);

    let module = linked_module().expect("linked manifest is valid");
    let info = host
        .attach_linked_with_config(module, serde_json::json!({}))
        .expect("linked tinymemes is admitted");
    assert_eq!(info.manifest.bus_name.as_ref() as &str, BUS_NAME);
    assert_eq!(BUS_NAME, STATE_INTERFACE);
    assert!(matches!(
        info.state,
        ModuleState::Initializing | ModuleState::Resolved
    ));

    let client = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let attestation = client
        .attestation(info.manifest.bus_name.clone())
        .await
        .unwrap()
        .expect("linked module is attested as host code");
    let exe = std::fs::read(std::env::current_exe().unwrap()).unwrap();
    let hex = Sha256::digest(exe)
        .iter()
        .fold(String::new(), |mut out, b| {
            use std::fmt::Write as _;
            let _ = write!(out, "{b:02x}");
            out
        });
    assert_eq!(attestation.sha256, hex);

    // The linked module serves its content operation...
    let state = StateClient::new(&client).unwrap();
    let mut applied = None;
    for _ in 0..50 {
        match state
            .apply(ApplyRequest {
                state: ContentState::default(),
                expected_revision: 0,
                operation: ContentOperation::MarkRemixed {
                    reply_id: "linked".into(),
                },
            })
            .await
        {
            Ok(result) => {
                applied = Some(result);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
    assert_eq!(applied.unwrap().unwrap().remixed, ["linked"]);

    // ...and a disabled remix is a pass-through that needs no host callback.
    let remix = RemixClient::new(&client).unwrap();
    let request: RemixRequest =
        serde_json::from_str(r#"{"conversation":[],"reply":"hi","enabled":false}"#).unwrap();
    let reply = remix.remix(request).await.unwrap();
    assert!(matches!(reply, Ok(r) if r.reply == "hi" && !r.remixed));
}
