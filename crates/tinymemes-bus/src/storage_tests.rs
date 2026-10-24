use super::*;
use serde_json::json;

#[test]
fn collections_declare_memes_state_and_marker_storage_with_shared_runner_records() {
    let names: Vec<_> = collections()
        .into_iter()
        .map(|collection| collection.name)
        .collect();
    assert_eq!(
        names,
        [
            STATE_COLLECTION,
            MARKERS_COLLECTION,
            CONTENT_COLLECTION,
            LEGACY_MARKERS_COLLECTION,
            LEGACY_STATE_COLLECTION,
            MIGRATIONS_COLLECTION,
            LEASES_COLLECTION,
        ]
    );
}

#[test]
fn document_ids_are_the_v1_ids() {
    for (kind, expected) in [
        (IndexKind::Slang, "slang"),
        (IndexKind::Memes, "memes"),
        (IndexKind::Remixed, "remixed"),
    ] {
        assert_eq!(state_id(kind), expected);
    }
}

#[test]
fn an_object_text_becomes_the_body_itself() {
    let body = state_body(IndexKind::Slang, r#"{"terms":[],"searches":[]}"#);
    assert_eq!(body, json!({"terms": [], "searches": []}));
    let text = state_text(IndexKind::Slang, &body);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&text).unwrap(),
        body
    );
}

#[test]
fn the_remixed_ring_is_wrapped_because_a_body_is_an_object() {
    let body = state_body(IndexKind::Remixed, r#"["a","b"]"#);
    assert_eq!(body, json!({"ids": ["a", "b"]}));
    assert_eq!(state_text(IndexKind::Remixed, &body), r#"["a","b"]"#);
}

#[test]
fn text_that_is_not_the_expected_shape_is_kept_verbatim() {
    for (kind, text) in [
        (IndexKind::Slang, "not json"),
        (IndexKind::Slang, "[1,2]"),
        (IndexKind::Remixed, r#"{"x":1}"#),
    ] {
        let body = state_body(kind, text);
        assert_eq!(body, json!({ "unparsed": text }));
        assert_eq!(state_text(kind, &body), text);
    }
}

#[test]
fn a_v1_record_keeps_its_json_content_for_the_owner_migrator() {
    assert_eq!(
        migrate_state_body("slang", &json!({"json": r#"{"terms":[]}"#})),
        json!({"terms": []})
    );
    assert_eq!(
        migrate_state_body("remixed", &json!({"json": r#"["a","b"]"#})),
        json!({"ids": ["a", "b"]})
    );
}

#[test]
fn malformed_v1_records_are_preserved_as_unparsed_text() {
    let body = json!({"other": 1});
    assert_eq!(
        migrate_state_body("memes", &body),
        json!({ "unparsed": body.to_string() })
    );
}

#[test]
fn an_object_with_an_unparsed_field_survives_a_round_trip() {
    for kind in [IndexKind::Slang, IndexKind::Memes] {
        let text = r#"{"unparsed":"hello","terms":[]}"#;
        let body = state_body(kind, text);
        assert_eq!(body, json!({ "unparsed": text }));
        assert_eq!(state_text(kind, &body), text);
    }
}
