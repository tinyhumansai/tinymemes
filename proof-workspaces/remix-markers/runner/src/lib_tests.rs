use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Duration};
use tinybus::{
    Connection,
    broker::Broker,
    module::{HostCallbackBinding, HostCallbackProxy, ModuleHost},
    transport::memory::MemoryBus,
};
use tinymemes_bus::*;
use tinystoragedrivers_core::{DocumentStore, MemoryStorage, Precondition, Scope, StorageBackend};
use tinystoragedrivers_tinybus::{endpoint, serve_collection};

const COLLECTION: &str = "remix_markers";
struct Fixture {
    host: Option<ModuleHost>,
    broker: Broker,
    binding: HostCallbackBinding,
    attacker: Connection,
    _artifact: tempfile::TempDir,
    artifact: PathBuf,
    broker_task: tokio::task::JoinHandle<tinybus::Result<()>>,
}
impl Fixture {
    async fn new(documents: Arc<dyn DocumentStore>) -> Self {
        let artifact = PathBuf::from(
            std::env::var_os("TINYMEMES_MODULE").expect("run scripts/test-remix-markers.sh"),
        );
        let hash = tinybus::module::sha256_file(&artifact).unwrap();
        let directory = tempfile::tempdir_in(artifact.parent().unwrap()).unwrap();
        let filename = artifact.file_name().unwrap();
        let staged = directory.path().join(filename);
        std::fs::copy(&artifact, &staged).unwrap();
        std::fs::write(
            directory.path().join("modules.toml"),
            format!("{:?} = {hash:?}\n", filename.to_str().unwrap()),
        )
        .unwrap();
        let bus = MemoryBus::new();
        let broker = Broker::new();
        let broker_task = broker.spawn(bus.clone());
        let host = ModuleHost::new(broker.clone());
        let binding = host
            .prepare_host_callbacks(vec![endpoint(MARKER_DOCUMENT_PATH).unwrap()])
            .await
            .unwrap();
        serve_collection(
            &binding,
            MARKER_DOCUMENT_PATH.parse().unwrap(),
            documents,
            COLLECTION,
        )
        .await
        .unwrap();
        host.load_file_with_host_callbacks(&staged, json!({}), &binding)
            .await
            .unwrap();
        let attacker = Connection::connect(bus.connect().await.unwrap())
            .await
            .unwrap();
        Self {
            host: Some(host),
            broker,
            binding,
            attacker,
            _artifact: directory,
            artifact: staged,
            broker_task,
        }
    }
    fn proxy_to(&self, peer: &str) -> HostCallbackProxy {
        self.binding
            .proxy(peer, MARKER_PATH, MARKER_INTERFACE)
            .unwrap()
            .with_timeout(Duration::from_secs(3))
    }
    fn client(&self) -> RemixMarkerClient {
        RemixMarkerClient::new(self.proxy_to(self.binding.guard().module_peer().unwrap().as_str()))
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.broker_task.abort();
    }
}
fn request(id: &str) -> MarkerRequest {
    MarkerRequest {
        reply_id: id.into(),
    }
}

async fn marker_roundtrip(backend: Arc<dyn StorageBackend>) {
    let alice = backend.for_scope(&Scope::new("alice").unwrap()).unwrap();
    let bob = backend.for_scope(&Scope::new("bob").unwrap()).unwrap();
    let first = Fixture::new(alice.documents().clone()).await;
    assert!(
        !first
            .client()
            .was_remixed(request("reply"))
            .await
            .unwrap()
            .unwrap()
    );
    let marked = first
        .client()
        .mark_remixed(request("reply"))
        .await
        .unwrap()
        .unwrap();
    assert!(marked.changed);
    assert_eq!(marked.business_revision, 1);
    let reopened = Fixture::new(alice.documents().clone()).await;
    assert!(
        reopened
            .client()
            .was_remixed(request("reply"))
            .await
            .unwrap()
            .unwrap()
    );
    let duplicate = reopened
        .client()
        .mark_remixed(request("reply"))
        .await
        .unwrap()
        .unwrap();
    assert!(!duplicate.changed);
    assert_eq!(duplicate.business_revision, 1);
    let other = Fixture::new(bob.documents().clone()).await;
    assert!(
        !other
            .client()
            .was_remixed(request("reply"))
            .await
            .unwrap()
            .unwrap()
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_marker_is_native_reopened_and_scope_isolated() {
    marker_roundtrip(Arc::new(MemoryStorage::new())).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_marker_survives_cold_database_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("markers.db");
    {
        let backend = tinystoragedrivers_sqlite::SqliteStorage::open(&path).unwrap();
        let scoped = backend.for_scope(&Scope::new("alice").unwrap()).unwrap();
        let fixture = Fixture::new(scoped.documents().clone()).await;
        let marked = fixture
            .client()
            .mark_remixed(request("reply"))
            .await
            .unwrap()
            .unwrap();
        assert!(marked.changed);
        assert_eq!(marked.business_revision, 1);
        fixture
            .host
            .as_ref()
            .unwrap()
            .shutdown(Duration::from_secs(3))
            .await;
    }
    assert!(path.is_file());
    let reopened_backend = tinystoragedrivers_sqlite::SqliteStorage::open(&path).unwrap();
    let alice = reopened_backend
        .for_scope(&Scope::new("alice").unwrap())
        .unwrap();
    let bob = reopened_backend
        .for_scope(&Scope::new("bob").unwrap())
        .unwrap();
    let alice_fixture = Fixture::new(alice.documents().clone()).await;
    let bob_fixture = Fixture::new(bob.documents().clone()).await;
    assert!(
        alice_fixture
            .client()
            .was_remixed(request("reply"))
            .await
            .unwrap()
            .unwrap()
    );
    assert!(
        !bob_fixture
            .client()
            .was_remixed(request("reply"))
            .await
            .unwrap()
            .unwrap()
    );
    let duplicate = alice_fixture
        .client()
        .mark_remixed(request("reply"))
        .await
        .unwrap()
        .unwrap();
    assert!(!duplicate.changed);
    assert_eq!(duplicate.business_revision, 1);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_same_artifact_load_is_refused_without_native_effects() {
    let backend = MemoryStorage::new();
    let scoped = backend.for_scope(&Scope::new("victim").unwrap()).unwrap();
    let fixture = Fixture::new(scoped.documents().clone()).await;
    let second_host = ModuleHost::new(fixture.broker.clone());
    let second_binding = second_host
        .prepare_host_callbacks(vec![endpoint(MARKER_DOCUMENT_PATH).unwrap()])
        .await
        .unwrap();
    serve_collection(
        &second_binding,
        MARKER_DOCUMENT_PATH.parse().unwrap(),
        scoped.documents().clone(),
        COLLECTION,
    )
    .await
    .unwrap();
    let error = second_host
        .load_file_with_host_callbacks(&fixture.artifact, json!({}), &second_binding)
        .await
        .unwrap_err();
    assert!(
        matches!(error, tinybus::Error::ModuleRefused { reason, .. } if reason == "module bus name is already owned")
    );
    assert!(
        scoped
            .documents()
            .get(COLLECTION, MARKER_DOCUMENT_KEY)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replacement_instance_cannot_enter_retained_previous_callback() {
    let backend = MemoryStorage::new();
    let scoped = backend.for_scope(&Scope::new("victim").unwrap()).unwrap();
    let original = json!({"format_version":1,"business_revision":"0","remixed":[]});
    let version = scoped
        .documents()
        .put(
            COLLECTION,
            MARKER_DOCUMENT_KEY,
            original.clone(),
            Precondition::Absent,
        )
        .await
        .unwrap();
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
    // Keep the original registration and host Connection alive, never re-enable
    // or retarget its captured guard. Only native loader creates the new peer.
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
    let replay = RemixMarkerClient::new(fixture.proxy_to(new_peer.as_str())).unwrap();
    assert_admission_error(
        replay.mark_remixed(request("reply")).await.unwrap_err(),
        tinybus::Error::NOT_ATTESTED,
    );
    old_documents.assert_calls(0, 0);
    new_documents.assert_calls(0, 0);
    // B is alive and its own captured route performs a successful native Get.
    let fresh = RemixMarkerClient::new(
        second_binding
            .proxy(new_peer.as_str(), MARKER_PATH, MARKER_INTERFACE)
            .unwrap()
            .with_timeout(Duration::from_secs(3)),
    )
    .unwrap();
    assert_eq!(
        fresh.was_remixed(request("reply")).await.unwrap(),
        Ok(false)
    );
    new_documents.assert_calls(1, 0);
    old_documents.assert_calls(0, 0);
    let after = scoped
        .documents()
        .get(COLLECTION, MARKER_DOCUMENT_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.version, version);
    assert_eq!(after.doc, original);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unauthorized_and_selector_requests_never_mutate_native_state() {
    let backend = MemoryStorage::new();
    let scoped = backend.for_scope(&Scope::new("victim").unwrap()).unwrap();
    let observed = PairedReads::observed(scoped.documents().clone());
    let fixture = Fixture::new(observed.clone()).await;
    let peer = fixture.binding.guard().module_peer().unwrap();
    let ordinary = fixture
        .attacker
        .proxy(peer.as_str(), MARKER_PATH, MARKER_INTERFACE)
        .unwrap();
    assert_admission_error(
        ordinary
            .call::<serde_json::Value>("MarkRemixed", (request("reply"),))
            .await
            .unwrap_err(),
        tinybus::Error::CONFIDENTIALITY_REQUIRED,
    );
    assert_admission_error(
        ordinary
            .call_confidential::<serde_json::Value>("MarkRemixed", (request("reply"),))
            .await
            .unwrap_err(),
        tinybus::Error::NOT_ATTESTED,
    );
    for field in [
        "scope",
        "url",
        "collection",
        "destination",
        "state",
        "expected_revision",
    ] {
        let mut value = json!({"reply_id":"reply"});
        value[field] = json!("forged");
        let error = fixture
            .proxy_to(peer.as_str())
            .call_confidential::<serde_json::Value>("MarkRemixed", (value,))
            .await
            .unwrap_err();
        assert_eq!(error.wire_message(), "invalid marker request");
        assert_admission_error(error, tinybus::Error::FAILED);
    }
    observed.assert_calls(0, 0);
    assert!(
        scoped
            .documents()
            .get(COLLECTION, MARKER_DOCUMENT_KEY)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_native_state_is_not_replaced_with_default() {
    let backend = MemoryStorage::new();
    let scoped = backend.for_scope(&Scope::new("victim").unwrap()).unwrap();
    let version = scoped
        .documents()
        .put(
            COLLECTION,
            MARKER_DOCUMENT_KEY,
            json!({"not":"marker-state"}),
            Precondition::Absent,
        )
        .await
        .unwrap();
    let fixture = Fixture::new(scoped.documents().clone()).await;
    assert_eq!(
        fixture
            .client()
            .mark_remixed(request("reply"))
            .await
            .unwrap()
            .unwrap_err(),
        MarkerFailure::CorruptState
    );
    let persisted = scoped
        .documents()
        .get(COLLECTION, MARKER_DOCUMENT_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.version, version);
    assert_eq!(persisted.doc, json!({"not":"marker-state"}));
}

async fn stale_actor(backend: Arc<dyn StorageBackend>) {
    use tinystoragedrivers_core::{Fence, Filter};
    let guard_scope = Scope::new("cluster").unwrap();
    let guard = backend.for_scope(&guard_scope).unwrap();
    guard
        .documents()
        .put("leases", "victim", json!({"epoch":1}), Precondition::None)
        .await
        .unwrap();
    let victim_scope = Scope::new("victim").unwrap();
    let unguarded = backend.for_scope(&victim_scope).unwrap();
    let original = json!({"format_version":1,"business_revision":"0","remixed":[]});
    let original_version = unguarded
        .documents()
        .put(
            COLLECTION,
            MARKER_DOCUMENT_KEY,
            original.clone(),
            Precondition::Absent,
        )
        .await
        .unwrap();
    let actor = backend
        .for_scope_fenced(
            &victim_scope,
            &Fence::new(guard_scope, "leases", "victim", Filter::eq("epoch", 1)),
        )
        .unwrap();
    let observed = PairedReads::observed(actor.documents().clone());
    let fixture = Fixture::new(observed.clone()).await;
    guard
        .documents()
        .put("leases", "victim", json!({"epoch":2}), Precondition::None)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .client()
            .mark_remixed(request("reply"))
            .await
            .unwrap()
            .unwrap_err(),
        MarkerFailure::Storage(tinystoragedrivers_bus::StorageKind::Fenced)
    );
    observed.assert_calls(1, 1);
    let after = unguarded
        .documents()
        .get(COLLECTION, MARKER_DOCUMENT_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.version, original_version);
    assert_eq!(after.doc, original);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_stale_actor_refuses_without_replacement_grant() {
    stale_actor(Arc::new(MemoryStorage::new())).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_stale_actor_refuses_without_replacement_grant() {
    let directory = tempfile::tempdir().unwrap();
    stale_actor(Arc::new(
        tinystoragedrivers_sqlite::SqliteStorage::open(directory.path().join("markers.db"))
            .unwrap(),
    ))
    .await;
}

// Interpose scheduling and explicit fault/acknowledgment injection over a real
// native port. All persisted state and competing CAS remain in the driver. Exactly two initial reads observe the same state.
#[derive(Clone, Copy, Debug)]
enum PutPolicy {
    Conflicts(usize),
    Refuse(tinystoragedrivers_core::ErrorKind),
    LostAcknowledgment,
}
#[derive(Debug)]
struct PairedReads {
    inner: Arc<dyn DocumentStore>,
    reads: std::sync::atomic::AtomicUsize,
    puts: std::sync::atomic::AtomicUsize,
    policy: Option<PutPolicy>,
    barrier: Option<tokio::sync::Barrier>,
}
impl PairedReads {
    fn observed(inner: Arc<dyn DocumentStore>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            reads: std::sync::atomic::AtomicUsize::new(0),
            puts: std::sync::atomic::AtomicUsize::new(0),
            policy: None,
            barrier: None,
        })
    }
    fn faulty(inner: Arc<dyn DocumentStore>, policy: PutPolicy) -> Arc<Self> {
        Arc::new(Self {
            inner,
            reads: std::sync::atomic::AtomicUsize::new(0),
            puts: std::sync::atomic::AtomicUsize::new(0),
            policy: Some(policy),
            barrier: None,
        })
    }
    fn assert_calls(&self, reads: usize, puts: usize) {
        assert_eq!(self.reads.load(std::sync::atomic::Ordering::SeqCst), reads);
        assert_eq!(self.puts.load(std::sync::atomic::Ordering::SeqCst), puts);
    }
}
// Error identity survives nested wire forwarding: canonical wire_name maps
// NotAttested/ConfidentialityRequired directly, and MethodFailed retains name.
fn assert_admission_error(error: tinybus::Error, name: &str) {
    assert_eq!(
        error.wire_name(),
        name,
        "unexpected failure category: {error:?}"
    );
}
#[async_trait::async_trait]
impl DocumentStore for PairedReads {
    fn capabilities(&self) -> tinystoragedrivers_core::Capabilities {
        self.inner.capabilities()
    }
    async fn ensure_collection(
        &self,
        spec: &tinystoragedrivers_core::CollectionSpec,
    ) -> tinystoragedrivers_core::Result<()> {
        self.inner.ensure_collection(spec).await
    }
    async fn get(
        &self,
        collection: &str,
        id: &str,
    ) -> tinystoragedrivers_core::Result<
        Option<tinystoragedrivers_core::Versioned<serde_json::Value>>,
    > {
        let result = self.inner.get(collection, id).await;
        let ordinal = self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(barrier) = self.barrier.as_ref().filter(|_| ordinal < 2) {
            barrier.wait().await;
        }
        result
    }
    async fn put(
        &self,
        collection: &str,
        id: &str,
        doc: serde_json::Value,
        condition: Precondition,
    ) -> tinystoragedrivers_core::Result<tinystoragedrivers_core::Version> {
        let ordinal = self.puts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        match self.policy {
            Some(PutPolicy::Conflicts(count)) if ordinal < count => {
                return Err(tinystoragedrivers_core::StorageError::conflict(
                    "injected native conflict",
                ));
            }
            Some(PutPolicy::Refuse(kind)) => {
                return Err(tinystoragedrivers_core::StorageError::new(
                    kind,
                    "private backend cause",
                ));
            }
            Some(PutPolicy::LostAcknowledgment) => {
                self.inner.put(collection, id, doc, condition).await?;
                return Err(tinystoragedrivers_core::StorageError::unavailable(
                    "private lost acknowledgment",
                ));
            }
            _ => {}
        }
        self.inner.put(collection, id, doc, condition).await
    }
    async fn delete(
        &self,
        collection: &str,
        id: &str,
        condition: Precondition,
    ) -> tinystoragedrivers_core::Result<bool> {
        self.inner.delete(collection, id, condition).await
    }
    async fn query(
        &self,
        collection: &str,
        query: &tinystoragedrivers_core::Query,
    ) -> tinystoragedrivers_core::Result<
        tinystoragedrivers_core::Page<tinystoragedrivers_core::Versioned<serde_json::Value>>,
    > {
        self.inner.query(collection, query).await
    }
    async fn count(
        &self,
        collection: &str,
        filter: &tinystoragedrivers_core::Filter,
    ) -> tinystoragedrivers_core::Result<u64> {
        self.inner.count(collection, filter).await
    }
    async fn delete_where(
        &self,
        collection: &str,
        filter: &tinystoragedrivers_core::Filter,
    ) -> tinystoragedrivers_core::Result<u64> {
        self.inner.delete_where(collection, filter).await
    }
    async fn claim(
        &self,
        collection: &str,
        filter: &tinystoragedrivers_core::Filter,
        sort: &[tinystoragedrivers_core::Sort],
        patch: &serde_json::Value,
    ) -> tinystoragedrivers_core::Result<
        Option<tinystoragedrivers_core::Versioned<serde_json::Value>>,
    > {
        self.inner.claim(collection, filter, sort, patch).await
    }
    async fn drop_collection(&self, collection: &str) -> tinystoragedrivers_core::Result<()> {
        self.inner.drop_collection(collection).await
    }
}
async fn competing_markers(backend: Arc<dyn StorageBackend>, same_id: bool) {
    let scoped = backend
        .for_scope(&Scope::new("competing").unwrap())
        .unwrap();
    let documents: Arc<dyn DocumentStore> = Arc::new(PairedReads {
        inner: scoped.documents().clone(),
        reads: std::sync::atomic::AtomicUsize::new(0),
        puts: std::sync::atomic::AtomicUsize::new(0),
        policy: None,
        barrier: Some(tokio::sync::Barrier::new(2)),
    });
    let first = Fixture::new(documents.clone()).await;
    let second = Fixture::new(documents).await;
    let first_client = first.client();
    let second_client = second.client();
    let second_id = if same_id { "first" } else { "second" };
    let (a, b) = tokio::join!(
        first_client.mark_remixed(request("first")),
        second_client.mark_remixed(request(second_id))
    );
    let a = a.unwrap().unwrap();
    let b = b.unwrap().unwrap();
    assert_eq!(
        usize::from(a.changed) + usize::from(b.changed),
        if same_id { 1 } else { 2 }
    );
    assert!(
        first_client
            .was_remixed(request("first"))
            .await
            .unwrap()
            .unwrap()
    );
    assert!(
        first_client
            .was_remixed(request(second_id))
            .await
            .unwrap()
            .unwrap()
    );
    assert_eq!(
        a.business_revision.max(b.business_revision),
        if same_id { 1 } else { 2 }
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_competing_distinct_markers_preserve_union_after_cas_reread() {
    competing_markers(Arc::new(MemoryStorage::new()), false).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_competing_duplicate_marker_commits_once() {
    competing_markers(Arc::new(MemoryStorage::new()), true).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_competing_distinct_markers_preserve_union_after_cas_reread() {
    let directory = tempfile::tempdir().unwrap();
    competing_markers(
        Arc::new(
            tinystoragedrivers_sqlite::SqliteStorage::open(directory.path().join("markers.db"))
                .unwrap(),
        ),
        false,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_native_marker_envelopes_are_refused_without_overwrite() {
    let malformed = [
        json!({"business_revision":"0","remixed":[]}),
        json!({"format_version":2,"business_revision":"0","remixed":[]}),
        json!({"format_version":1,"business_revision":"01","remixed":[]}),
        json!({"format_version":1,"business_revision":"-1","remixed":[]}),
        json!({"format_version":1,"business_revision":"18446744073709551616","remixed":[]}),
        json!({"format_version":1,"business_revision":0,"remixed":[]}),
        json!({"format_version":1,"business_revision":"0","remixed":"invalid"}),
        json!({"format_version":1,"business_revision":"0","remixed":["duplicate","duplicate"]}),
        json!({"format_version":1,"business_revision":"0","remixed":[],"scope":"forged"}),
    ];
    for (index, document) in malformed.into_iter().enumerate() {
        let backend = MemoryStorage::new();
        let scoped = backend
            .for_scope(&Scope::new(format!("invalid-{index}")).unwrap())
            .unwrap();
        let version = scoped
            .documents()
            .put(
                COLLECTION,
                MARKER_DOCUMENT_KEY,
                document.clone(),
                Precondition::Absent,
            )
            .await
            .unwrap();
        let observed = PairedReads::observed(scoped.documents().clone());
        let fixture = Fixture::new(observed.clone()).await;
        assert_eq!(
            fixture
                .client()
                .mark_remixed(request("reply"))
                .await
                .unwrap(),
            Err(MarkerFailure::CorruptState)
        );
        observed.assert_calls(1, 0);
        let persisted = scoped
            .documents()
            .get(COLLECTION, MARKER_DOCUMENT_KEY)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(persisted.version, version);
        assert_eq!(persisted.doc, document);
    }
}
async fn revision_exhaustion(backend: Arc<dyn StorageBackend>) {
    let scoped = backend
        .for_scope(&Scope::new("revision-limit").unwrap())
        .unwrap();
    let original =
        json!({"format_version":1,"business_revision":u64::MAX.to_string(),"remixed":["existing"]});
    let version = scoped
        .documents()
        .put(
            COLLECTION,
            MARKER_DOCUMENT_KEY,
            original.clone(),
            Precondition::Absent,
        )
        .await
        .unwrap();
    let observed = PairedReads::observed(scoped.documents().clone());
    let fixture = Fixture::new(observed.clone()).await;
    assert_eq!(
        fixture
            .client()
            .mark_remixed(request("existing"))
            .await
            .unwrap()
            .unwrap(),
        MarkRemixedReply {
            changed: false,
            business_revision: u64::MAX
        }
    );
    assert_eq!(
        fixture.client().mark_remixed(request("new")).await.unwrap(),
        Err(MarkerFailure::Content(ContentError::RevisionExhausted))
    );
    observed.assert_calls(2, 0);
    let persisted = scoped
        .documents()
        .get(COLLECTION, MARKER_DOCUMENT_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(persisted.version, version);
    assert_eq!(persisted.doc, original);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn memory_revision_max_preserves_duplicate_and_refuses_changed_state() {
    revision_exhaustion(Arc::new(MemoryStorage::new())).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_revision_max_preserves_duplicate_and_refuses_changed_state() {
    let directory = tempfile::tempdir().unwrap();
    revision_exhaustion(Arc::new(
        tinystoragedrivers_sqlite::SqliteStorage::open(directory.path().join("markers.db"))
            .unwrap(),
    ))
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conflicts_trigger_fresh_reads_before_eventual_commit() {
    let backend = MemoryStorage::new();
    let scoped = backend.for_scope(&Scope::new("retry").unwrap()).unwrap();
    let observed = PairedReads::faulty(scoped.documents().clone(), PutPolicy::Conflicts(2));
    let fixture = Fixture::new(observed.clone()).await;
    assert_eq!(
        fixture
            .client()
            .mark_remixed(request("reply"))
            .await
            .unwrap()
            .unwrap(),
        MarkRemixedReply {
            changed: true,
            business_revision: 1
        }
    );
    observed.assert_calls(3, 3);
    assert!(
        scoped
            .documents()
            .get(COLLECTION, MARKER_DOCUMENT_KEY)
            .await
            .unwrap()
            .is_some()
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_conflict_retries_have_an_exact_finite_bound() {
    let backend = MemoryStorage::new();
    let scoped = backend
        .for_scope(&Scope::new("exhausted").unwrap())
        .unwrap();
    let observed =
        PairedReads::faulty(scoped.documents().clone(), PutPolicy::Conflicts(usize::MAX));
    let fixture = Fixture::new(observed.clone()).await;
    assert_eq!(
        fixture
            .client()
            .mark_remixed(request("reply"))
            .await
            .unwrap(),
        Err(MarkerFailure::ConflictExhausted)
    );
    observed.assert_calls(8, 8);
    assert!(
        scoped
            .documents()
            .get(COLLECTION, MARKER_DOCUMENT_KEY)
            .await
            .unwrap()
            .is_none()
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_conflict_native_errors_never_retry_or_expose_private_cause() {
    use tinystoragedrivers_bus::StorageKind;
    use tinystoragedrivers_core::ErrorKind;
    for (index, (native, wire)) in [
        (
            ErrorKind::Unsupported(tinystoragedrivers_core::Capability::Fencing),
            StorageKind::Unsupported,
        ),
        (ErrorKind::Fenced, StorageKind::Fenced),
        (ErrorKind::Unavailable, StorageKind::Unavailable),
        (ErrorKind::AlreadyExists, StorageKind::AlreadyExists),
    ]
    .into_iter()
    .enumerate()
    {
        let backend = MemoryStorage::new();
        let scoped = backend
            .for_scope(&Scope::new(format!("refused-{index}")).unwrap())
            .unwrap();
        let observed = PairedReads::faulty(scoped.documents().clone(), PutPolicy::Refuse(native));
        let fixture = Fixture::new(observed.clone()).await;
        let result = fixture
            .client()
            .mark_remixed(request("reply"))
            .await
            .unwrap();
        assert_eq!(result, Err(MarkerFailure::Storage(wire)));
        assert!(!serde_json::to_string(&result).unwrap().contains("private"));
        observed.assert_calls(1, 1);
        assert!(
            scoped
                .documents()
                .get(COLLECTION, MARKER_DOCUMENT_KEY)
                .await
                .unwrap()
                .is_none()
        );
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_native_acknowledgment_does_not_retry_or_claim_rollback() {
    let backend = MemoryStorage::new();
    let scoped = backend
        .for_scope(&Scope::new("uncertain").unwrap())
        .unwrap();
    let observed = PairedReads::faulty(scoped.documents().clone(), PutPolicy::LostAcknowledgment);
    let fixture = Fixture::new(observed.clone()).await;
    assert_eq!(
        fixture
            .client()
            .mark_remixed(request("reply"))
            .await
            .unwrap(),
        Err(MarkerFailure::Storage(
            tinystoragedrivers_bus::StorageKind::Unavailable
        ))
    );
    observed.assert_calls(1, 1);
    let committed = scoped
        .documents()
        .get(COLLECTION, MARKER_DOCUMENT_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(committed.doc["remixed"], json!(["reply"]));
    let duplicate = fixture
        .client()
        .mark_remixed(request("reply"))
        .await
        .unwrap()
        .unwrap();
    assert!(!duplicate.changed);
    assert_eq!(duplicate.business_revision, 1);
    observed.assert_calls(2, 1);
    let after = scoped
        .documents()
        .get(COLLECTION, MARKER_DOCUMENT_KEY)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.version, committed.version);
    assert_eq!(after.doc, committed.doc);
}

#[path = "content_usage_tests.rs"]
mod content_usage_tests;
