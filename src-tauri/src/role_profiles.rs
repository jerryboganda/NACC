use serde::{Deserialize, Serialize};
use tauri::State;

use nacc_domain::{
    ModelId, PermissionProfile, ProviderId, ReasoningLevel, RoleKind, RoleProfile, RoleProfileId,
    RoleProfileUpdate, ThinkingMode,
};

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

fn validate_profile(name: &str, permission: PermissionProfile) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("role profile name must not be empty".to_string());
    }
    if permission == PermissionProfile::TemporaryDangerFullAccess {
        return Err("temporary full access cannot be saved in a role profile".to_string());
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
    validate_profile(&args.name, args.permission_profile)?;
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
    validate_profile(&update.name, update.permission_profile)?;
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

    #[test]
    fn empty_names_are_rejected() {
        for name in ["", "   ", "\n\t"] {
            assert_eq!(
                validate_profile(name, PermissionProfile::ReadOnly),
                Err("role profile name must not be empty".to_string())
            );
        }
    }

    #[test]
    fn temporary_full_access_cannot_be_persisted() {
        assert_eq!(
            validate_profile("Explorer", PermissionProfile::TemporaryDangerFullAccess),
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
            assert_eq!(validate_profile("Explorer", permission), Ok(()));
        }
    }
}
