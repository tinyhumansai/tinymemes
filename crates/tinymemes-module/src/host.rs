//! The module's view of the host: the callbacks served at `HOST_INTERFACE`.
use std::sync::Arc;

use async_trait::async_trait;
use tinymemes_bus::{
    CompleteRequest, DeliverRequest, EndRequest, EvaluateEnvelope, HOST_METHODS, HostFailure,
    HostResult, IndexKind, LoadIndexRequest, SaveIndexRequest, SearchRequest, SearchResult,
    host_methods,
};

/// Everything the module asks of the host. A mock implements this in tests.
#[async_trait]
pub(crate) trait Host: Send + Sync {
    async fn complete(&self, request: CompleteRequest) -> HostResult<String>;
    async fn evaluate(&self, request: serde_json::Value) -> HostResult<serde_json::Value>;
    async fn search(&self, request: SearchRequest) -> HostResult<Vec<SearchResult>>;
    async fn load_index(&self, kind: IndexKind) -> HostResult<Option<String>>;
    async fn save_index(&self, kind: IndexKind, json: String) -> HostResult<()>;
    async fn deliver(&self, request: DeliverRequest) -> HostResult<()>;
    /// This request, background research included, is finished.
    async fn end(&self) -> HostResult<()>;
}

/// The caller's host over the bus. The destination is the request's
/// authenticated sender, never configuration or request data. Every callback
/// carries the request's correlation id, so the host can serve concurrent
/// remixes without a process-wide gate.
pub(crate) struct BusHost {
    proxy: tinybus::Proxy,
    correlation: String,
}

impl BusHost {
    pub(crate) fn new(proxy: tinybus::Proxy, correlation: String) -> Self {
        Self { proxy, correlation }
    }

    async fn ask<A, R>(&self, member: &'static str, args: A) -> HostResult<R>
    where
        A: serde::Serialize + Send,
        R: serde::de::DeserializeOwned,
    {
        debug_assert!(HOST_METHODS.contains(&member));
        // A transport failure or an undecodable reply is the host failing.
        match self.proxy.call::<HostResult<R>>(member, (args,)).await {
            Ok(result) => result,
            Err(error) => {
                tracing::debug!(member, error = %error, "[tinymemes] host callback failed");
                Err(HostFailure::Backend)
            }
        }
    }
}

#[async_trait]
impl Host for BusHost {
    async fn complete(&self, mut request: CompleteRequest) -> HostResult<String> {
        request.correlation = self.correlation.clone();
        self.ask(host_methods::COMPLETE, request).await
    }
    async fn evaluate(&self, request: serde_json::Value) -> HostResult<serde_json::Value> {
        let reply: EvaluateEnvelope = self
            .ask(
                host_methods::EVALUATE,
                EvaluateEnvelope {
                    correlation: self.correlation.clone(),
                    body: request,
                },
            )
            .await?;
        Ok(reply.body)
    }
    async fn search(&self, mut request: SearchRequest) -> HostResult<Vec<SearchResult>> {
        request.correlation = self.correlation.clone();
        self.ask(host_methods::SEARCH, request).await
    }
    async fn load_index(&self, kind: IndexKind) -> HostResult<Option<String>> {
        let correlation = self.correlation.clone();
        self.ask(
            host_methods::LOAD_INDEX,
            LoadIndexRequest { correlation, kind },
        )
        .await
    }
    async fn save_index(&self, kind: IndexKind, json: String) -> HostResult<()> {
        let correlation = self.correlation.clone();
        self.ask(
            host_methods::SAVE_INDEX,
            SaveIndexRequest {
                correlation,
                kind,
                json,
            },
        )
        .await
    }
    async fn deliver(&self, mut request: DeliverRequest) -> HostResult<()> {
        request.correlation = self.correlation.clone();
        self.ask(host_methods::DELIVER, request).await
    }
    async fn end(&self) -> HostResult<()> {
        let correlation = self.correlation.clone();
        self.ask(host_methods::END, EndRequest { correlation })
            .await
    }
}

pub(crate) type SharedHost = Arc<dyn Host>;
