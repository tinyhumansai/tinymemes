//! Drives the real compiled `cdylib` through the module loader: a scripted
//! host serves the callback interface and the module remixes a reply over the
//! bus, falling back to the chat model for Jev because the host has none.
//!
//! The artifact is built with `cargo build -p tinymemes-module` unless
//! `TINYMEMES_MODULE` points at one (the path override CI and hosts use).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinybus::broker::Broker;
use tinybus::module::ModuleHost;
use tinybus::transport::memory::MemoryBus;
use tinybus::{CallContext, Connection, InterfaceName, MemberName, ObjectPath};
use tinymemes_bus::{
    HOST_INTERFACE, HOST_METHODS, HOST_PATH, HostFailure, MarkerRequest, RemixRequest, host_methods,
};
use tinymemes_client::{RemixClient, RemixMarkerClient};
use tinystoragedrivers_api::{DocumentStore, Storage, StorageBackend};
use tinystoragedrivers_busproxy::{PerCallerEndpoint, ServedStorage, StorageServer};
use tinystoragedrivers_testkit::{Failing, FaultPlan, MemoryBackend};
use tinystoragedrivers_types::{ErrorKind, Precondition, Scope, WriteOptions};

fn artifact() -> PathBuf {
    let path = if let Some(path) = std::env::var_os("TINYMEMES_MODULE") {
        PathBuf::from(path)
    } else {
        let exe = std::env::current_exe().unwrap();
        let profile_dir = exe.parent().and_then(Path::parent).unwrap().to_owned();
        let name = format!(
            "{}tinymemes_module{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        );
        let path = profile_dir.join(&name);
        if !path.exists() {
            let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
            let mut build = std::process::Command::new(cargo);
            build.args(["build", "--package", "tinymemes-module"]);
            if profile_dir.ends_with("release") {
                build.arg("--release");
            }
            assert!(build.status().unwrap().success(), "build the module");
        }
        path
    };
    assert!(
        path.is_file(),
        "compiled module artifact is missing: {}",
        path.display()
    );
    path
}

/// One callback as the host saw it.
#[derive(Clone, Debug)]
struct Seen {
    member: String,
    correlation: String,
    sender_is_authenticated: bool,
}

struct ScriptedHost {
    seen: Arc<Mutex<Vec<Seen>>>,
}

#[async_trait]
impl tinybus::Interface for ScriptedHost {
    fn name(&self) -> InterfaceName {
        InterfaceName::new(HOST_INTERFACE).unwrap()
    }
    fn members(&self) -> Vec<MemberName> {
        HOST_METHODS
            .iter()
            .map(|m| MemberName::new(*m).unwrap())
            .collect()
    }
    async fn call(
        &self,
        _: &MemberName,
        _: serde_json::Value,
    ) -> tinybus::Result<serde_json::Value> {
        Err(tinybus::Error::failed("unauthenticated"))
    }
    async fn call_with_context(
        &self,
        member: &MemberName,
        args: serde_json::Value,
        context: &CallContext,
    ) -> tinybus::Result<serde_json::Value> {
        let (payload,): (serde_json::Value,) = serde_json::from_value(args).unwrap();
        self.seen.lock().unwrap().push(Seen {
            member: member.as_str().to_owned(),
            correlation: payload["correlation"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            sender_is_authenticated: context.authenticated_sender().is_some(),
        });
        let reply = match member.as_str() {
            // No Jev here: the module must ask the chat model instead.
            host_methods::EVALUATE | host_methods::SEARCH => {
                serde_json::to_value(Result::<(), _>::Err(HostFailure::Unsupported))
            }
            host_methods::COMPLETE => {
                let text = if payload["system"]
                    .as_str()
                    .unwrap_or("")
                    .contains("classification")
                {
                    r#"{"chat_intent": {"choice": "grind"}, "reply_intent": {"choice": "grind"},
                        "frankness": {"level": 2}, "playful": {"p": 0.2}, "serious": {"p": 0.0}}"#
                } else {
                    "ngl that's done"
                };
                serde_json::to_value(Result::<_, HostFailure>::Ok(text))
            }
            host_methods::LOAD_INDEX => {
                serde_json::to_value(Result::<Option<String>, HostFailure>::Ok(None))
            }
            _ => serde_json::to_value(Result::<(), HostFailure>::Ok(())),
        };
        Ok(reply.unwrap())
    }
}

async fn load() -> (
    ModuleHost,
    Connection,
    tokio::task::JoinHandle<tinybus::Result<()>>,
    tempfile_dir::Dir,
    TestStorageHost,
) {
    let backend = MemoryBackend::new();
    let scope = Scope::new("tenant:compiled-remix-test").expect("scope");
    let storage = backend.for_scope(&scope).expect("storage ports");
    load_with_storage(storage).await
}

async fn load_with_storage(
    storage_bundle: Storage,
) -> (
    ModuleHost,
    Connection,
    tokio::task::JoinHandle<tinybus::Result<()>>,
    tempfile_dir::Dir,
    TestStorageHost,
) {
    load_with_storage_mode(storage_bundle, true)
        .await
        .expect("module setup succeeds with bound host storage")
}

type LoadedModule = (
    ModuleHost,
    Connection,
    tokio::task::JoinHandle<tinybus::Result<()>>,
    tempfile_dir::Dir,
    TestStorageHost,
);

async fn load_with_storage_mode(
    storage_bundle: Storage,
    serve_native_storage: bool,
) -> tinybus::Result<LoadedModule> {
    let artifact = artifact();
    let hash = tinybus::module::sha256_file(&artifact).unwrap();
    #[cfg(unix)]
    let fixture_parent = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| artifact.parent().unwrap().to_owned());
    #[cfg(not(unix))]
    let fixture_parent = artifact.parent().unwrap().to_owned();
    let dir = tempfile_dir::Dir::new_in(&fixture_parent);
    let staged = dir.path().join(artifact.file_name().unwrap());
    std::fs::copy(&artifact, &staged).unwrap();
    std::fs::write(
        dir.path().join("modules.toml"),
        format!(
            "{:?} = {hash:?}\n",
            staged.file_name().unwrap().to_str().unwrap()
        ),
    )
    .unwrap();
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let task = broker.spawn(bus.clone());
    let mut host_storage = TestStorageHost {
        _host: None,
        _served: None,
        storage: storage_bundle.clone(),
        binding: None,
        broker: broker.clone(),
    };
    if serve_native_storage {
        host_storage = serve_storage(&bus, storage_bundle, broker.clone()).await;
    }
    let host = ModuleHost::new(broker);
    let host_endpoint = tinybus::module::HostCallbackEndpoint {
        path: ObjectPath::new(HOST_PATH).unwrap(),
        interface: InterfaceName::new(HOST_INTERFACE).unwrap(),
        members: HOST_METHODS
            .iter()
            .map(|member| MemberName::new(*member).unwrap())
            .collect(),
    };
    let binding = host
        .prepare_host_callbacks(vec![host_endpoint])
        .await
        .unwrap();
    binding
        .serve_at(
            ObjectPath::new(HOST_PATH).unwrap(),
            ScriptedHost {
                seen: Arc::new(Mutex::new(Vec::new())),
            },
        )
        .await
        .unwrap();
    if let Err(error) = host
        .load_file_with_host_callbacks(&staged, serde_json::Value::Null, &binding)
        .await
    {
        task.abort();
        return Err(error);
    }
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    host_storage.binding = Some(binding);
    Ok((host, caller, task, dir, host_storage))
}

async fn load_unattested_with_storage(
    storage_bundle: Storage,
) -> (
    ModuleHost,
    Connection,
    tokio::task::JoinHandle<tinybus::Result<()>>,
    tempfile_dir::Dir,
    TestStorageHost,
) {
    let artifact = artifact();
    #[cfg(unix)]
    let fixture_parent = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| artifact.parent().unwrap().to_owned());
    #[cfg(not(unix))]
    let fixture_parent = artifact.parent().unwrap().to_owned();
    let dir = tempfile_dir::Dir::new_in(&fixture_parent);
    let staged = dir.path().join(artifact.file_name().unwrap());
    std::fs::copy(&artifact, &staged).unwrap();

    // Deliberately omit modules.toml. The module may load and serve ordinary
    // traffic, but the broker must not treat it as a confidential recipient.
    let bus = MemoryBus::new();
    let broker = Broker::new();
    let task = broker.spawn(bus.clone());
    let host_storage = serve_storage(&bus, storage_bundle, broker.clone()).await;
    let host = ModuleHost::new(broker);
    host.load_file(&staged)
        .expect("unattested module still loads");
    let caller = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    (host, caller, task, dir, host_storage)
}

struct TestStorageHost {
    _host: Option<Connection>,
    _served: Option<ServedStorage>,
    storage: Storage,
    binding: Option<tinybus::module::HostCallbackBinding>,
    broker: Broker,
}

async fn serve_storage(bus: &MemoryBus, storage: Storage, broker: Broker) -> TestStorageHost {
    let bound = storage.clone();
    let endpoint = PerCallerEndpoint::new(move |_sender| Some(StorageServer::new(bound.clone())));
    let host = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let served = tinystoragedrivers_busproxy::serve(&host, Arc::new(endpoint))
        .await
        .unwrap();
    TestStorageHost {
        _host: Some(host),
        _served: Some(served),
        storage,
        binding: None,
        broker,
    }
}

fn marker_client(storage_host: &TestStorageHost) -> RemixMarkerClient {
    let binding = storage_host
        .binding
        .as_ref()
        .expect("module callback binding");
    let peer = binding.guard().module_peer().expect("loaded module peer");
    RemixMarkerClient::new(
        binding
            .proxy(
                peer.as_str(),
                tinymemes_bus::MARKER_PATH,
                tinymemes_bus::MARKER_INTERFACE,
            )
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn previous_release_marker_is_migrated_and_read_by_the_compiled_module() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/v0.4.2-remix-marker.json")).unwrap();
    let backend = MemoryBackend::new();
    let scope = Scope::new("tenant:legacy-release-fixture").expect("scope");
    let storage = backend.for_scope(&scope).expect("storage ports");
    let mut options = WriteOptions::new().schema(fixture["schema"].as_u64().unwrap() as u32);
    if let Some(expires_ms) = fixture["expires_ms"].as_i64() {
        options = options.expires_ms(expires_ms);
    }
    storage
        .documents()
        .put_with(
            fixture["collection"].as_str().unwrap(),
            fixture["id"].as_str().unwrap(),
            fixture["body"].clone(),
            Precondition::Absent,
            options,
        )
        .await
        .unwrap();

    let (_host, _caller, _task, _dir, host_storage) = load_with_storage(storage).await;
    let migrated = host_storage
        .storage
        .documents()
        .get(
            tinymemes_bus::storage::MARKERS_COLLECTION,
            fixture["id"].as_str().unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        migrated.as_ref().map(|record| &record.doc),
        Some(&fixture["body"])
    );
    assert!(
        marker_client(&host_storage)
            .was_remixed(MarkerRequest {
                reply_id: "old-release-reply".into(),
            })
            .await
            .unwrap()
            .unwrap()
    );
    assert!(
        host_storage
            .storage
            .documents()
            .get(
                tinymemes_bus::storage::LEGACY_MARKERS_COLLECTION,
                fixture["id"].as_str().unwrap()
            )
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        host_storage
            .storage
            .documents()
            .get(
                tinymemes_bus::storage::MARKERS_COLLECTION,
                fixture["id"].as_str().unwrap(),
            )
            .await
            .unwrap()
            .unwrap()
            .doc,
        fixture["body"]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn module_startup_refuses_when_the_host_has_not_bound_storage() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:unbound-storage").unwrap())
        .unwrap();

    let error = match load_with_storage_mode(storage, false).await {
        Ok(_) => panic!("module must refuse startup when storage is not served"),
        Err(error) => error,
    };
    // TinyBus intentionally hides setup callback details from the loader, so
    // pin the specific module-initialization refusal. The fixture above omits
    // the storage endpoint; `artifact()` separately proves the cdylib exists.
    assert!(
        matches!(error, tinybus::Error::ModuleRefused { ref reason, .. }
            if reason == "module initialization failed"),
        "unexpected startup failure: {error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn marker_migration_conflict_refuses_module_start_and_preserves_both_records() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:marker-conflict").unwrap())
        .unwrap();
    let old = serde_json::json!({
        "format_version": 1,
        "business_revision": "1",
        "remixed": ["legacy"],
    });
    let current = serde_json::json!({
        "format_version": 1,
        "business_revision": "2",
        "remixed": ["newer"],
    });
    storage
        .documents()
        .put(
            tinymemes_bus::storage::LEGACY_MARKERS_COLLECTION,
            "remix-state",
            old.clone(),
            Precondition::Absent,
        )
        .await
        .unwrap();
    storage
        .documents()
        .put(
            tinymemes_bus::storage::MARKERS_COLLECTION,
            "remix-state",
            current.clone(),
            Precondition::Absent,
        )
        .await
        .unwrap();

    let error = match load_with_storage_mode(storage.clone(), true).await {
        Ok(_) => panic!("conflicting marker migration must refuse startup"),
        Err(error) => error,
    };
    // The loader redacts the callback's conflict text to the same setup
    // refusal. The conflicting records are asserted unchanged below, which
    // proves this fixture reached migration and refused for the right cause.
    assert!(
        matches!(error, tinybus::Error::ModuleRefused { ref reason, .. }
            if reason == "module initialization failed"),
        "unexpected startup failure: {error}"
    );
    assert_eq!(
        storage
            .documents()
            .get(
                tinymemes_bus::storage::LEGACY_MARKERS_COLLECTION,
                "remix-state"
            )
            .await
            .unwrap()
            .unwrap()
            .doc,
        old,
        "an incompatible copy must leave its source available for recovery"
    );
    assert_eq!(
        storage
            .documents()
            .get(tinymemes_bus::storage::MARKERS_COLLECTION, "remix-state")
            .await
            .unwrap()
            .unwrap()
            .doc,
        current,
        "module startup must not overwrite a conflicting current record"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_persisted_markers_are_refused_without_replacement() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:corrupt-markers").unwrap())
        .unwrap();
    let malformed = serde_json::json!({"format_version": 2, "remixed": "not-an-array"});
    let version = storage
        .documents()
        .put(
            tinymemes_bus::storage::MARKERS_COLLECTION,
            tinymemes_bus::MARKER_DOCUMENT_KEY,
            malformed.clone(),
            Precondition::Absent,
        )
        .await
        .unwrap();
    let (_host, _caller, _task, _dir, host_storage) = load_with_storage(storage.clone()).await;

    assert_eq!(
        marker_client(&host_storage)
            .was_remixed(MarkerRequest {
                reply_id: "reply".into(),
            })
            .await
            .unwrap(),
        Err(tinymemes_bus::MarkerFailure::CorruptState)
    );
    let after = storage
        .documents()
        .get(
            tinymemes_bus::storage::MARKERS_COLLECTION,
            tinymemes_bus::MARKER_DOCUMENT_KEY,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.version, version);
    assert_eq!(after.doc, malformed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revision_exhaustion_does_not_modify_persisted_markers() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:revision-limit").unwrap())
        .unwrap();
    let original = serde_json::json!({
        "format_version": 1,
        "business_revision": u64::MAX.to_string(),
        "remixed": ["already-marked"],
    });
    let version = storage
        .documents()
        .put(
            tinymemes_bus::storage::MARKERS_COLLECTION,
            tinymemes_bus::MARKER_DOCUMENT_KEY,
            original.clone(),
            Precondition::Absent,
        )
        .await
        .unwrap();
    let (_host, _caller, _task, _dir, host_storage) = load_with_storage(storage.clone()).await;
    let client = marker_client(&host_storage);

    assert_eq!(
        client
            .mark_remixed(MarkerRequest {
                reply_id: "already-marked".into(),
            })
            .await
            .unwrap()
            .unwrap(),
        tinymemes_bus::MarkRemixedReply {
            changed: false,
            business_revision: u64::MAX,
        }
    );
    assert_eq!(
        client
            .mark_remixed(MarkerRequest {
                reply_id: "new".into(),
            })
            .await
            .unwrap(),
        Err(tinymemes_bus::MarkerFailure::Content(
            tinymemes_bus::ContentError::RevisionExhausted
        ))
    );
    let after = storage
        .documents()
        .get(
            tinymemes_bus::storage::MARKERS_COLLECTION,
            tinymemes_bus::MARKER_DOCUMENT_KEY,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.version, version);
    assert_eq!(after.doc, original);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeating_a_compiled_mark_is_idempotent_after_the_first_write() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:repeat-mark").unwrap())
        .unwrap();
    let (_host, _caller, _task, _dir, host_storage) = load_with_storage(storage.clone()).await;
    let client = marker_client(&host_storage);
    let request = MarkerRequest {
        reply_id: "same-reply".into(),
    };
    assert!(
        client
            .mark_remixed(request.clone())
            .await
            .unwrap()
            .unwrap()
            .changed
    );
    let stored = storage
        .documents()
        .get(
            tinymemes_bus::storage::MARKERS_COLLECTION,
            tinymemes_bus::MARKER_DOCUMENT_KEY,
        )
        .await
        .unwrap()
        .unwrap();
    let duplicate = client.mark_remixed(request).await.unwrap().unwrap();
    assert!(!duplicate.changed);
    assert_eq!(duplicate.business_revision, 1);
    let after = storage
        .documents()
        .get(
            tinymemes_bus::storage::MARKERS_COLLECTION,
            tinymemes_bus::MARKER_DOCUMENT_KEY,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.version, stored.version);
    assert_eq!(after.doc, stored.doc);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ordinary_marker_calls_are_refused_without_mutation() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:unbound-caller").unwrap())
        .unwrap();
    let (_host, caller, _task, _dir, host_storage) = load_with_storage(storage.clone()).await;
    let peer = host_storage
        .binding
        .as_ref()
        .unwrap()
        .guard()
        .module_peer()
        .unwrap();
    let proxy = caller
        .proxy(
            peer.as_str(),
            tinymemes_bus::MARKER_PATH,
            tinymemes_bus::MARKER_INTERFACE,
        )
        .unwrap();

    let error = proxy
        .call::<serde_json::Value>(
            "WasRemixed",
            (MarkerRequest {
                reply_id: "reply".into(),
            },),
        )
        .await
        .unwrap_err();
    assert_eq!(error.wire_name(), tinybus::Error::CONFIDENTIALITY_REQUIRED);
    assert!(
        storage
            .documents()
            .get(
                tinymemes_bus::storage::MARKERS_COLLECTION,
                tinymemes_bus::MARKER_DOCUMENT_KEY,
            )
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admitted_module_accepts_confidential_marker_calls_from_bus_peers() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:attested-module").unwrap())
        .unwrap();
    let (_host, caller, _task, _dir, host_storage) = load_with_storage(storage.clone()).await;
    let peer = host_storage
        .binding
        .as_ref()
        .unwrap()
        .guard()
        .module_peer()
        .unwrap();
    let proxy = caller
        .proxy(
            peer.as_str(),
            tinymemes_bus::MARKER_PATH,
            tinymemes_bus::MARKER_INTERFACE,
        )
        .unwrap();
    let reply = proxy
        .call_confidential::<Result<tinymemes_bus::MarkRemixedReply, tinymemes_bus::MarkerFailure>>(
            "MarkRemixed",
            (MarkerRequest {
                reply_id: "reply".into(),
            },),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(reply.changed);
    assert!(
        storage
            .documents()
            .get(
                tinymemes_bus::storage::MARKERS_COLLECTION,
                tinymemes_bus::MARKER_DOCUMENT_KEY,
            )
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unattested_module_refuses_confidential_marker_calls_without_mutation() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:unattested-module").unwrap())
        .unwrap();
    let (_host, caller, _task, _dir, _host_storage) =
        load_unattested_with_storage(storage.clone()).await;
    let proxy = caller
        .proxy(
            tinymemes_bus::STATE_INTERFACE,
            tinymemes_bus::MARKER_PATH,
            tinymemes_bus::MARKER_INTERFACE,
        )
        .unwrap();
    let error = proxy
        .call_confidential::<serde_json::Value>(
            "MarkRemixed",
            (MarkerRequest {
                reply_id: "reply".into(),
            },),
        )
        .await
        .unwrap_err();
    assert_eq!(error.wire_name(), tinybus::Error::NOT_ATTESTED);
    assert!(
        storage
            .documents()
            .get(
                tinymemes_bus::storage::MARKERS_COLLECTION,
                tinymemes_bus::MARKER_DOCUMENT_KEY,
            )
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn simultaneous_module_load_is_refused_by_the_existing_bus_name_owner() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:duplicate-module").unwrap())
        .unwrap();
    let (_first_host, _caller, _task, first_dir, host_storage) =
        load_with_storage(storage.clone()).await;
    let artifact = artifact();
    let staged = first_dir
        .path()
        .join(artifact.file_name().expect("module filename"));
    let second_host = ModuleHost::new(host_storage.broker.clone());
    let host_endpoint = tinybus::module::HostCallbackEndpoint {
        path: ObjectPath::new(HOST_PATH).unwrap(),
        interface: InterfaceName::new(HOST_INTERFACE).unwrap(),
        members: HOST_METHODS
            .iter()
            .map(|member| MemberName::new(*member).unwrap())
            .collect(),
    };
    let binding = second_host
        .prepare_host_callbacks(vec![host_endpoint])
        .await
        .unwrap();
    binding
        .serve_at(
            ObjectPath::new(HOST_PATH).unwrap(),
            ScriptedHost {
                seen: Arc::new(Mutex::new(Vec::new())),
            },
        )
        .await
        .unwrap();

    let error = second_host
        .load_file_with_host_callbacks(&staged, serde_json::Value::Null, &binding)
        .await
        .unwrap_err();
    assert!(
        matches!(error, tinybus::Error::ModuleRefused { ref reason, .. } if reason == "module bus name is already owned"),
        "unexpected second-load result: {error:?}"
    );
    assert!(
        storage
            .documents()
            .get(
                tinymemes_bus::storage::MARKERS_COLLECTION,
                tinymemes_bus::MARKER_DOCUMENT_KEY,
            )
            .await
            .unwrap()
            .is_none()
    );
}

fn replace_documents(storage: &Storage, documents: Arc<dyn DocumentStore>) -> Storage {
    Storage::new(
        documents,
        Arc::clone(storage.streams()),
        Arc::clone(storage.kv()),
        Arc::clone(storage.blobs()),
        Arc::clone(storage.queues()),
        Arc::clone(storage.secrets()),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compiled_marker_write_retries_a_testkit_conflict() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:conflict-retry").unwrap())
        .unwrap();
    let faults = FaultPlan::new();
    let failing = Arc::new(Failing::with_plan(
        Arc::clone(storage.documents()),
        faults.clone(),
    ));
    let decorated = replace_documents(&storage, failing);
    let (_host, _caller, _task, _dir, host_storage) = load_with_storage(decorated.clone()).await;
    // busproxy performs a wire put as a metadata-aware native `put_with`.
    faults.fail_operation_nth("documents.put_with", 1, ErrorKind::Conflict);

    let result = marker_client(&host_storage)
        .mark_remixed(MarkerRequest {
            reply_id: "retried".into(),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(result.changed);
    assert_eq!(result.business_revision, 1);
    assert!(
        storage
            .documents()
            .get(
                tinymemes_bus::storage::MARKERS_COLLECTION,
                tinymemes_bus::MARKER_DOCUMENT_KEY,
            )
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compiled_marker_write_surfaces_non_conflict_storage_failure() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:write-failure").unwrap())
        .unwrap();
    let faults = FaultPlan::new();
    let failing = Arc::new(Failing::with_plan(
        Arc::clone(storage.documents()),
        faults.clone(),
    ));
    let decorated = replace_documents(&storage, failing);
    let (_host, _caller, _task, _dir, host_storage) = load_with_storage(decorated).await;
    faults.fail_operation_nth("documents.put_with", 1, ErrorKind::Unavailable);

    assert_eq!(
        marker_client(&host_storage)
            .mark_remixed(MarkerRequest {
                reply_id: "not-written".into(),
            })
            .await
            .unwrap(),
        Err(tinymemes_bus::MarkerFailure::Storage(
            tinymemes_bus::MarkerStorageKind::Unavailable
        ))
    );
    assert!(
        storage
            .documents()
            .get(
                tinymemes_bus::storage::MARKERS_COLLECTION,
                tinymemes_bus::MARKER_DOCUMENT_KEY,
            )
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compiled_markers_keep_scopes_isolated() {
    let backend = MemoryBackend::new();
    let alice = backend
        .for_scope(&Scope::new("tenant:compiled-alice").unwrap())
        .unwrap();
    let bob = backend
        .for_scope(&Scope::new("tenant:compiled-bob").unwrap())
        .unwrap();
    let (_alice_host, _alice_caller, _alice_task, _alice_dir, alice_host) =
        load_with_storage(alice.clone()).await;
    let (_bob_host, _bob_caller, _bob_task, _bob_dir, bob_host) =
        load_with_storage(bob.clone()).await;
    marker_client(&alice_host)
        .mark_remixed(MarkerRequest {
            reply_id: "alice-only".into(),
        })
        .await
        .unwrap()
        .unwrap();

    assert!(
        marker_client(&alice_host)
            .was_remixed(MarkerRequest {
                reply_id: "alice-only".into(),
            })
            .await
            .unwrap()
            .unwrap()
    );
    assert!(
        !marker_client(&bob_host)
            .was_remixed(MarkerRequest {
                reply_id: "alice-only".into(),
            })
            .await
            .unwrap()
            .unwrap()
    );
    assert!(
        alice
            .documents()
            .get(
                tinymemes_bus::storage::MARKERS_COLLECTION,
                tinymemes_bus::MARKER_DOCUMENT_KEY,
            )
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        bob.documents()
            .get(
                tinymemes_bus::storage::MARKERS_COLLECTION,
                tinymemes_bus::MARKER_DOCUMENT_KEY,
            )
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_compiled_marks_keep_both_ids() {
    let storage = MemoryBackend::new()
        .for_scope(&Scope::new("tenant:compiled-concurrent-marks").unwrap())
        .unwrap();
    let (_host, _caller, _task, _dir, host_storage) = load_with_storage(storage.clone()).await;
    let first = marker_client(&host_storage);
    let second = marker_client(&host_storage);
    let (first, second) = tokio::join!(
        first.mark_remixed(MarkerRequest {
            reply_id: "first".into(),
        }),
        second.mark_remixed(MarkerRequest {
            reply_id: "second".into(),
        })
    );
    assert!(first.unwrap().unwrap().changed);
    assert!(second.unwrap().unwrap().changed);
    let record = storage
        .documents()
        .get(
            tinymemes_bus::storage::MARKERS_COLLECTION,
            tinymemes_bus::MARKER_DOCUMENT_KEY,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.doc["business_revision"], "2");
    let mut ids = record.doc["remixed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect::<Vec<_>>();
    ids.sort_unstable();
    assert_eq!(ids, ["first", "second"]);
}

#[path = "support/content_usage.rs"]
mod content_usage;

/// A scratch directory next to the artifact, removed on drop.
mod tempfile_dir {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    pub struct Dir(PathBuf);
    impl Dir {
        pub fn new_in(parent: &Path) -> Self {
            loop {
                let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
                let path = parent.join(format!("compiled-remix-{}-{id}", std::process::id()));
                match std::fs::create_dir(&path) {
                    Ok(()) => {
                        #[cfg(unix)]
                        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                            .expect("make compiled-module fixture directory private");
                        return Self(path);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create compiled-module fixture directory: {error}"),
                }
            }
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[test]
fn concurrent_module_fixtures_get_distinct_directories() {
    let root = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("compiled-remix-dir-test-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let dirs = std::thread::scope(|scope| {
        (0..8)
            .map(|_| scope.spawn(|| tempfile_dir::Dir::new_in(&root)))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect::<Vec<_>>()
    });
    let mut paths = dirs
        .iter()
        .map(|dir| dir.path().to_owned())
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    assert_eq!(paths.len(), dirs.len());
    assert!(paths.iter().all(|path| path.is_dir()));
    drop(dirs);
    std::fs::remove_dir(root).unwrap();
}

fn request(correlation: &str) -> RemixRequest {
    serde_json::from_value(serde_json::json!({
        "conversation": [{"role": "user", "text": "ok the report is done", "id": "u-1"}],
        "reply": "That's done.",
        "region": "global",
        "correlation": correlation,
        "reply_id": "run-1:reply",
        "timeout_ms": 20_000,
    }))
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_compiled_module_remixes_through_the_hosts_callbacks_with_a_correlation_id() {
    let (_host, caller, _task, _dir, _storage) = load().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    caller
        .serve_at(
            ObjectPath::new(HOST_PATH).unwrap(),
            ScriptedHost { seen: seen.clone() },
        )
        .await
        .unwrap();
    let client = RemixClient::new(&caller).unwrap();
    let mut response = None;
    for _ in 0..50 {
        match client.remix(request("corr-e2e-1")).await {
            Ok(result) => {
                response = Some(result);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(100)).await,
        }
    }
    let response = response
        .expect("module answered")
        .expect("module accepted the caller");
    assert!(response.remixed, "{response:?}");
    assert_eq!(response.reply, "ngl that's done");

    let seen = seen.lock().unwrap().clone();
    let members: Vec<&str> = seen.iter().map(|s| s.member.as_str()).collect();
    // The host has no Jev, so the model answered Jev's questions, then rewrote.
    assert!(members.contains(&"Evaluate"), "{members:?}");
    assert!(
        members.iter().filter(|m| **m == "Complete").count() >= 2,
        "{members:?}"
    );
    assert!(members.contains(&"SaveIndex"), "{members:?}");
    assert!(
        seen.iter().all(|s| s.correlation == "corr-e2e-1"),
        "every callback carries the request's correlation id: {seen:?}"
    );
    assert!(seen.iter().all(|s| s.sender_is_authenticated), "{seen:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_remixes_keep_their_own_correlation_ids() {
    let (_host, caller, _task, _dir, _storage) = load().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    caller
        .serve_at(
            ObjectPath::new(HOST_PATH).unwrap(),
            ScriptedHost { seen: seen.clone() },
        )
        .await
        .unwrap();
    let client = RemixClient::new(&caller).unwrap();
    // Wait for the module to come up.
    for _ in 0..50 {
        if client.remix(request("warmup")).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    seen.lock().unwrap().clear();
    let calls = (0..4).map(|i| {
        let client = client.clone();
        tokio::spawn(async move { client.remix(request(&format!("corr-{i}"))).await })
    });
    for call in calls {
        let response = call.await.unwrap().unwrap().unwrap();
        assert!(response.remixed, "{response:?}");
    }
    let seen: Vec<Seen> = seen
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.correlation != "warmup")
        .cloned()
        .collect();
    for i in 0..4 {
        let id = format!("corr-{i}");
        assert!(
            seen.iter()
                .any(|s| s.member == "Complete" && s.correlation == id),
            "{id} reached the host: {seen:?}"
        );
    }
    assert!(
        seen.iter().all(|s| s.correlation.starts_with("corr-")),
        "{seen:?}"
    );
}
