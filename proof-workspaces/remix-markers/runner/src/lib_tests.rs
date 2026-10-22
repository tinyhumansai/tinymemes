// Keep the external proof workspace on the exact compiled-module tests used by
// the package. This ensures it exercises the published host API and testkit
// ports instead of maintaining a second, stale storage adapter implementation.
#[path = "../../../../crates/tinymemes-module/tests/compiled_remix.rs"]
mod compiled_remix;
