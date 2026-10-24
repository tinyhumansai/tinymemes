use super::*;
use tinybus::{Connection, broker::Broker, module::ModuleHost, transport::memory::MemoryBus};
use tinymemes_bus::{ApplyRequest, ContentError, ContentOperation, ContentState};
use tinymemes_client::StateClient;

#[tokio::test]
async fn compiled_abi_returns_typed_content_and_conflicts_without_retaining_caller_state() {
    let bus = MemoryBus::new();
    let broker = Broker::new();
    broker.clone().spawn(bus.clone());
    let _storage = crate::test_support::serve_storage(&bus, "tenant:compiled-module-test")
        .await
        .unwrap();
    let host = ModuleHost::new(broker);
    let manifest = exports::linked_module().unwrap();
    host.attach_linked_with_config(manifest, serde_json::Value::Null)
        .unwrap();
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let client = StateClient::new(&caller).unwrap();
    assert_eq!(format!("{client:?}"), "StateClient { .. }");
    let request = |id: &str| ApplyRequest {
        state: ContentState::default(),
        expected_revision: 0,
        operation: ContentOperation::MarkRemixed {
            reply_id: id.into(),
        },
    };
    let first = client.apply(request("first")).await.unwrap().unwrap();
    let second = client.apply(request("second")).await.unwrap().unwrap();
    assert_eq!(first.remixed, ["first"]);
    assert_eq!(second.remixed, ["second"]);
    assert_eq!(first.revision, 1);
    let mut conflicting = request("third");
    conflicting.expected_revision = 9;
    assert_eq!(
        client.apply(conflicting).await.unwrap(),
        Err(ContentError::RevisionConflict {
            expected: 9,
            actual: 0
        })
    );
    assert_eq!(
        client.apply(request("first")).await.unwrap().unwrap(),
        first
    );
    assert!(
        client
            .clone()
            .apply(request("clone"))
            .await
            .unwrap()
            .is_ok()
    );
}

#[tokio::test]
async fn malformed_calls_are_refused_without_echoing_the_payload() {
    use tinybus::service::Interface;
    let error = ContentService
        .call(
            &"Apply".try_into().unwrap(),
            serde_json::json!([{"private-payload":"secret"}]),
        )
        .await
        .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("invalid content request"));
    assert!(!text.contains("secret"));
    let error = ContentService
        .call(&"Unknown".try_into().unwrap(), serde_json::Value::Null)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unknown content operation"));
}
