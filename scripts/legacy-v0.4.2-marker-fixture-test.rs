
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn generate_legacy_public_marker_fixture() {
    let backend = MemoryStorage::new();
    let scoped = backend
        .for_scope(&Scope::new("legacy-fixture").unwrap())
        .unwrap();
    let fixture = Fixture::new(scoped.documents().clone()).await;
    let outcome = fixture
        .client()
        .mark_remixed(request("old-release-reply"))
        .await
        .unwrap()
        .unwrap();
    assert!(outcome.changed);
    let record = scoped
        .documents()
        .get(COLLECTION, MARKER_DOCUMENT_KEY)
        .await
        .unwrap()
        .unwrap();
    let artifact = serde_json::json!({
        "writer_release": "tinymemes-v0.4.2",
        "storage_release": "tinystoragedrivers-v1.6.0",
        "collection": COLLECTION,
        "id": MARKER_DOCUMENT_KEY,
        "schema": record.meta.schema,
        "expires_ms": record.meta.expires_ms,
        "body": record.doc,
    });
    std::fs::write(
        std::env::var_os("FIXTURE_OUTPUT").expect("FIXTURE_OUTPUT is required"),
        serde_json::to_vec_pretty(&artifact).unwrap(),
    )
    .unwrap();
}
