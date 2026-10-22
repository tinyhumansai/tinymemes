//! Content operations and persistent remix markers over the host's storage.
//!
//! The module owns its collection setup and migration when it starts; hosts
//! provide the scoped storage ports but do not compose module migrations.
use tinymemes_bus::{STATE_INTERFACE, STATE_PATH};

/// Bus name the module claims.
pub const BUS_NAME: &str = STATE_INTERFACE;

mod docs;
mod engine;
mod host;
mod persistent;
mod remix;
mod storage;

struct ContentService;
#[async_trait::async_trait]
impl tinybus::service::Interface for ContentService {
    /// Advertise the fixed content contract declared in the module manifest.
    fn name(&self) -> tinybus::InterfaceName {
        STATE_INTERFACE.try_into().unwrap()
    }
    /// List the entire callable surface; no configuration grants extra methods.
    fn members(&self) -> Vec<tinybus::MemberName> {
        vec![tinymemes_bus::APPLY_METHOD.try_into().unwrap()]
    }
    /// Decode positional arguments and invoke owning pure operations with fixed errors.
    async fn call(
        &self,
        member: &tinybus::MemberName,
        args: serde_json::Value,
    ) -> tinybus::Result<serde_json::Value> {
        if member.as_str() != "Apply" {
            return Err(tinybus::Error::failed("unknown content operation"));
        }
        let (request,): (tinymemes_bus::ApplyRequest,) = serde_json::from_value(args)
            .map_err(|_| tinybus::Error::failed("invalid content request"))?;
        serde_json::to_value(tinymemes::state::apply(request))
            .map_err(|_| tinybus::Error::failed("invalid content response"))
    }
}
/// Bind host storage, prepare this owner's collections, then serve operations.
async fn setup(connection: tinybus::Connection) -> tinybus::Result<()> {
    let storage = tinystoragedrivers_busproxy::connect_storage(connection.clone())
        .await
        .map_err(|error| tinybus::Error::failed(format!("connect storage: {error}")))?;
    storage::prepare(&storage)
        .await
        .map_err(|error| tinybus::Error::failed(format!("prepare memes storage: {error}")))?;
    connection
        .serve_at(STATE_PATH.try_into()?, ContentService)
        .await?;
    connection
        .serve_at(
            tinymemes_bus::MARKER_PATH.try_into()?,
            persistent::PersistentService {
                documents: std::sync::Arc::clone(storage.documents()),
            },
        )
        .await?;
    connection
        .serve_at(
            tinymemes_bus::REMIX_PATH.try_into()?,
            remix::RemixService {
                connection: connection.clone(),
                drain: Default::default(),
            },
        )
        .await?;
    connection.request_name(STATE_INTERFACE).await?;
    Ok(())
}
macro_rules! module_declaration {
    ($export:ident) => {
        tinybus_module::$export! {
            setup = setup,
            worker_threads = 1,
            provides = ["ai.tinyhumans.tinymemes.ContentState", "ai.tinyhumans.tinymemes.RemixMarkers1"],
            methods = ["Apply", "MarkRemixed", "WasRemixed", "Remix"], signals = [], requires = [], optional = ["ai.tinyhumans.tinymemes.Host"], lazy = false,
        }
    };
}

/// The exported entry points. The default build exports the shared C names a
/// dynamic loader resolves; `static-link` (and unit tests) export Rust entry
/// points instead, so a host can link the module and admit it in-process.
#[allow(missing_docs, unreachable_pub)]
pub mod exports {
    use super::setup;

    #[cfg(not(test))]
    module_declaration!(module_export_optional_static);

    #[cfg(test)]
    module_declaration!(module_export_static);
}

/// Entry points and parsed manifest for a host that links this module.
#[cfg(any(test, feature = "static-link"))]
pub use exports::linked_module;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "test_support.rs"]
mod test_support;
