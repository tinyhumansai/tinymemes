//! Crate-wide error type.

/// Boxed error returned by pluggable backends (Jev, chat models, meme sources).
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Why the engine could not remix a reply.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The Jev reading call failed.
    #[error("jev reading failed: {0}")]
    Reading(#[source] BoxError),
    /// Jev answered but omitted or mistyped a question we asked.
    #[error("jev response missing or mistyped answer `{0}`")]
    MissingAnswer(&'static str),
    /// The chat model behind the slang agent failed.
    #[error("slang agent model call failed: {0}")]
    Model(#[source] BoxError),
    /// The rewrite dropped content that must survive verbatim (code, links).
    #[error("rewrite dropped protected content")]
    ProtectedContentLost,
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;
