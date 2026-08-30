//! The provider-adapter contract layer (master plan S8).
//!
//! `nacc-provider-core` defines the `AgentProvider` trait every concrete
//! adapter (`nacc-provider-claude`, `-codex`, `-antigravity`, `-copilot`,
//! `-opencode`) implements, the normalized event vocabulary adapters emit,
//! and the capability-snapshot types the Role Matrix reads to decide which
//! of its four per-role switches (role / model / thinking / reasoning
//! effort) are actually live for a given provider+model combination.
//!
//! Depends only on `nacc-domain`. Nothing in this crate knows about any
//! specific CLI, Tauri, or a GUI -- concrete provider crates and the
//! orchestrator depend on this crate, never the reverse (master plan S6).

mod capability;
mod error;
mod events;
mod provider;

pub use capability::{
    AcpTransport, AuthProbe, CapabilitySnapshot, InstallationProbe, ModelDescriptor,
    RuntimeLocation,
};
pub use error::{ProviderError, Result};
pub use events::{EventSink, ProviderEvent};
pub use provider::{
    AccountProfile, AgentInput, AgentProvider, AgentSessionHandle, CancellationMode,
    CapabilityContext, LaunchRequest, ProfileValidation, ResolvedAgentProfile, ResumeRequest,
    RuntimeProfile, SessionId, UsageObservation,
};
