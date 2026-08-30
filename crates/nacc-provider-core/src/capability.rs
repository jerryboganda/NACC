//! Capability discovery and installation/auth probing (master plan S8.3).
//!
//! A `CapabilitySnapshot` is what makes the Role Matrix's four switches
//! (role / model / thinking / reasoning effort) honest rather than
//! decorative: the GUI enables or disables each control per row based on
//! what the resolved provider+model actually reports here, and blocks
//! saving a combination the snapshot says is unsupported (master plan
//! S2.7, S10.1: "never silently downgrade").

use serde::{Deserialize, Serialize};

use nacc_domain::{ModelId, ProviderId, ReasoningLevel, ThinkingMode};

/// Result of probing whether a provider's CLI is installed on a given
/// runtime (native Windows, WSL2, Docker -- see `nacc-runtime`).
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct InstallationProbe {
    pub installed: bool,
    pub executable_path: Option<String>,
    /// Exact version string as reported by the CLI's own `--version`
    /// (never a hard-coded/assumed value -- master plan S2.7).
    pub version: Option<String>,
}

/// Result of probing whether a configured account profile is currently
/// authenticated. NACC never inspects or stores the credential itself
/// (master plan S8.4) -- only this fact, plus whatever label the provider
/// safely exposes.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct AuthProbe {
    pub authenticated: bool,
    /// Account label as safely exposed by the provider (e.g. a GitHub
    /// login or masked email) -- never a raw token or secret.
    pub account_label: Option<String>,
    pub detail: Option<String>,
}

/// One model as reported by a provider's own discovery mechanism. Every
/// field here must come from the provider, not be assumed -- see
/// `nacc_domain::ModelId`'s own doc comment.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct ModelDescriptor {
    pub id: ModelId,
    pub display_name: String,
    pub reasoning_levels: Vec<ReasoningLevel>,
    pub thinking: ThinkingMode,
    pub structured_output: bool,
    pub context_window_tokens: Option<u64>,
}

/// What runtime location a provider's CLI is being probed/launched under.
/// Kept intentionally small in Phase 1; `nacc-runtime` (Phase 3) owns the
/// real WSL2/Docker abstraction this references.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLocation {
    NativeWindows,
    Wsl2,
    Docker,
}

/// A timestamped, versioned capability record for one provider+account
/// combination (master plan S8.3's full field list -- reproduced here
/// with real types rather than the plan's prose list).
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct CapabilitySnapshot {
    pub provider: ProviderId,
    pub runtime: RuntimeLocation,
    pub installation: InstallationProbe,
    pub models: Vec<ModelDescriptor>,
    pub noninteractive_mode: bool,
    pub structured_json_output: bool,
    pub streaming_json_output: bool,
    pub interactive_pty: bool,
    pub session_resume: bool,
    pub custom_agents: bool,
    pub subagents: bool,
    pub mcp: bool,
    /// `Native` if the provider's own CLI speaks ACP directly, `Bridged`
    /// if only via a separate third-party package, `Unsupported`
    /// otherwise. See `docs/adr/0002-provider-transport.md` -- this field
    /// exists specifically because that ADR found the answer differs
    /// meaningfully per provider and must never be assumed uniform.
    pub acp_transport: AcpTransport,
    pub usage_reporting: bool,
    pub cancellation_documented: bool,
    /// Milliseconds since the Unix epoch. A plain `u64`, not
    /// `std::time::SystemTime`: specta's built-in mapping for
    /// `SystemTime` generates a `{ duration_since_epoch: ... }` object
    /// shape that would not match a simpler custom wire format, and
    /// serde itself has no default `SystemTime` impl to begin with --
    /// using a plain integer sidesteps both problems and is directly
    /// usable as a JS timestamp on the frontend.
    pub captured_at_millis: u64,
}

/// Milliseconds since the Unix epoch, for stamping a freshly captured
/// `CapabilitySnapshot`. A thin wrapper so call sites do not each repeat
/// the `SystemTime` -> `u64` conversion (and its fallible
/// `duration_since` call) inline.
pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum AcpTransport {
    Unsupported,
    Native,
    Bridged,
    Unverified,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_snapshot_roundtrips_through_json() {
        let snap = CapabilitySnapshot {
            provider: ProviderId::Copilot,
            runtime: RuntimeLocation::NativeWindows,
            installation: InstallationProbe {
                installed: true,
                executable_path: Some("copilot.exe".into()),
                version: Some("1.0.82".into()),
            },
            models: vec![],
            noninteractive_mode: true,
            structured_json_output: true,
            streaming_json_output: true,
            interactive_pty: false,
            session_resume: true,
            custom_agents: true,
            subagents: false,
            mcp: true,
            acp_transport: AcpTransport::Native,
            usage_reporting: true,
            cancellation_documented: false,
            captured_at_millis: now_millis(),
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: CapabilitySnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.provider, ProviderId::Copilot);
        assert_eq!(back.acp_transport, AcpTransport::Native);
        assert!(back.captured_at_millis > 0, "now_millis() must produce a real, nonzero timestamp");
    }
}
