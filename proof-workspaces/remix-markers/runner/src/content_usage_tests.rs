use super::*;
use tinystoragedrivers_bus::StorageKind;

/// Build the catalog client from this fixture's exact loaded-module binding.
fn content_client(fixture: &Fixture) -> ContentUsageClient {
    ContentUsageClient::new(
        fixture
            .binding
            .proxy(
                fixture.binding.guard().module_peer().unwrap().as_str(),
                CONTENT_USAGE_PATH,
                CONTENT_USAGE_INTERFACE,
            )
            .unwrap()
            .with_timeout(Duration::from_secs(3)),
    )
    .unwrap()
}
/// Return matching business text without native selectors.
fn slang_request() -> SlangUsageRequest {
    SlangUsageRequest {
        region: "gb".into(),
        text: "HELLO mate!".into(),
    }
}
/// Return a stored media identifier without selecting a backend route.
fn meme_request() -> MemeUsageRequest {
    MemeUsageRequest {
        media_url: "https://example.invalid/media.gif".into(),
    }
}
/// Return an exact stored region/title match for removal.
fn remove_request() -> RemoveMemeRequest {
    RemoveMemeRequest {
        region: "gb".into(),
        title: "wave".into(),
    }
}
/// Construct trusted fixture data with legal duplicates and complete histories.
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
    serde_json::json!({
        "format_version":1, "business_revision":revision.to_string(),
        "slang": SlangSnapshot {
            // Duplicates are legal classic data: both matching terms increment.
            terms: vec![term.clone(), term, other_term],
            searches: vec![SearchRecord {region:"gb".into(), intent:Intent::Banter, query:"old query".into(), at:11, accepted:2}],
        },
        "memes": MemeSnapshot {
            // Same URL is legal: usage only changes the first stored match;
            // removal deletes both same-region/title matches.
            memes: vec![meme.clone(), meme, other_meme],
            searches: vec![MemeSearchRecord {region:"gb".into(), intent:Intent::Gratitude, query:"old meme query".into(), at:12, added:2}],
        },
    })
}
/// Seed only the supplied scratch native port, conditional on document absence.
async fn seed(
    documents: &dyn DocumentStore,
    value: serde_json::Value,
) -> tinystoragedrivers_core::Version {
    documents
        .put(
            COLLECTION,
            CONTENT_DOCUMENT_KEY,
            value,
            Precondition::Absent,
        )
        .await
        .unwrap()
}
/// Read the scratch catalog together with its genuine native Version.
async fn stored(
    documents: &dyn DocumentStore,
) -> tinystoragedrivers_core::Versioned<serde_json::Value> {
    documents
        .get(COLLECTION, CONTENT_DOCUMENT_KEY)
        .await
        .unwrap()
        .unwrap()
}
/// Verify owning updates preserve complete catalogs, marker data, and sibling scopes.
async fn catalog_operations(backend: Arc<dyn StorageBackend>) {
    let scoped = backend
        .for_scope(&Scope::new("catalog-alice").unwrap())
        .unwrap();
    let sibling = backend
        .for_scope(&Scope::new("catalog-bob").unwrap())
        .unwrap();
    let original = catalog(9);
    seed(scoped.documents().as_ref(), original.clone()).await;
    let sibling_version = seed(sibling.documents().as_ref(), original.clone()).await;
    let marker = json!({"format_version":1,"business_revision":"4","remixed":["reply"]});
    let marker_version = scoped
        .documents()
        .put(
            COLLECTION,
            MARKER_DOCUMENT_KEY,
            marker.clone(),
            Precondition::Absent,
        )
        .await
        .unwrap();
    let observed = PairedReads::observed(scoped.documents().clone());
    let fixture = Fixture::new(observed.clone()).await;
    let client = content_client(&fixture);
    assert_eq!(
        client
            .record_slang_usage(slang_request())
            .await
            .unwrap()
            .unwrap(),
        ContentMutationReply {
            changed: true,
            business_revision: 10
        }
    );
    let mut expected = original;
    expected["business_revision"] = json!("10");
    expected["slang"]["terms"][0]["used"] = json!(4);
    expected["slang"]["terms"][1]["used"] = json!(4);
    assert_eq!(stored(scoped.documents().as_ref()).await.doc, expected);
    assert_eq!(
        client
            .record_meme_usage(meme_request())
            .await
            .unwrap()
            .unwrap(),
        ContentMutationReply {
            changed: true,
            business_revision: 11
        }
    );
    expected["business_revision"] = json!("11");
    expected["memes"]["memes"][0]["used"] = json!(5);
    assert_eq!(stored(scoped.documents().as_ref()).await.doc, expected);
    assert_eq!(
        client.remove_meme(remove_request()).await.unwrap().unwrap(),
        ContentMutationReply {
            changed: true,
            business_revision: 12
        }
    );
    expected["business_revision"] = json!("12");
    expected["memes"]["memes"] = json!([expected["memes"]["memes"][2].clone()]);
    let after = stored(scoped.documents().as_ref()).await;
    assert_eq!(after.doc, expected);
    assert_eq!(
        client.remove_meme(remove_request()).await.unwrap().unwrap(),
        ContentMutationReply {
            changed: false,
            business_revision: 12
        }
    );
    assert_eq!(
        stored(scoped.documents().as_ref()).await.version,
        after.version
    );
    observed.assert_calls(4, 3);
    let untouched = stored(sibling.documents().as_ref()).await;
    assert_eq!(untouched.version, sibling_version);
    assert_eq!(untouched.doc, catalog(9));
    let untouched = scoped
        .documents()
        .get(COLLECTION, MARKER_DOCUMENT_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(untouched.version, marker_version);
    assert_eq!(untouched.doc, marker);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_catalog_usage_and_removal_preserve_full_metadata_and_siblings() {
    catalog_operations(Arc::new(MemoryStorage::new())).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_catalog_usage_and_removal_preserve_full_metadata_and_siblings() {
    let directory = tempfile::tempdir().unwrap();
    catalog_operations(Arc::new(
        tinystoragedrivers_sqlite::SqliteStorage::open(directory.path().join("catalog.db"))
            .unwrap(),
    ))
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_catalog_usage_survives_actual_cold_file_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("catalog.db");
    {
        let backend = tinystoragedrivers_sqlite::SqliteStorage::open(&path).unwrap();
        let scoped = backend.for_scope(&Scope::new("cold").unwrap()).unwrap();
        seed(scoped.documents().as_ref(), catalog(0)).await;
        let fixture = Fixture::new(scoped.documents().clone()).await;
        assert_eq!(
            content_client(&fixture)
                .record_meme_usage(meme_request())
                .await
                .unwrap()
                .unwrap()
                .business_revision,
            1
        );
        fixture
            .host
            .as_ref()
            .unwrap()
            .shutdown(Duration::from_secs(3))
            .await;
    }
    assert!(path.is_file());
    let reopened = tinystoragedrivers_sqlite::SqliteStorage::open(&path).unwrap();
    let scoped = reopened.for_scope(&Scope::new("cold").unwrap()).unwrap();
    let mut expected = catalog(1);
    expected["memes"]["memes"][0]["used"] = json!(5);
    assert_eq!(stored(scoped.documents().as_ref()).await.doc, expected);
    let fixture = Fixture::new(scoped.documents().clone()).await;
    assert_eq!(
        content_client(&fixture)
            .record_meme_usage(meme_request())
            .await
            .unwrap()
            .unwrap()
            .business_revision,
        2
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn absent_catalog_is_an_empty_noop_without_native_write() {
    let backend = MemoryStorage::new();
    let scoped = backend.for_scope(&Scope::new("absent").unwrap()).unwrap();
    let observed = PairedReads::observed(scoped.documents().clone());
    let fixture = Fixture::new(observed.clone()).await;
    let client = content_client(&fixture);
    for result in [
        client.record_slang_usage(slang_request()).await,
        client.record_meme_usage(meme_request()).await,
        client.remove_meme(remove_request()).await,
    ] {
        assert_eq!(
            result.unwrap().unwrap(),
            ContentMutationReply {
                changed: false,
                business_revision: 0
            }
        );
    }
    observed.assert_calls(3, 0);
    assert!(
        scoped
            .documents()
            .get(COLLECTION, CONTENT_DOCUMENT_KEY)
            .await
            .unwrap()
            .is_none()
    );
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
        let backend = MemoryStorage::new();
        let scoped = backend
            .for_scope(&Scope::new(format!("corrupt-{index}")).unwrap())
            .unwrap();
        let version = seed(scoped.documents().as_ref(), original.clone()).await;
        let observed = PairedReads::observed(scoped.documents().clone());
        let fixture = Fixture::new(observed.clone()).await;
        assert_eq!(
            content_client(&fixture)
                .record_meme_usage(meme_request())
                .await
                .unwrap(),
            Err(ContentUsageFailure::CorruptState)
        );
        observed.assert_calls(1, 0);
        let after = stored(scoped.documents().as_ref()).await;
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
        let backend = MemoryStorage::new();
        let scoped = backend
            .for_scope(&Scope::new(format!("limit-{index}")).unwrap())
            .unwrap();
        let mut original = catalog(if index == 1 { u64::MAX } else { 0 });
        if index == 0 {
            original["memes"]["memes"][0]["used"] = json!(u32::MAX);
        }
        let version = seed(scoped.documents().as_ref(), original.clone()).await;
        let observed = PairedReads::observed(scoped.documents().clone());
        let fixture = Fixture::new(observed.clone()).await;
        let client = content_client(&fixture);
        assert_eq!(
            client.record_meme_usage(meme_request()).await.unwrap(),
            Err(ContentUsageFailure::Content(failure))
        );
        assert_eq!(
            client
                .remove_meme(RemoveMemeRequest {
                    region: "none".into(),
                    title: "none".into()
                })
                .await
                .unwrap()
                .unwrap(),
            ContentMutationReply {
                changed: false,
                business_revision: if index == 1 { u64::MAX } else { 0 }
            }
        );
        observed.assert_calls(2, 0);
        let after = stored(scoped.documents().as_ref()).await;
        assert_eq!(after.version, version);
        assert_eq!(after.doc, original);
    }
}
/// Synchronize after actual native reads and exercise genuine competing Version CAS.
async fn concurrent_usage(backend: Arc<dyn StorageBackend>, same_operation: bool) {
    let scoped = backend
        .for_scope(&Scope::new("competing").unwrap())
        .unwrap();
    seed(scoped.documents().as_ref(), catalog(0)).await;
    let paired = Arc::new(PairedReads {
        inner: scoped.documents().clone(),
        reads: std::sync::atomic::AtomicUsize::new(0),
        puts: std::sync::atomic::AtomicUsize::new(0),
        policy: None,
        barrier: Some(tokio::sync::Barrier::new(2)),
    });
    let first = Fixture::new(paired.clone()).await;
    let second = Fixture::new(paired.clone()).await;
    let a = content_client(&first);
    let b = content_client(&second);
    let (one, two) = tokio::join!(a.record_meme_usage(meme_request()), async {
        if same_operation {
            b.record_meme_usage(meme_request()).await
        } else {
            b.record_slang_usage(slang_request()).await
        }
    });
    let one = one.unwrap().unwrap();
    let two = two.unwrap().unwrap();
    assert!(one.changed && two.changed);
    assert_ne!(one.business_revision, two.business_revision);
    let mut expected = catalog(2);
    expected["memes"]["memes"][0]["used"] = json!(if same_operation { 6 } else { 5 });
    if !same_operation {
        expected["slang"]["terms"][0]["used"] = json!(4);
        expected["slang"]["terms"][1]["used"] = json!(4);
    }
    assert_eq!(stored(scoped.documents().as_ref()).await.doc, expected);
    paired.assert_calls(3, 3); // Includes a real native conditional-write conflict.
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_independent_modules_reread_conflicts_without_losing_other_usage() {
    concurrent_usage(Arc::new(MemoryStorage::new()), false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_independent_modules_count_two_nonidempotent_usage_operations() {
    let directory = tempfile::tempdir().unwrap();
    concurrent_usage(
        Arc::new(
            tinystoragedrivers_sqlite::SqliteStorage::open(directory.path().join("catalog.db"))
                .unwrap(),
        ),
        true,
    )
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_conflict_retry_is_bounded_and_nonconflicts_never_retry() {
    use tinystoragedrivers_core::ErrorKind;
    for (index, policy, expected, calls) in [
        (
            0,
            PutPolicy::Conflicts(usize::MAX),
            ContentUsageFailure::ConflictExhausted,
            8,
        ),
        (
            1,
            PutPolicy::Refuse(ErrorKind::Unavailable),
            ContentUsageFailure::Storage(StorageKind::Unavailable),
            1,
        ),
        (
            2,
            PutPolicy::Refuse(ErrorKind::Fenced),
            ContentUsageFailure::Storage(StorageKind::Fenced),
            1,
        ),
    ] {
        let backend = MemoryStorage::new();
        let scoped = backend
            .for_scope(&Scope::new(format!("fault-{index}")).unwrap())
            .unwrap();
        let version = seed(scoped.documents().as_ref(), catalog(0)).await;
        let observed = PairedReads::faulty(scoped.documents().clone(), policy);
        let fixture = Fixture::new(observed.clone()).await;
        assert_eq!(
            content_client(&fixture)
                .record_meme_usage(meme_request())
                .await
                .unwrap(),
            Err(expected)
        );
        observed.assert_calls(calls, calls);
        assert_eq!(stored(scoped.documents().as_ref()).await.version, version);
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_native_port_lost_ack_is_uncertain_and_not_retried() {
    let backend = MemoryStorage::new();
    let scoped = backend.for_scope(&Scope::new("lost-ack").unwrap()).unwrap();
    seed(scoped.documents().as_ref(), catalog(0)).await;
    let observed = PairedReads::faulty(scoped.documents().clone(), PutPolicy::LostAcknowledgment);
    let fixture = Fixture::new(observed.clone()).await;
    assert_eq!(
        content_client(&fixture)
            .record_meme_usage(meme_request())
            .await
            .unwrap(),
        Err(ContentUsageFailure::Storage(StorageKind::Unavailable))
    );
    observed.assert_calls(1, 1);
    let mut expected = catalog(1);
    expected["memes"]["memes"][0]["used"] = json!(5);
    assert_eq!(stored(scoped.documents().as_ref()).await.doc, expected);
}
/// Replace native lease metadata and verify snapshot reads do not authorize stale writes.
async fn stale_catalog(backend: Arc<dyn StorageBackend>) {
    use tinystoragedrivers_core::{Fence, Filter};
    let guard = Scope::new("cluster").unwrap();
    let scope = Scope::new("catalog-stale").unwrap();
    let metadata = backend.for_scope(&guard).unwrap();
    metadata
        .documents()
        .put("leases", "victim", json!({"epoch":1}), Precondition::Absent)
        .await
        .unwrap();
    let unguarded = backend.for_scope(&scope).unwrap();
    let version = seed(unguarded.documents().as_ref(), catalog(0)).await;
    let actor = backend
        .for_scope_fenced(
            &scope,
            &Fence::new(guard, "leases", "victim", Filter::eq("epoch", 1)),
        )
        .unwrap();
    let observed = PairedReads::observed(actor.documents().clone());
    let fixture = Fixture::new(observed.clone()).await;
    metadata
        .documents()
        .put("leases", "victim", json!({"epoch":2}), Precondition::None)
        .await
        .unwrap();
    assert_eq!(
        content_client(&fixture)
            .record_meme_usage(meme_request())
            .await
            .unwrap(),
        Err(ContentUsageFailure::Storage(StorageKind::Fenced))
    );
    observed.assert_calls(1, 1); // Snapshot read is allowed; the mutation is refused.
    let after = stored(unguarded.documents().as_ref()).await;
    assert_eq!(after.version, version);
    assert_eq!(after.doc, catalog(0));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_catalog_stale_native_actor_cannot_mutate() {
    stale_catalog(Arc::new(MemoryStorage::new())).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_catalog_stale_native_actor_cannot_mutate() {
    let directory = tempfile::tempdir().unwrap();
    stale_catalog(Arc::new(
        tinystoragedrivers_sqlite::SqliteStorage::open(directory.path().join("catalog.db"))
            .unwrap(),
    ))
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_plain_and_unbound_peers_refuse_before_native_access() {
    let backend = MemoryStorage::new();
    let scoped = backend.for_scope(&Scope::new("private").unwrap()).unwrap();
    let version = seed(scoped.documents().as_ref(), catalog(0)).await;
    let observed = PairedReads::observed(scoped.documents().clone());
    let fixture = Fixture::new(observed.clone()).await;
    let peer = fixture.binding.guard().module_peer().unwrap();
    let plain = fixture
        .attacker
        .proxy(peer.as_str(), CONTENT_USAGE_PATH, CONTENT_USAGE_INTERFACE)
        .unwrap();
    assert_admission_error(
        plain
            .call::<ContentUsageOutcome>("RecordMemeUsage", (meme_request(),))
            .await
            .unwrap_err(),
        tinybus::Error::CONFIDENTIALITY_REQUIRED,
    );
    let private = fixture
        .attacker
        .proxy(peer.as_str(), CONTENT_USAGE_PATH, CONTENT_USAGE_INTERFACE)
        .unwrap();
    assert_admission_error(
        private
            .call_confidential::<ContentUsageOutcome>("RecordMemeUsage", (meme_request(),))
            .await
            .unwrap_err(),
        tinybus::Error::NOT_ATTESTED,
    );
    observed.assert_calls(0, 0);
    assert_eq!(stored(scoped.documents().as_ref()).await.version, version);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn catalog_selectors_are_rejected_before_any_native_read_or_write() {
    let backend = MemoryStorage::new();
    let scoped = backend
        .for_scope(&Scope::new("selectors").unwrap())
        .unwrap();
    let observed = PairedReads::observed(scoped.documents().clone());
    let fixture = Fixture::new(observed.clone()).await;
    let proxy = fixture
        .binding
        .proxy(
            fixture.binding.guard().module_peer().unwrap().as_str(),
            CONTENT_USAGE_PATH,
            CONTENT_USAGE_INTERFACE,
        )
        .unwrap();
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
        assert_admission_error(
            proxy
                .call_confidential::<ContentUsageOutcome>("RecordMemeUsage", (args,))
                .await
                .unwrap_err(),
            tinybus::Error::FAILED,
        );
    }
    observed.assert_calls(0, 0);
}

/// Exercise removal and independent slang usage against one native conditional document.
async fn removal_race(backend: Arc<dyn StorageBackend>) {
    let scoped = backend
        .for_scope(&Scope::new("remove-race").unwrap())
        .unwrap();
    seed(scoped.documents().as_ref(), catalog(0)).await;
    let paired = Arc::new(PairedReads {
        inner: scoped.documents().clone(),
        reads: std::sync::atomic::AtomicUsize::new(0),
        puts: std::sync::atomic::AtomicUsize::new(0),
        policy: None,
        barrier: Some(tokio::sync::Barrier::new(2)),
    });
    let first = Fixture::new(paired.clone()).await;
    let second = Fixture::new(paired.clone()).await;
    let a = content_client(&first);
    let b = content_client(&second);
    let (removed, used) = tokio::join!(
        a.remove_meme(remove_request()),
        b.record_slang_usage(slang_request())
    );
    let removed = removed.unwrap().unwrap();
    let used = used.unwrap().unwrap();
    assert!(removed.changed && used.changed);
    assert_ne!(removed.business_revision, used.business_revision);
    let mut expected = catalog(2);
    expected["slang"]["terms"][0]["used"] = json!(4);
    expected["slang"]["terms"][1]["used"] = json!(4);
    expected["memes"]["memes"] = json!([expected["memes"]["memes"][2].clone()]);
    assert_eq!(stored(scoped.documents().as_ref()).await.doc, expected);
    paired.assert_calls(3, 3);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_native_removal_cas_preserves_competing_slang_usage() {
    removal_race(Arc::new(MemoryStorage::new())).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_native_removal_cas_preserves_competing_slang_usage() {
    let directory = tempfile::tempdir().unwrap();
    removal_race(Arc::new(
        tinystoragedrivers_sqlite::SqliteStorage::open(directory.path().join("catalog.db"))
            .unwrap(),
    ))
    .await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_catalog_instance_cannot_use_retained_previous_callback() {
    let backend = MemoryStorage::new();
    let scoped = backend
        .for_scope(&Scope::new("catalog-replacement").unwrap())
        .unwrap();
    let version = seed(scoped.documents().as_ref(), catalog(0)).await;
    let old_documents = PairedReads::observed(scoped.documents().clone());
    let mut fixture = Fixture::new(old_documents.clone()).await;
    let old_peer = fixture.binding.guard().module_peer().unwrap();
    fixture
        .host
        .as_ref()
        .unwrap()
        .shutdown(Duration::from_secs(3))
        .await;
    drop(fixture.host.take());
    let second_host = ModuleHost::new(fixture.broker.clone());
    let second_binding = second_host
        .prepare_host_callbacks(vec![endpoint(MARKER_DOCUMENT_PATH).unwrap()])
        .await
        .unwrap();
    let new_documents = PairedReads::observed(scoped.documents().clone());
    serve_collection(
        &second_binding,
        MARKER_DOCUMENT_PATH.parse().unwrap(),
        new_documents.clone(),
        COLLECTION,
    )
    .await
    .unwrap();
    second_host
        .load_file_with_host_callbacks(&fixture.artifact, json!({}), &second_binding)
        .await
        .unwrap();
    let new_peer = second_binding.guard().module_peer().unwrap();
    assert_ne!(old_peer, new_peer);
    let replay = ContentUsageClient::new(
        fixture
            .binding
            .proxy(
                new_peer.as_str(),
                CONTENT_USAGE_PATH,
                CONTENT_USAGE_INTERFACE,
            )
            .unwrap(),
    )
    .unwrap();
    assert_admission_error(
        replay.record_meme_usage(meme_request()).await.unwrap_err(),
        tinybus::Error::NOT_ATTESTED,
    );
    old_documents.assert_calls(0, 0);
    new_documents.assert_calls(0, 0);
    let fresh = ContentUsageClient::new(
        second_binding
            .proxy(
                new_peer.as_str(),
                CONTENT_USAGE_PATH,
                CONTENT_USAGE_INTERFACE,
            )
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        fresh
            .remove_meme(RemoveMemeRequest {
                region: "none".into(),
                title: "none".into()
            })
            .await
            .unwrap()
            .unwrap(),
        ContentMutationReply {
            changed: false,
            business_revision: 0
        }
    );
    new_documents.assert_calls(1, 0);
    old_documents.assert_calls(0, 0);
    let after = stored(scoped.documents().as_ref()).await;
    assert_eq!(after.version, version);
    assert_eq!(after.doc, catalog(0));
}
