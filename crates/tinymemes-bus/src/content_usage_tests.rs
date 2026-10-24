use super::*;

#[test]
fn catalog_arguments_cannot_supply_native_selectors_or_snapshots() {
    let good = serde_json::json!({"region":"gb","text":"hello"});
    assert!(serde_json::from_value::<SlangUsageRequest>(good.clone()).is_ok());
    for selector in [
        "scope",
        "collection",
        "document_id",
        "state",
        "expected_revision",
        "destination",
        "storage_url",
    ] {
        let mut request = good.clone();
        request[selector] = serde_json::json!("not-authority");
        assert!(serde_json::from_value::<SlangUsageRequest>(request).is_err());
    }
    assert!(
        serde_json::from_value::<MemeUsageRequest>(
            serde_json::json!({"media_url":"media","url":"endpoint"})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<RemoveMemeRequest>(
            serde_json::json!({"region":"gb","title":"title","verified":true})
        )
        .is_err()
    );
}

#[test]
fn catalog_storage_categories_have_no_private_backend_prose() {
    let encoded = serde_json::to_value(ContentUsageFailure::Storage(
        crate::MarkerStorageKind::Fenced,
    ))
    .unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({"kind":"storage","detail":"fenced"})
    );
}
