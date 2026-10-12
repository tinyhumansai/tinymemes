//! Content module with stateless transforms and admitted persistent operations. Configuration cannot admit a backend or profile.
use tinymemes_bus::{STATE_INTERFACE, STATE_PATH};

mod content_usage;
mod persistent;

struct ContentService;
#[async_trait::async_trait]
impl tinybus::service::Interface for ContentService {
    /// Advertise the fixed content contract declared in the module manifest.
    fn name(&self) -> tinybus::InterfaceName {
        STATE_INTERFACE.try_into().unwrap()
    }
    /// List the entire callable surface; no configuration grants extra methods.
    fn members(&self) -> Vec<tinybus::MemberName> {
        vec!["Apply".try_into().unwrap()]
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
/// Register the stateless operation at its declared path and bus name.
async fn setup(connection: tinybus::Connection) -> tinybus::Result<()> {
    connection
        .serve_at(STATE_PATH.try_into()?, ContentService)
        .await?;
    connection
        .serve_at(
            tinymemes_bus::MARKER_PATH.try_into()?,
            persistent::PersistentService {
                connection: connection.clone(),
            },
        )
        .await?;
    connection
        .serve_at(
            tinymemes_bus::CONTENT_USAGE_PATH.try_into()?,
            content_usage::ContentUsageService {
                connection: connection.clone(),
            },
        )
        .await?;
    connection.request_name(STATE_INTERFACE).await?;
    Ok(())
}
mod exports {
    use super::setup;
    #[cfg(not(test))]
    tinybus_module::module_export! {
        setup = setup,
        worker_threads = 1,
        provides = ["ai.tinyhumans.tinymemes.ContentState", "ai.tinyhumans.tinymemes.RemixMarkers1", "ai.tinyhumans.tinymemes.ContentUsage1"],
        methods = ["Apply", "MarkRemixed", "WasRemixed", "RecordSlangUsage", "RecordMemeUsage", "RemoveMeme"], signals = [], requires = [], optional = [], lazy = false,
    }

    #[cfg(test)]
    tinybus_module::module_export_static! {
        setup = setup,
        worker_threads = 1,
        provides = ["ai.tinyhumans.tinymemes.ContentState", "ai.tinyhumans.tinymemes.RemixMarkers1", "ai.tinyhumans.tinymemes.ContentUsage1"],
        methods = ["Apply", "MarkRemixed", "WasRemixed", "RecordSlangUsage", "RecordMemeUsage", "RemoveMeme"], signals = [], requires = [], optional = [], lazy = false,
    }
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
