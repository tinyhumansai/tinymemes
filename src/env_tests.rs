use super::*;
use std::collections::HashMap;

fn cfg(pairs: &[(&str, &str)]) -> EnvConfig {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    EnvConfig::from_lookup(|k| map.get(k).cloned())
}

#[test]
fn unset_env_defers_everything_to_the_host() {
    let c = cfg(&[]);
    assert!(c.openrouter_key.is_none());
    assert_eq!(c.model_id(), DEFAULT_MODEL);
    assert!(c.jev().unwrap().is_none());
    assert!(c.chat_model(&reqwest::Client::new()).is_none());
}

#[test]
fn blank_values_count_as_unset() {
    let c = cfg(&[("TINYMEMES_OPENROUTER_KEY", "  "), ("TINYMEMES_MODEL", "")]);
    assert!(c.openrouter_key.is_none());
    assert_eq!(c.model_id(), DEFAULT_MODEL);
}

#[test]
fn explicit_modes_pick_routes_or_fail_loudly() {
    let c = cfg(&[("TINYMEMES_JEV", "llm"), ("TYPESAFE_API_KEY", "ts")]);
    assert!(c.forces_llm_jev());
    assert!(c.jev().unwrap().is_none());

    let c = cfg(&[("TINYMEMES_JEV", "typesafe"), ("TYPESAFE_API_KEY", "ts")]);
    assert_eq!(c.jev().unwrap().unwrap().1, "typesafe");

    let c = cfg(&[("TINYMEMES_JEV", "openrouter")]);
    assert!(c.jev().is_err());

    // Unset mode: an env OpenRouter key alone does not claim Jev (the host's
    // managed Jev comes first); explicit `auto` lets it.
    let c = cfg(&[("TINYMEMES_OPENROUTER_KEY", "or")]);
    assert!(c.jev().unwrap().is_none());
    let c = cfg(&[
        ("TINYMEMES_OPENROUTER_KEY", "or"),
        ("TINYMEMES_JEV", "auto"),
    ]);
    assert_eq!(c.jev().unwrap().unwrap().1, "openrouter");
}

#[test]
fn debug_never_prints_keys() {
    let c = cfg(&[("TINYMEMES_OPENROUTER_KEY", "sk-secret")]);
    assert!(!format!("{c:?}").contains("sk-secret"));
}
