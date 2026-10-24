use std::sync::Arc;

use tinybus::{Connection, transport::memory::MemoryBus};
use tinystoragedrivers_api::StorageBackend;
use tinystoragedrivers_busproxy::{PerCallerEndpoint, ServedStorage, StorageServer};
use tinystoragedrivers_testkit::MemoryBackend;
use tinystoragedrivers_types::Scope;

pub(crate) struct StorageHost {
    _host: Connection,
    _served: ServedStorage,
}

pub(crate) async fn serve_storage(
    bus: &MemoryBus,
    scope_name: &str,
) -> tinybus::Result<StorageHost> {
    let backend = MemoryBackend::new();
    let scope = Scope::new(scope_name).expect("test storage scope");
    let storage = backend.for_scope(&scope).expect("test storage ports");
    let bound = storage.clone();
    let endpoint = PerCallerEndpoint::new(move |_sender| Some(StorageServer::new(bound.clone())));
    let host = Connection::connect(bus.connect().await?).await?;
    let served = tinystoragedrivers_busproxy::serve(&host, Arc::new(endpoint)).await?;
    Ok(StorageHost {
        _host: host,
        _served: served,
    })
}
