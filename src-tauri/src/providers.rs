//! Provider detection IPC (master plan S8.3, S17.4): probes the real
//! provider CLIs through the registered adapters and persists each result,
//! so the Providers page shows observations (command + the CLI's version
//! string), not authentication or readiness. The current adapters report
//! command names, not resolved absolute executable paths. Only adapters with
//! real probing are registered; other providers are not presented as supported.

use serde::{Deserialize, Serialize};
use tauri::State;

use nacc_domain::{ProviderId, ReasoningLevel, ThinkingMode};
use nacc_provider_core::{
    AccountProfile, CapabilityContext, CapabilitySnapshot, ProviderInstallation, ProviderRegistry,
    RuntimeLocation, RuntimeProfile,
};

use crate::AppState;

/// The adapters this build can actually probe and (later) launch. Kept as a
/// function so tests and future callers construct the exact registry the
/// running app uses.
pub fn build_registry() -> ProviderRegistry {
    let runner: std::sync::Arc<dyn nacc_provider_core::CommandRunner> =
        std::sync::Arc::new(nacc_provider_core::ProcessCommandRunner::new());
    let mut registry = ProviderRegistry::new();
    registry
        .register(std::sync::Arc::new(
            nacc_provider_claude::ClaudeCodeProvider::new(runner.clone()),
        ))
        .expect("the built-in adapter set must never claim a provider twice");
    registry
        .register(std::sync::Arc::new(
            nacc_provider_codex::CodexProvider::new(runner),
        ))
        .expect("the built-in adapter set is disjoint by construction");
    registry
}

/// IPC view over [`ProviderInstallation`]: same shape as the stored record
/// with string timestamps (the same convention `RoleProfileView` uses, so
/// the webview never receives a bare integer epoch it must format).
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct ProviderInstallationView {
    pub provider: ProviderId,
    pub runtime: RuntimeLocation,
    pub installed: bool,
    pub executable_path: Option<String>,
    pub version: Option<String>,
    pub detected_at_millis: String,
}

impl From<ProviderInstallation> for ProviderInstallationView {
    fn from(installation: ProviderInstallation) -> Self {
        Self {
            provider: installation.provider,
            runtime: installation.runtime,
            installed: installation.probe.installed,
            executable_path: installation.probe.executable_path,
            version: installation.probe.version,
            detected_at_millis: installation.detected_at_millis.to_string(),
        }
    }
}

/// The one runtime probes run under today: the native Windows host, the
/// runtime this desktop build actually executes in. WSL2/Docker detection
/// is added when `nacc-runtime`'s bridge is wired into the app (Phase 8
/// scope); until then the GUI does not offer those buttons.
fn native_runtime() -> RuntimeProfile {
    RuntimeProfile {
        location: RuntimeLocation::NativeWindows,
        working_directory: ".".to_string(),
    }
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct DetectProviderArgs {
    pub provider_id: ProviderId,
}

#[tauri::command]
#[specta::specta]
pub async fn detect_provider(
    args: DetectProviderArgs,
    state: State<'_, AppState>,
) -> Result<ProviderInstallationView, String> {
    let provider = state
        .providers
        .require(args.provider_id)
        .map_err(|e| e.to_string())?;
    // Dropping capture on timeout drops SupervisedProcess, whose Drop
    // terminates the contained process tree (nacc-process).
    let probe = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        provider.probe_installation(&native_runtime()),
    )
    .await
    .map_err(|_| {
        "Provider detection timed out after 15 seconds; previous observation retained".to_string()
    })?
    .map_err(|e| e.to_string())?;
    let installation = ProviderInstallation {
        provider: args.provider_id,
        runtime: RuntimeLocation::NativeWindows,
        probe,
        detected_at_millis: now_millis(),
    };
    state
        .storage
        .upsert_provider_installation(&installation)
        .await
        .map_err(|e| e.to_string())?;
    Ok(ProviderInstallationView::from(installation))
}

#[tauri::command]
#[specta::specta]
pub async fn list_provider_installations(
    state: State<'_, AppState>,
) -> Result<Vec<ProviderInstallationView>, String> {
    state
        .storage
        .list_provider_installations()
        .await
        .map(|rows| {
            rows.into_iter()
                .map(ProviderInstallationView::from)
                .collect()
        })
        .map_err(|e| e.to_string())
}

/// Point-in-time sign-in status for one provider. Not persisted: the
/// adapters check the native credential store's *existence* live on every
/// call (contents never read, master plan S8.4), so a stored answer would
/// only go stale; the frontend labels this as "checked now".
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct AuthProbeView {
    pub provider: ProviderId,
    pub authenticated: bool,
    pub account_label: Option<String>,
    pub detail: Option<String>,
    pub checked_at_millis: String,
}

#[tauri::command]
#[specta::specta]
pub async fn check_provider_auth(
    args: DetectProviderArgs,
    state: State<'_, AppState>,
) -> Result<AuthProbeView, String> {
    let provider = state
        .providers
        .require(args.provider_id)
        .map_err(|e| e.to_string())?;
    // The adapters ignore the account parameter today (one native store per
    // provider on this machine); a real label is passed so multi-account
    // probing needs no command-shape change later.
    let probe = provider
        .probe_authentication(&nacc_provider_core::AccountProfile {
            id: nacc_domain::ProviderAccountId::new(),
            provider: args.provider_id,
            label: String::new(),
        })
        .await
        .map_err(|e| e.to_string())?;
    Ok(AuthProbeView {
        provider: args.provider_id,
        authenticated: probe.authenticated,
        account_label: probe.account_label,
        detail: probe.detail,
        checked_at_millis: now_millis().to_string(),
    })
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// IPC view over one discovered model: exactly what the adapter reported
/// (S10.1/S2.7 -- shown as-is, never normalized into a NACC-owned catalog).
/// `reasoning_levels` is the honest list a GUI may offer for the model;
/// an empty list means "no reasoning control has been verified".
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct ModelCapabilityView {
    pub id: String,
    pub display_name: String,
    pub reasoning_levels: Vec<ReasoningLevel>,
    pub thinking: ThinkingMode,
    /// Context window in tokens as reported, stringified for IPC. `None`
    /// means the provider did not report one -- shown as unknown, not guessed.
    pub context_window_tokens: Option<String>,
}

/// IPC view over a [`CapabilitySnapshot`] (master plan S8.3): the timestamped
/// facts a GUI needs to enable or disable model-aware controls honestly.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct CapabilitySnapshotView {
    pub provider: ProviderId,
    pub installed: bool,
    pub version: Option<String>,
    pub authenticated: bool,
    /// Adapter-declared health, snake_case ("ready", "not_installed",
    /// "unauthenticated", ...). A GUI shows it; nothing infers readiness.
    pub health: String,
    pub models: Vec<ModelCapabilityView>,
    pub checked_at_millis: String,
}

fn snapshot_view(snapshot: &CapabilitySnapshot) -> CapabilitySnapshotView {
    CapabilitySnapshotView {
        provider: snapshot.provider,
        installed: snapshot.installation.installed,
        version: snapshot.installation.version.clone(),
        authenticated: snapshot.auth.authenticated,
        health: match snapshot.health {
            nacc_provider_core::ProviderHealth::Ready => "ready",
            nacc_provider_core::ProviderHealth::NotInstalled => "not_installed",
            nacc_provider_core::ProviderHealth::Unauthenticated => "unauthenticated",
            nacc_provider_core::ProviderHealth::IneligibleCredential { .. } => {
                "ineligible_credential"
            }
            nacc_provider_core::ProviderHealth::IncompatibleVersion { .. } => {
                "incompatible_version"
            }
        }
        .to_string(),
        models: snapshot
            .models
            .iter()
            .map(|model| ModelCapabilityView {
                id: model.id.0.clone(),
                display_name: model.display_name.clone(),
                reasoning_levels: model.reasoning_levels.clone(),
                thinking: model.thinking,
                context_window_tokens: model.context_window_tokens.map(|tokens| tokens.to_string()),
            })
            .collect(),
        checked_at_millis: snapshot.captured_at_millis.to_string(),
    }
}

/// Probe one provider's capabilities live through its adapter and persist the
/// snapshot (master plan S8.3: "store a timestamped snapshot and refresh it
/// on demand"). Only registered adapters can be probed; an unknown provider
/// is a typed error, never a guessed answer.
#[tauri::command]
#[specta::specta]
pub async fn probe_provider_capabilities(
    args: DetectProviderArgs,
    state: State<'_, AppState>,
) -> Result<CapabilitySnapshotView, String> {
    let provider_id = args.provider_id;
    let provider = state
        .providers
        .require(provider_id)
        .map_err(|e| e.to_string())?
        .clone();
    let context = CapabilityContext {
        account: AccountProfile {
            id: nacc_domain::ProviderAccountId::new(),
            provider: provider_id,
            // A label is a display fact; nothing has discovered one yet.
            label: String::new(),
        },
        runtime: RuntimeProfile {
            location: RuntimeLocation::NativeWindows,
            working_directory: String::new(),
        },
    };
    let mut snapshot = provider
        .capabilities(&context)
        .await
        .map_err(|e| e.to_string())?;
    snapshot.captured_at_millis = now_millis();
    state
        .storage
        .record_capability_snapshot(&snapshot)
        .await
        .map_err(|e| e.to_string())?;
    Ok(snapshot_view(&snapshot))
}

/// The most recent persisted snapshot for one provider on native Windows,
/// if any. This is what the Role Matrix reads to decide whether a
/// thinking/reasoning control may honestly be enabled -- and a GUI must
/// treat `None` as "disabled with an explanation", never as a default.
#[tauri::command]
#[specta::specta]
pub async fn latest_provider_capabilities(
    args: DetectProviderArgs,
    state: State<'_, AppState>,
) -> Result<Option<CapabilitySnapshotView>, String> {
    state
        .storage
        .latest_capability_snapshot(args.provider_id, RuntimeLocation::NativeWindows)
        .await
        .map(|stored| stored.as_ref().map(snapshot_view))
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_holds_exactly_the_real_adapters() {
        let registry = build_registry();
        assert_eq!(registry.ids(), vec![ProviderId::Claude, ProviderId::Codex]);
    }

    #[test]
    fn installation_view_stringifies_the_timestamp() {
        let view = ProviderInstallationView::from(ProviderInstallation {
            provider: ProviderId::Codex,
            runtime: RuntimeLocation::NativeWindows,
            probe: nacc_provider_core::InstallationProbe {
                installed: true,
                executable_path: Some("codex.exe".into()),
                version: Some("0.149.1".into()),
            },
            detected_at_millis: 42,
        });
        assert_eq!(view.detected_at_millis, "42");
        assert!(view.installed);
    }
}
