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
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct RoleProfileMutation {
    pub profile: RoleProfile,
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

#[tauri::command]
#[specta::specta]
pub async fn list_role_profiles(state: State<'_, AppState>) -> Result<Vec<RoleProfile>, String> {
    state
        .storage
        .list_role_profiles()
        .await
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
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(RoleProfileMutation { profile })
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
    Ok(RoleProfileMutation { profile })
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
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_role_profile(
    id: RoleProfileId,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    state
        .storage
        .delete_role_profile(id)
        .await
        .map_err(|e| e.to_string())
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
