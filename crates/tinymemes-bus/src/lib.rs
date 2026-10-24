//! Minimal content-only TinyMemes contract. No inference or storage implementation.
//!
//! This crate is plain data: interface and member names, versions and
//! request/response types. It is synchronous and does no I/O, and depends on
//! not on `tinybus`. Pure storage declarations are available from
//! [`storage`]; the typed bus clients live in `tinymemes-client`.
//!
//! Supplied snapshots and revision numbers are data. They confer no authority,
//! admission, durable CAS guarantee, research verification or source ownership.
mod content_usage;
pub use content_usage::*;
mod persistent;
pub use persistent::*;
mod intent;
mod records;
mod remix;
mod state;
pub mod storage;
mod tier;
pub use intent::Intent;
pub use records::*;
pub use remix::*;
pub use state::*;
pub use tier::Tier;

/// Stateless content service name and interface.
pub const STATE_INTERFACE: &str = "ai.tinyhumans.tinymemes.ContentState";
/// Stateless content service object path.
pub const STATE_PATH: &str = "/ai/tinyhumans/tinymemes/ContentState";
/// Member of [`STATE_INTERFACE`] that transforms supplied content.
pub const APPLY_METHOD: &str = "Apply";
