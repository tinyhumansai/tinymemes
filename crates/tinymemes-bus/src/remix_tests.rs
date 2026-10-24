use super::*;

#[test]
fn request_defaults_enable_remix_with_a_budget() {
    let r: RemixRequest = serde_json::from_str(r#"{"conversation":[],"reply":"hi"}"#).unwrap();
    assert!(r.enabled && !r.deliver);
    assert_eq!(r.timeout_ms, 60_000);
    assert_eq!(r.region, None);
    assert_eq!(r.correlation, "");
    assert_eq!(r.reply_id, None);
    assert_eq!(
        r.options,
        RemixOptions {
            meme_cooldown_turns: None,
            research: true
        }
    );
}

#[test]
fn a_one_zero_peer_payload_still_decodes_without_correlation_fields() {
    let c: CompleteRequest = serde_json::from_str(r#"{"system":"s","user":"u"}"#).unwrap();
    assert_eq!(c.correlation, "");
    let t: WireTurn = serde_json::from_str(r#"{"role":"user","text":"hi"}"#).unwrap();
    assert_eq!(t.id, None);
    let l: LoadIndexRequest = serde_json::from_str(r#"{"kind":"remixed"}"#).unwrap();
    assert_eq!(l.kind, IndexKind::Remixed);
}

#[test]
fn bus_contract_uses_only_storage_data_types() {
    let manifest = include_str!("../Cargo.toml");
    // Normal dependencies only; dev-dependencies may use a runtime for tests.
    let normal = manifest.split("[dev-dependencies]").next().unwrap();
    for forbidden in [
        "tinybus",
        "tinystoragedrivers-core",
        "tinystoragedrivers-document",
        "tinystoragedrivers-testkit",
        "tokio",
        "reqwest",
    ] {
        assert!(
            !normal.contains(forbidden),
            "{forbidden} must not be a dependency"
        );
    }
    assert!(normal.contains("tinystoragedrivers-types"));
}

#[test]
fn member_tables_match_constants_and_names_are_distinct() {
    assert_eq!(REMIX_METHODS, &["Remix"]);
    assert_eq!(HOST_METHODS.len(), 7);
    assert!(HOST_METHODS.contains(&host_methods::END));
    assert!(HOST_METHODS.contains(&host_methods::DELIVER));
    assert_ne!(REMIX_INTERFACE, HOST_INTERFACE);
    assert_ne!(REMIX_PATH, HOST_PATH);
}

#[test]
fn version_binds_same_major_with_at_least_the_host_minor() {
    assert!(is_compatible(CONTRACT_VERSION));
    assert!(is_compatible((CONTRACT_VERSION.0, CONTRACT_VERSION.1 + 1)));
    assert!(!is_compatible((CONTRACT_VERSION.0 + 1, 0)));
}

#[test]
fn skip_reasons_and_failures_serialize_content_free() {
    assert_eq!(
        serde_json::to_string(&SkipReason::BelowThreshold).unwrap(),
        "\"below_threshold\""
    );
    assert_eq!(
        serde_json::to_string(&HostFailure::Timeout).unwrap(),
        r#"{"kind":"timeout"}"#
    );
}
