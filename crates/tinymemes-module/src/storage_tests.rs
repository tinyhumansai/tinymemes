use super::*;
use serde_json::json;
use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_testkit::MemoryBackend;
use tinystoragedrivers_types::{Query, Scope};

fn storage() -> Storage {
    MemoryBackend::new()
        .for_scope(&Scope::new("tenant:tinymemes-tests").unwrap())
        .unwrap()
}

async fn seed_legacy(storage: &Storage, id: &str, text: &str) -> Version {
    storage
        .documents()
        .put_with(
            LEGACY_STATE_COLLECTION,
            id,
            json!({ "json": text }),
            Precondition::Absent,
            WriteOptions::new()
                .schema(1)
                .created_ms(1_234)
                .expires_ms(8_000_000_000_000),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn opening_migrates_previous_state_before_returning() {
    let storage = storage();
    let source_version = seed_legacy(
        &storage,
        "slang",
        r#"{"terms":[{"term":"bruh"}],"searches":[]}"#,
    )
    .await;

    prepare(&storage).await.unwrap();

    let migrated = storage
        .documents()
        .get(STATE_COLLECTION, "slang")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        migrated.doc,
        json!({"terms":[{"term":"bruh"}],"searches":[]})
    );
    assert_eq!(migrated.meta.schema, STATE_VERSION);
    assert_eq!(migrated.meta.expires_ms, Some(8_000_000_000_000));
    assert_ne!(migrated.meta.created_ms, 1_234, "new targets use host time");
    assert!(
        storage
            .documents()
            .get(LEGACY_STATE_COLLECTION, "slang")
            .await
            .unwrap()
            .is_none()
    );
    let marker = storage
        .documents()
        .get_as::<Progress>(
            MIGRATIONS_COLLECTION,
            Key::new(format!("{OWNER}/{STATE_COLLECTION}/{STATE_STEP}"))
                .unwrap()
                .as_str(),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(marker.doc.done);
    assert_eq!(source_version, Version::FIRST);
}

#[tokio::test]
async fn an_interrupted_copy_is_completed_without_rewriting_the_target() {
    let storage = storage();
    let source_version = seed_legacy(&storage, "remixed", r#"["m1","m2"]"#).await;
    let body = migrate_state_body("remixed", &json!({"json": r#"["m1","m2"]"#}));
    let target_version = storage
        .documents()
        .put_with(
            STATE_COLLECTION,
            "remixed",
            body,
            Precondition::Absent,
            WriteOptions::new()
                .schema(STATE_VERSION)
                .expires_ms(8_000_000_000_000),
        )
        .await
        .unwrap();

    prepare(&storage).await.unwrap();

    assert_eq!(
        storage
            .documents()
            .get(STATE_COLLECTION, "remixed")
            .await
            .unwrap()
            .unwrap()
            .version,
        target_version
    );
    assert!(
        storage
            .documents()
            .get(LEGACY_STATE_COLLECTION, "remixed")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(source_version, Version::FIRST);
}

#[tokio::test]
async fn a_matching_body_with_different_expiry_keeps_the_legacy_source() {
    let storage = storage();
    seed_legacy(&storage, "slang", r#"{"terms":[],"searches":[]}"#).await;
    storage
        .documents()
        .put_with(
            STATE_COLLECTION,
            "slang",
            migrate_state_body("slang", &json!({ "json": r#"{"terms":[],"searches":[]}"# })),
            Precondition::Absent,
            WriteOptions::new()
                .schema(STATE_VERSION)
                .expires_ms(7_000_000_000_000),
        )
        .await
        .unwrap();

    let error = prepare(&storage).await.unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert!(
        storage
            .documents()
            .get(LEGACY_STATE_COLLECTION, "slang")
            .await
            .unwrap()
            .is_some(),
        "a target with a different absolute expiry cannot replace the source"
    );
}

#[tokio::test]
async fn conflicting_target_keeps_the_legacy_source_for_recovery() {
    let storage = storage();
    seed_legacy(&storage, "memes", r#"{"memes":[]}"#).await;
    storage
        .documents()
        .put(
            STATE_COLLECTION,
            "memes",
            json!({"memes":["newer"]}),
            Precondition::Absent,
        )
        .await
        .unwrap();

    let error = prepare(&storage).await.unwrap_err();

    assert_eq!(error.kind(), ErrorKind::Conflict);
    assert!(
        storage
            .documents()
            .get(LEGACY_STATE_COLLECTION, "memes")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        storage
            .documents()
            .query_all(MIGRATIONS_COLLECTION, &Query::all())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn opening_migrates_legacy_remix_markers_and_retries_idempotently() {
    let storage = storage();
    let body = json!({
        "format_version": 1,
        "business_revision": "1",
        "remixed": ["old-release-reply"],
    });
    let expiry = 8_000_000_000_000;
    storage
        .documents()
        .put_with(
            LEGACY_MARKERS_COLLECTION,
            "remix-state",
            body.clone(),
            Precondition::Absent,
            WriteOptions::new().schema(1).expires_ms(expiry),
        )
        .await
        .unwrap();

    prepare(&storage).await.unwrap();
    let first_target = storage
        .documents()
        .get(MARKERS_COLLECTION, "remix-state")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first_target.doc, body);
    assert_eq!(first_target.meta.schema, 1);
    assert_eq!(first_target.meta.expires_ms, Some(expiry));
    assert!(
        storage
            .documents()
            .get(LEGACY_MARKERS_COLLECTION, "remix-state")
            .await
            .unwrap()
            .is_none()
    );

    prepare(&storage).await.unwrap();
    assert_eq!(
        storage
            .documents()
            .get(MARKERS_COLLECTION, "remix-state")
            .await
            .unwrap()
            .unwrap()
            .version,
        first_target.version,
        "an already migrated marker is not rewritten on reopen"
    );
}

#[tokio::test]
async fn conflicting_marker_target_keeps_legacy_source_for_recovery() {
    let storage = storage();
    let old_body = json!({
        "format_version": 1,
        "business_revision": "1",
        "remixed": ["legacy"],
    });
    storage
        .documents()
        .put_with(
            LEGACY_MARKERS_COLLECTION,
            "remix-state",
            old_body,
            Precondition::Absent,
            WriteOptions::new().schema(1),
        )
        .await
        .unwrap();
    let conflicting = json!({
        "format_version": 1,
        "business_revision": "1",
        "remixed": ["newer"],
    });
    storage
        .documents()
        .put_with(
            MARKERS_COLLECTION,
            "remix-state",
            conflicting.clone(),
            Precondition::Absent,
            WriteOptions::new().schema(1),
        )
        .await
        .unwrap();

    assert_eq!(
        prepare(&storage).await.unwrap_err().kind(),
        ErrorKind::Conflict
    );
    assert_eq!(
        storage
            .documents()
            .get(LEGACY_MARKERS_COLLECTION, "remix-state")
            .await
            .unwrap()
            .unwrap()
            .doc["remixed"],
        json!(["legacy"])
    );
    assert_eq!(
        storage
            .documents()
            .get(MARKERS_COLLECTION, "remix-state")
            .await
            .unwrap()
            .unwrap()
            .doc,
        conflicting
    );
}

#[tokio::test]
async fn concurrent_owner_opens_serialize_and_finish_one_migration() {
    let storage = storage();
    seed_legacy(&storage, "memes", r#"{"memes":[]}"#).await;

    let (left, right) = tokio::join!(prepare(&storage), prepare(&storage));

    left.unwrap();
    right.unwrap();
    let targets = storage
        .documents()
        .query_all(STATE_COLLECTION, &Query::all())
        .await
        .unwrap();
    assert_eq!(targets.len(), 1);
    assert!(
        storage
            .documents()
            .get(LEGACY_STATE_COLLECTION, "memes")
            .await
            .unwrap()
            .is_none()
    );
}
