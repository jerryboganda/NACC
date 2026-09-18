use serde::{Deserialize, Serialize};
use tauri::State;

use nacc_domain::{
    ModelId, NodeFallback, PermissionProfile, ProviderId, ReasoningLevel, RoleKind, RoleProfile,
    RoleProfileId, RoleProfileUpdate, ThinkingMode,
};
use nacc_provider_core::{ProviderRegistry, RuntimeLocation};

use crate::AppState;

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct CreateRoleProfileArgs {
    pub name: String,
    pub role_kind: RoleKind,
    pub provider_id: Option<ProviderId>,
    pub model_id: Option<ModelId>,
    pub thinking_mode: ThinkingMode,
    pub reasoning_level: ReasoningLevel,
    pub permission_profile: PermissionProfile,
    /// A user-typed account preference label -- never a credential (S11).
    pub account_label: Option<String>,
    /// The row's fallback chain, consulted only when no primary provider is
    /// assigned and the node declares none of its own (S11, S14.4).
    pub fallbacks: Vec<nacc_domain::NodeFallback>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct RoleProfileView {
    pub id: RoleProfileId,
    pub name: String,
    pub role_kind: RoleKind,
    pub provider_id: Option<ProviderId>,
    pub model_id: Option<ModelId>,
    pub thinking_mode: ThinkingMode,
    pub reasoning_level: ReasoningLevel,
    pub permission_profile: PermissionProfile,
    pub account_label: Option<String>,
    pub fallbacks: Vec<nacc_domain::NodeFallback>,
    pub enabled: bool,
    pub created_at_millis: String,
    pub updated_at_millis: String,
}

impl From<RoleProfile> for RoleProfileView {
    fn from(profile: RoleProfile) -> Self {
        Self {
            id: profile.id,
            name: profile.name,
            role_kind: profile.role_kind,
            provider_id: profile.provider_id,
            model_id: profile.model_id,
            thinking_mode: profile.thinking_mode,
            reasoning_level: profile.reasoning_level,
            permission_profile: profile.permission_profile,
            account_label: profile.account_label,
            fallbacks: profile.fallbacks,
            enabled: profile.enabled,
            created_at_millis: profile.created_at_millis.to_string(),
            updated_at_millis: profile.updated_at_millis.to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct RoleProfileMutation {
    pub profile: RoleProfileView,
}

fn validate_profile_basics(
    name: &str,
    permission: PermissionProfile,
    provider_id: Option<ProviderId>,
    model_id: Option<&ModelId>,
    fallbacks: &[NodeFallback],
    providers: &ProviderRegistry,
) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("role profile name must not be empty".to_string());
    }
    if permission == PermissionProfile::TemporaryDangerFullAccess {
        return Err("temporary full access cannot be saved in a role profile".to_string());
    }

    if model_id.is_some() && provider_id.is_none() {
        return Err("a model cannot be saved without a primary provider".to_string());
    }

    if let Some(provider_id) = provider_id {
        providers.require(provider_id).map_err(|_| {
            format!(
                "provider {provider_id} is not runnable in this build; available providers: {}",
                providers
                    .ids()
                    .into_iter()
                    .map(|provider| provider.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    }

    for fallback in fallbacks {
        providers.require(fallback.provider_id).map_err(|_| {
            format!(
                "fallback provider {} is not runnable in this build; available providers: {}",
                fallback.provider_id,
                providers
                    .ids()
                    .into_iter()
                    .map(|provider| provider.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;
    }

    Ok(())
}

async fn validate_model_assignment(
    provider_id: ProviderId,
    model_id: &ModelId,
    context: &str,
    storage: &nacc_storage::Database,
) -> Result<(), String> {
    if model_id.0.trim().is_empty() {
        return Err(format!("{context} model ID must not be blank"));
    }

    let snapshot = storage
        .latest_capability_snapshot(provider_id, RuntimeLocation::NativeWindows)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| {
            format!(
                "{context} model {} cannot be verified for {provider_id}; run Check capabilities for this provider first",
                model_id.0
            )
        })?;

    if !snapshot.models.iter().any(|model| model.id == *model_id) {
        return Err(format!(
            "{context} model {} is not present in the latest verified {provider_id} capability snapshot",
            model_id.0
        ));
    }

    Ok(())
}

async fn validate_profile(
    name: &str,
    permission: PermissionProfile,
    provider_id: Option<ProviderId>,
    model_id: Option<&ModelId>,
    fallbacks: &[NodeFallback],
    providers: &ProviderRegistry,
    storage: &nacc_storage::Database,
) -> Result<(), String> {
    validate_profile_basics(
        name,
        permission,
        provider_id,
        model_id,
        fallbacks,
        providers,
    )?;

    if let (Some(provider_id), Some(model_id)) = (provider_id, model_id) {
        validate_model_assignment(provider_id, model_id, "primary", storage).await?;
    }

    for (index, fallback) in fallbacks.iter().enumerate() {
        if let Some(model_id) = fallback.model_id.as_ref() {
            validate_model_assignment(
                fallback.provider_id,
                model_id,
                &format!("fallback {}", index + 1),
                storage,
            )
            .await?;
        }
    }

    Ok(())
}

/// Reloads the engine's routing snapshot from storage after a successful
/// mutation, so a run started right after a Role Matrix edit routes by the
/// rules the GUI now shows, not the ones it showed at startup. A failed
/// refresh must not fail the command -- the storage mutation already
/// happened -- but it must be loud: the snapshot going stale silently is
/// exactly the inconsistency this exists to prevent.
async fn refresh_routing(state: &State<'_, AppState>) {
    if let Err(err) = state.routing.refresh_from(&state.storage).await {
        tracing::warn!(
            error = %err,
            "role profile mutated but the routing snapshot refresh failed"
        );
    }
}

#[tauri::command]
#[specta::specta]
pub async fn list_role_profiles(
    state: State<'_, AppState>,
) -> Result<Vec<RoleProfileView>, String> {
    state
        .storage
        .list_role_profiles()
        .await
        .map(|profiles| profiles.into_iter().map(RoleProfileView::from).collect())
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn create_role_profile(
    args: CreateRoleProfileArgs,
    state: State<'_, AppState>,
) -> Result<RoleProfileMutation, String> {
    validate_profile(
        &args.name,
        args.permission_profile,
        args.provider_id,
        args.model_id.as_ref(),
        &args.fallbacks,
        &state.providers,
        &state.storage,
    )
    .await?;
    let profile = state
        .storage
        .create_role_profile(
            args.name,
            args.role_kind,
            args.provider_id,
            args.model_id,
            args.thinking_mode,
            args.reasoning_level,
            args.permission_profile,
            args.account_label,
            args.fallbacks,
        )
        .await
        .map_err(|e| e.to_string())?;
    refresh_routing(&state).await;
    Ok(RoleProfileMutation {
        profile: RoleProfileView::from(profile),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn update_role_profile(
    id: RoleProfileId,
    update: RoleProfileUpdate,
    state: State<'_, AppState>,
) -> Result<RoleProfileMutation, String> {
    validate_profile(
        &update.name,
        update.permission_profile,
        update.provider_id,
        update.model_id.as_ref(),
        &update.fallbacks,
        &state.providers,
        &state.storage,
    )
    .await?;
    let profile = state
        .storage
        .update_role_profile(id, update)
        .await
        .map_err(|e| e.to_string())?;
    refresh_routing(&state).await;
    Ok(RoleProfileMutation {
        profile: RoleProfileView::from(profile),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn set_role_profile_enabled(
    id: RoleProfileId,
    enabled: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if enabled {
        let profile = state
            .storage
            .get_role_profile(id)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("role profile {id} was not found"))?;
        validate_profile(
            &profile.name,
            profile.permission_profile,
            profile.provider_id,
            profile.model_id.as_ref(),
            &profile.fallbacks,
            &state.providers,
            &state.storage,
        )
        .await?;
    }
    state
        .storage
        .set_role_profile_enabled(id, enabled)
        .await
        .map_err(|e| e.to_string())?;
    refresh_routing(&state).await;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_role_profile(
    id: RoleProfileId,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let deleted = state
        .storage
        .delete_role_profile(id)
        .await
        .map_err(|e| e.to_string())?;
    refresh_routing(&state).await;
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> ProviderRegistry {
        crate::providers::build_registry()
    }

    #[test]
    fn empty_names_are_rejected() {
        for name in ["", "   ", "\n\t"] {
            assert_eq!(
                validate_profile_basics(
                    name,
                    PermissionProfile::ReadOnly,
                    None,
                    None,
                    &[],
                    &registry()
                ),
                Err("role profile name must not be empty".to_string())
            );
        }
    }

    #[test]
    fn temporary_full_access_cannot_be_persisted() {
        assert_eq!(
            validate_profile_basics(
                "Explorer",
                PermissionProfile::TemporaryDangerFullAccess,
                None,
                None,
                &[],
                &registry()
            ),
            Err("temporary full access cannot be saved in a role profile".to_string())
        );
    }

    #[test]
    fn valid_names_and_persistent_permissions_are_accepted() {
        for permission in [
            PermissionProfile::ReadOnly,
            PermissionProfile::PlanOnly,
            PermissionProfile::AutonomousWorktree,
            PermissionProfile::RepositoryMaintainer,
            PermissionProfile::CiMaintainer,
            PermissionProfile::ReleaseCandidate,
        ] {
            assert_eq!(
                validate_profile_basics("Explorer", permission, None, None, &[], &registry()),
                Ok(())
            );
        }
    }

    #[test]
    fn model_without_provider_and_unregistered_providers_are_rejected() {
        let providers = registry();
        let model = ModelId::from("gpt-5-codex");
        assert_eq!(
            validate_profile_basics(
                "Explorer",
                PermissionProfile::ReadOnly,
                None,
                Some(&model),
                &[],
                &providers
            ),
            Err("a model cannot be saved without a primary provider".to_string())
        );

        let err = validate_profile_basics(
            "Explorer",
            PermissionProfile::ReadOnly,
            Some(ProviderId::Copilot),
            None,
            &[],
            &providers,
        )
        .expect_err("an adapter stub must not become a runnable role assignment");
        assert!(err.contains("copilot is not runnable"));
        assert!(err.contains("claude, codex"));
    }

    #[tokio::test]
    async fn explicit_model_requires_and_matches_latest_capability_snapshot() {
        use nacc_provider_core::{
            AcpTransport, AuthProbe, CapabilitySnapshot, InstallationProbe, ModelDescriptor,
            ProviderHealth,
        };

        let providers = registry();
        let storage = nacc_storage::Database::open_in_memory().expect("test database");
        let model = ModelId::from("gpt-5-codex");

        let missing = validate_profile(
            "Explorer",
            PermissionProfile::ReadOnly,
            Some(ProviderId::Codex),
            Some(&model),
            &[],
            &providers,
            &storage,
        )
        .await
        .expect_err("an unverified model must fail closed");
        assert!(missing.contains("run Check capabilities"));

        storage
            .record_capability_snapshot(&CapabilitySnapshot {
                provider: ProviderId::Codex,
                runtime: RuntimeLocation::NativeWindows,
                installation: InstallationProbe {
                    installed: true,
                    executable_path: Some("codex.exe".to_string()),
                    version: Some("test".to_string()),
                },
                auth: AuthProbe {
                    authenticated: true,
                    account_label: None,
                    detail: None,
                },
                health: ProviderHealth::Ready,
                models: vec![ModelDescriptor {
                    id: model.clone(),
                    display_name: model.0.clone(),
                    reasoning_levels: vec![ReasoningLevel::High],
                    thinking: ThinkingMode::Unsupported,
                    structured_output: true,
                    context_window_tokens: None,
                }],
                noninteractive_mode: true,
                structured_json_output: true,
                streaming_json_output: true,
                interactive_pty: false,
                session_resume: false,
                custom_agents: false,
                subagents: false,
                mcp: false,
                acp_transport: AcpTransport::Unverified,
                usage_reporting: false,
                cancellation_documented: true,
                captured_at_millis: 1,
            })
            .await
            .expect("store capability snapshot");

        validate_profile(
            "Explorer",
            PermissionProfile::ReadOnly,
            Some(ProviderId::Codex),
            Some(&model),
            &[],
            &providers,
            &storage,
        )
        .await
        .expect("model listed by the latest snapshot must be accepted");

        let unknown = ModelId::from("not-in-snapshot");
        let err = validate_profile(
            "Explorer",
            PermissionProfile::ReadOnly,
            Some(ProviderId::Codex),
            Some(&unknown),
            &[],
            &providers,
            &storage,
        )
        .await
        .expect_err("unknown model must fail closed");
        assert!(err.contains("not present in the latest verified codex capability snapshot"));
    }
}
