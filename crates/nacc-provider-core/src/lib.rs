//! The provider-adapter contract layer (master plan S8, S17's Phase 4).
//!
//! `nacc-provider-core` defines the `AgentProvider` trait every concrete
//! adapter (`nacc-provider-claude`, `-codex`, `-antigravity`, `-copilot`,
//! `-opencode`) implements, the normalized event vocabulary adapters emit,
//! the capability/health types the Role Matrix reads, the registry the
//! orchestrator resolves a provider id against, and the contract suite that
//! holds every adapter to the same bar.
//!
//! Depends on `nacc-domain` (the domain vocabulary) and `nacc-process` (the
//! contained-command runner every adapter executes through -- a generic
//! process API, not a CLI). Nothing in this crate knows about any specific
//! CLI, Tauri, or a GUI, and no adapter-specific logic lives here:
//! concrete provider crates and the orchestrator depend on this crate,
//! never the reverse (master plan S6).

mod capability;
pub mod cli;
pub mod contract;
mod error;
mod events;
mod health;
mod provider;
mod registry;

pub use capability::{
    AcpTransport, AuthProbe, CapabilitySnapshot, InstallationProbe, ModelDescriptor,
    ProviderInstallation, RuntimeLocation,
};
pub use cli::{
    CommandOutput, CommandRunner, FixtureCommandRunner, ProcessCommandRunner, RecordedInvocation,
};
pub use contract::{
    adapter_fixture_dir, run_contract_suite, ContractCheck, ContractFinding, ContractHarness,
    ContractReport, RecordingSink,
};
pub use error::{ProviderError, Result};
pub use events::{EventSink, ProviderEvent};
pub use health::ProviderHealth;
pub use provider::{
    AccountProfile, AgentInput, AgentProvider, AgentSessionHandle, CancellationMode,
    CapabilityContext, LaunchRequest, ProfileValidation, ResolvedAgentProfile, ResumeRequest,
    RuntimeProfile, SessionId, UsageObservation,
};
pub use registry::ProviderRegistry;
