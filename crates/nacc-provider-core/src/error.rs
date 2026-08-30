//! Errors an `AgentProvider` adapter can return. Deliberately a single
//! shared enum (rather than per-method errors): every adapter contract
//! test in the master plan's S6 exercises the same failure shapes
//! (installation probe, unauthenticated state, malformed output, timeout,
//! cancellation, permission denial, rate limit, structured handoff, CLI
//! incompatibility) regardless of which provider is under test, so a
//! shared vocabulary is what makes those tests comparable across adapters.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, ProviderError>;

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider executable not found on this runtime: {detail}")]
    NotInstalled { detail: String },

    #[error("provider is not authenticated: {detail}")]
    Unauthenticated { detail: String },

    /// A credential exists but is not valid for the requested mode --
    /// distinct from `Unauthenticated`. Added from live Phase 0 evidence:
    /// Copilot CLI's ACP mode rejected a valid `gh` classic PAT with a
    /// precise "not supported in this mode" error, and Gemini CLI rejected
    /// a valid-but-ineligible account tier. Neither is "no credential" --
    /// both are "this credential doesn't authorize this operation."
    #[error("credential present but not valid for this mode: {detail}")]
    IneligibleCredential { detail: String },

    #[error("provider output did not match the expected structured format: {detail}")]
    MalformedOutput { detail: String },

    #[error("provider did not respond within the configured timeout")]
    Timeout,

    #[error("operation was cancelled")]
    Cancelled,

    #[error("provider denied a requested permission: {detail}")]
    PermissionDenied { detail: String },

    #[error("provider reported a rate limit: {detail}")]
    RateLimited { detail: String },

    #[error("installed provider CLI version is incompatible with this adapter: {detail}")]
    IncompatibleVersion { detail: String },

    #[error("requested profile setting is not supported by this provider/model: {detail}")]
    UnsupportedSetting { detail: String },

    #[error("underlying process error: {0}")]
    Process(String),

    #[error("{0}")]
    Other(String),
}
