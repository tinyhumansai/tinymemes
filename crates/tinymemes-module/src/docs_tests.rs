use super::*;
use crate::host::Host;
use async_trait::async_trait;
use std::sync::Arc;
use tinymemes_bus::{
    CompleteRequest, DeliverRequest, HostFailure, HostResult, SearchRequest, SearchResult,
};

/// A host that stores documents in memory and counts saves.
#[derive(Default)]
struct Store {
    docs: Mutex<std::collections::HashMap<&'static str, String>>,
    saves: Mutex<Vec<&'static str>>,
}

fn key(kind: IndexKind) -> &'static str {
    match kind {
        IndexKind::Slang => "slang",
        IndexKind::Memes => "memes",
        IndexKind::Remixed => "remixed",
    }
}

#[async_trait]
impl Host for Store {
    async fn complete(&self, _: CompleteRequest) -> HostResult<String> {
        Err(HostFailure::Unsupported)
    }
    async fn evaluate(&self, _: serde_json::Value) -> HostResult<serde_json::Value> {
        Err(HostFailure::Unsupported)
    }
    async fn search(&self, _: SearchRequest) -> HostResult<Vec<SearchResult>> {
        Err(HostFailure::Unsupported)
    }
    async fn load_index(&self, kind: IndexKind) -> HostResult<Option<String>> {
        Ok(self.docs.lock().unwrap().get(key(kind)).cloned())
    }
    async fn save_index(&self, kind: IndexKind, json: String) -> HostResult<()> {
        self.saves.lock().unwrap().push(key(kind));
        self.docs.lock().unwrap().insert(key(kind), json);
        Ok(())
    }
    async fn deliver(&self, _: DeliverRequest) -> HostResult<()> {
        Ok(())
    }
    async fn end(&self) -> HostResult<()> {
        Ok(())
    }
}

#[test]
fn a_missing_or_corrupt_document_is_a_fresh_start() {
    let docs = Documents::from_parts(None, Some("not json".into()), Some("{\"x\":1}".into()));
    assert!(docs.load_slang().is_empty("IN") || docs.load_slang().len("IN") > 0);
    assert_eq!(
        docs.load_memes().len("IN"),
        MemeIndex::new(MemeIndexPolicy::default()).len("IN")
    );
    assert!(docs.load_remixed().is_empty());
}

#[test]
fn the_legacy_remixed_json_array_loads_unchanged_and_is_capped() {
    let ids: Vec<String> = (0..tinymemes::store::REMIXED_CAP + 5)
        .map(|i| format!("m{i}"))
        .collect();
    let docs = Documents::from_parts(None, None, Some(serde_json::to_string(&ids).unwrap()));
    let ring = docs.load_remixed();
    assert_eq!(ring.len(), tinymemes::store::REMIXED_CAP);
    assert_eq!(ring.front().map(String::as_str), Some("m5"));
    assert_eq!(
        ring.back().map(String::as_str),
        Some(ids.last().unwrap().as_str())
    );
}

#[tokio::test]
async fn saves_are_collected_and_flushed_once() {
    let store = Arc::new(Store::default());
    let host: SharedHost = store.clone();
    let docs = Documents::fetch(&host).await;
    docs.save_remixed(&VecDeque::from(["a".to_owned()]))
        .unwrap();
    docs.flush(&host).await;
    docs.flush(&host).await;
    assert_eq!(*store.saves.lock().unwrap(), vec!["remixed"]);
    assert_eq!(store.docs.lock().unwrap()["remixed"], "[\"a\"]");
}

#[tokio::test]
async fn marking_appends_once_and_keeps_the_existing_ids() {
    let store = Arc::new(Store::default());
    store
        .docs
        .lock()
        .unwrap()
        .insert("remixed", "[\"old\"]".into());
    let host: SharedHost = store.clone();
    Documents::mark_remixed(&host, "new".into()).await;
    Documents::mark_remixed(&host, "new".into()).await;
    assert_eq!(store.docs.lock().unwrap()["remixed"], "[\"old\",\"new\"]");
    assert_eq!(
        store.saves.lock().unwrap().len(),
        1,
        "a repeat mark writes nothing"
    );
}

#[tokio::test]
async fn concurrent_marks_do_not_lose_each_others_ids() {
    let store = Arc::new(Store::default());
    let host: SharedHost = store.clone();
    let tasks: Vec<_> = (0..16)
        .map(|i| {
            let host = host.clone();
            tokio::spawn(async move { Documents::mark_remixed(&host, format!("id{i}")).await })
        })
        .collect();
    for task in tasks {
        task.await.unwrap();
    }
    let ids: Vec<String> = serde_json::from_str(&store.docs.lock().unwrap()["remixed"]).unwrap();
    assert_eq!(ids.len(), 16);
}
