//! Catalog-usage operations against the real compiled module. Included from
//! `compiled_remix.rs` so it shares the loader and storage-host fixtures.
use super::*;
use serde_json::json;
use tinymemes_bus::storage::{CONTENT_COLLECTION, MARKERS_COLLECTION};
use tinymemes_bus::{
    CONTENT_DOCUMENT_KEY, CONTENT_USAGE_INTERFACE, CONTENT_USAGE_PATH, ContentError,
    ContentMutationReply, ContentUsageFailure, ContentUsageOutcome, Intent, LearnedMeme,
    LearnedTerm, MarkerStorageKind, MemeSearchRecord, MemeSnapshot, MemeUsageRequest, Origin,
    RemoveMemeRequest, SearchRecord, SlangSnapshot, SlangUsageRequest, Tier,
};
use tinymemes_client::ContentUsageClient;

fn content_proxy(host: &TestStorageHost) -> tinybus::module::HostCallbackProxy {
    let binding = host.binding.as_ref().expect("module callback binding");
    let peer = binding.guard().module_peer().expect("loaded module peer");
    binding
        .proxy(peer.as_str(), CONTENT_USAGE_PATH, CONTENT_USAGE_INTERFACE)
        .unwrap()
}
fn content_client(host: &TestStorageHost) -> ContentUsageClient {
    ContentUsageClient::new(content_proxy(host)).unwrap()
}
fn slang_request() -> SlangUsageRequest {
    SlangUsageRequest {
        region: "gb".into(),
        text: "HELLO mate!".into(),
    }
}
fn meme_request() -> MemeUsageRequest {
    MemeUsageRequest {
        media_url: "https://example.invalid/media.gif".into(),
    }
}
fn remove_request() -> RemoveMemeRequest {
    RemoveMemeRequest {
        region: "gb".into(),
        title: "wave".into(),
    }
}
/// A full catalog with legal duplicates and complete histories.
fn catalog(revision: u64) -> serde_json::Value {
    let term = LearnedTerm {
        region: "gb".into(),
        term: "mate".into(),
        meaning: "friend".into(),
        min_tier: Tier::Light,
        intents: vec![Intent::Banter],
        sources: vec!["https://example.invalid/source".into()],
        seen: 7,
        used: 3,
        origin: Origin::Web,
        verified: Some(0.9),
    };
    let mut other_term = term.clone();
    other_term.region = "au".into();
    let meme = LearnedMeme {
        region: "gb".into(),
        title: "wave".into(),
        meaning: "hello".into(),
        url: meme_request().media_url,
        page_url: "https://example.invalid/page".into(),
        intents: vec![Intent::Gratitude],
        tags: vec!["wave".into()],
        rating: "g".into(),
        verified: 0.8,
        used: 4,
        added_at: 42,
    };
    let mut other_meme = meme.clone();
    other_meme.region = "au".into();
    json!({
        "format_version": 1, "business_revision": revision.to_string(),
        "slang": SlangSnapshot {
            terms: vec![term.clone(), term, other_term],
            searches: vec![SearchRecord { region: "gb".into(), intent: Intent::Banter, query: "old query".into(), at: 11, accepted: 2 }],
        },
        "memes": MemeSnapshot {
            memes: vec![meme.clone(), meme, other_meme],
            searches: vec![MemeSearchRecord { region: "gb".into(), intent: Intent::Gratitude, query: "old meme query".into(), at: 12, added: 2 }],
        },
    })
}
async fn seed(storage: &Storage, value: serde_json::Value) -> tinystoragedrivers_types::Version {
    storage
        .documents()
        .put(
            CONTENT_COLLECTION,
            CONTENT_DOCUMENT_KEY,
            value,
            Precondition::Absent,
        )
        .await
        .unwrap()
}
async fn stored(
    storage: &Storage,
) -> Option<tinystoragedrivers_types::Versioned<serde_json::Value>> {
    storage
        .documents()
        .get(CONTENT_COLLECTION, CONTENT_DOCUMENT_KEY)
        .await
        .unwrap()
}
fn reply(changed: bool, business_revision: u64) -> ContentMutationReply {
    ContentMutationReply {
        changed,
        business_revision,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_usage_and_removal_preserve_full_metadata_markers_and_sibling_scopes() {
    let backend = MemoryBackend::new();
    let scoped = backend
        .for_scope(&Scope::new("tenant:catalog-alice").unwrap())
        .unwrap();
    let sibling = backend
        .for_scope(&Scope::new("tenant:catalog-bob").unwrap())
        .unwrap();
    let original = catalog(9);
    seed(&scoped, original.clone()).await;
    let sibling_version = seed(&sibling, original.clone()).await;
    let marker = json!({"format_version":1,"business_revision":"4","remixed":["reply"]});
    scoped
        .documents()
        .put(
            MARKERS_COLLECTION,
            tinymemes_bus::MARKER_DOCUMENT_KEY,
            marker.clone(),
            Precondition::Absent,
        )
        .await
        .unwrap();
    let marker_version = scoped
        .documents()
        .get(MARKERS_COLLECTION, tinymemes_bus::MARKER_DOCUMENT_KEY)
        .await
        .unwrap()
        .unwrap()
        .version;
    let (_host, _caller, _task, _dir, host) = load_with_storage(scoped.clone()).await;
    let client = content_client(&host);

    assert_eq!(
        client
            .record_slang_usage(slang_request())
            .await
            .unwrap()
            .unwrap(),
        reply(true, 10)
    );
    let mut expected = original.clone();
    expected["business_revision"] = json!("10");
    expected["slang"]["terms"][0]["used"] = json!(4);
    expected["slang"]["terms"][1]["used"] = json!(4);
    assert_eq!(stored(&scoped).await.unwrap().doc, expected);

    assert_eq!(
        client
            .record_meme_usage(meme_request())
            .await
            .unwrap()
            .unwrap(),
        reply(true, 11)
    );
    expected["business_revision"] = json!("11");
    expected["memes"]["memes"][0]["used"] = json!(5);
    assert_eq!(stored(&scoped).await.unwrap().doc, expected);

    assert_eq!(
        client.remove_meme(remove_request()).await.unwrap().unwrap(),
        reply(true, 12)
    );
    expected["business_revision"] = json!("12");
    expected["memes"]["memes"] = json!([expected["memes"]["memes"][2].clone()]);
    let after = stored(&scoped).await.unwrap();
    assert_eq!(after.doc, expected);

    // Removing again changes nothing and performs no write.
    assert_eq!(
        client.remove_meme(remove_request()).await.unwrap().unwrap(),
        reply(false, 12)
    );
    assert_eq!(stored(&scoped).await.unwrap().version, after.version);

    let untouched = stored(&sibling).await.unwrap();
    assert_eq!(untouched.version, sibling_version);
    assert_eq!(untouched.doc, original);
    let untouched = scoped
        .documents()
        .get(MARKERS_COLLECTION, tinymemes_bus::MARKER_DOCUMENT_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(untouched.version, marker_version);
    assert_eq!(untouched.doc, marker);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absent_catalog_is_an_empty_noop_without_native_write() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:catalog-absent").unwrap())
        .unwrap();
    let (_host, _caller, _task, _dir, host) = load_with_storage(storage.clone()).await;
    let client = content_client(&host);
    for result in [
        client.record_slang_usage(slang_request()).await,
        client.record_meme_usage(meme_request()).await,
        client.remove_meme(remove_request()).await,
    ] {
        assert_eq!(result.unwrap().unwrap(), reply(false, 0));
    }
    assert!(stored(&storage).await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_full_catalog_never_defaults_or_overwrites_native_data() {
    let mut unknown_term = catalog(0);
    unknown_term["slang"]["terms"][0]["unknown"] = json!(true);
    let mut missing_optional = catalog(0);
    missing_optional["slang"]["terms"][0]
        .as_object_mut()
        .unwrap()
        .remove("verified");
    let mut unknown_history = catalog(0);
    unknown_history["memes"]["searches"][0]["proof"] = json!(true);
    let mut unknown_top = catalog(0);
    unknown_top["scope"] = json!("victim");
    let mut revision = catalog(0);
    revision["business_revision"] = json!("01");
    let mut format = catalog(0);
    format["format_version"] = json!(2);
    for (index, original) in [
        unknown_term,
        missing_optional,
        unknown_history,
        unknown_top,
        revision,
        format,
        json!({}),
    ]
    .into_iter()
    .enumerate()
    {
        let storage = MemoryBackend::new()
            .for_scope(&Scope::new(&format!("tenant:catalog-corrupt-{index}")).unwrap())
            .unwrap();
        let version = seed(&storage, original.clone()).await;
        let (_host, _caller, _task, _dir, host) = load_with_storage(storage.clone()).await;
        assert_eq!(
            content_client(&host)
                .record_meme_usage(meme_request())
                .await
                .unwrap(),
            Err(ContentUsageFailure::CorruptState)
        );
        let after = stored(&storage).await.unwrap();
        assert_eq!(after.version, version);
        assert_eq!(after.doc, original);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_counter_and_revision_exhaustion_refuse_without_mutation() {
    for (index, failure) in [
        ContentError::CounterExhausted,
        ContentError::RevisionExhausted,
    ]
    .into_iter()
    .enumerate()
    {
        let storage = MemoryBackend::new()
            .for_scope(&Scope::new(&format!("tenant:catalog-limit-{index}")).unwrap())
            .unwrap();
        let mut original = catalog(if index == 1 { u64::MAX } else { 0 });
        if index == 0 {
            original["memes"]["memes"][0]["used"] = json!(u32::MAX);
        }
        let version = seed(&storage, original.clone()).await;
        let (_host, _caller, _task, _dir, host) = load_with_storage(storage.clone()).await;
        assert_eq!(
            content_client(&host)
                .record_meme_usage(meme_request())
                .await
                .unwrap(),
            Err(ContentUsageFailure::Content(failure))
        );
        let after = stored(&storage).await.unwrap();
        assert_eq!(after.version, version);
        assert_eq!(after.doc, original);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_catalog_usage_rereads_conflicts_without_losing_either_count() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:catalog-concurrent").unwrap())
        .unwrap();
    seed(&storage, catalog(0)).await;
    let (_host, _caller, _task, _dir, host) = load_with_storage(storage.clone()).await;
    let a = content_client(&host);
    let b = content_client(&host);
    let (one, two) = tokio::join!(
        a.record_meme_usage(meme_request()),
        b.record_slang_usage(slang_request())
    );
    let (one, two) = (one.unwrap().unwrap(), two.unwrap().unwrap());
    assert!(one.changed && two.changed);
    assert_ne!(one.business_revision, two.business_revision);
    let mut expected = catalog(2);
    expected["memes"]["memes"][0]["used"] = json!(5);
    expected["slang"]["terms"][0]["used"] = json!(4);
    expected["slang"]["terms"][1]["used"] = json!(4);
    assert_eq!(stored(&storage).await.unwrap().doc, expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_write_retries_a_conflict_but_never_a_non_conflict_failure() {
    for (index, (kind, expected)) in [
        (ErrorKind::Conflict, Ok(reply(true, 1))),
        (
            ErrorKind::Unavailable,
            Err(ContentUsageFailure::Storage(MarkerStorageKind::Unavailable)),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let storage = MemoryBackend::new()
            .for_scope(&Scope::new(&format!("tenant:catalog-fault-{index}")).unwrap())
            .unwrap();
        seed(&storage, catalog(0)).await;
        let faults = FaultPlan::new();
        let failing = Arc::new(Failing::with_plan(
            Arc::clone(storage.documents()),
            faults.clone(),
        ));
        let decorated = replace_documents(&storage, failing);
        let (_host, _caller, _task, _dir, host) = load_with_storage(decorated).await;
        faults.fail_operation_nth("documents.put_with", 1, kind);
        let outcome: ContentUsageOutcome = content_client(&host)
            .record_meme_usage(meme_request())
            .await
            .unwrap();
        assert_eq!(outcome, expected);
        let used = stored(&storage).await.unwrap().doc["memes"]["memes"][0]["used"].clone();
        assert_eq!(used, json!(if expected.is_ok() { 5 } else { 4 }));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_catalog_calls_and_selector_arguments_are_refused_without_mutation() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:catalog-refusals").unwrap())
        .unwrap();
    let version = seed(&storage, catalog(0)).await;
    let (_host, caller, _task, _dir, host) = load_with_storage(storage.clone()).await;
    let peer = host
        .binding
        .as_ref()
        .unwrap()
        .guard()
        .module_peer()
        .unwrap();
    let plain = caller
        .proxy(peer.as_str(), CONTENT_USAGE_PATH, CONTENT_USAGE_INTERFACE)
        .unwrap();
    let error = plain
        .call::<serde_json::Value>("RecordMemeUsage", (meme_request(),))
        .await
        .unwrap_err();
    assert_eq!(error.wire_name(), tinybus::Error::CONFIDENTIALITY_REQUIRED);

    let proxy = content_proxy(&host);
    for selector in [
        "scope",
        "collection",
        "document_id",
        "state",
        "expected_revision",
        "destination",
    ] {
        let mut args = serde_json::to_value(meme_request()).unwrap();
        args[selector] = json!("victim");
        let error = proxy
            .call_confidential::<ContentUsageOutcome>("RecordMemeUsage", (args,))
            .await
            .unwrap_err();
        assert_eq!(error.wire_name(), tinybus::Error::FAILED);
    }
    assert_eq!(stored(&storage).await.unwrap().version, version);
}
