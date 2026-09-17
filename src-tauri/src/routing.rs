//! The Role Matrix as the engine's `RoleRouting` (master plan S11): which
//! provider, model, permission ceiling, and workspace a role resolves to.
//!
//! # Why a snapshot instead of a database read per call
//!
//! `RoleRouting` is a synchronous trait -- the engine calls it while deciding
//! a dispatch window, not to do I/O -- and `nacc_storage`'s API is async. So
//! the commands that mutate role profiles (and startup) refresh this
//! snapshot, and the engine reads it without blocking. One process owns both
//! halves, so "the snapshot is current" is a property of the code, not a
//! cache-coherence problem.
//!
//! Reasoning and thinking are *not* part of the engine's routing contract
//! (that carries provider/model/permission only), so [`RoleSettings`] exposes
//! them here for the executor -- from the same rows, so a role cannot have
//! one model for routing and another for launching.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use nacc_domain::{
    ModelId, PermissionProfile, ProjectId, ProviderId, ReasoningLevel, RoleKind, RoleProfile,
    ThinkingMode, WorkflowRunId,
};
use nacc_orchestrator::{engine::role_key, RoleRouting};

/// A role's provider-independent execution settings.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RoleSettings {
    pub reasoning: ReasoningLevel,
    pub thinking: ThinkingMode,
}

impl Default for RoleSettings {
    fn default() -> Self {
        Self {
            // The provider's own default. Never a guessed concrete level:
            // master plan S10.1 forbids inventing an effort the user did not
            // choose, and every adapter maps `Auto` to "flag omitted".
            reasoning: ReasoningLevel::Auto,
            thinking: ThinkingMode::Auto,
        }
    }
}

#[derive(Default)]
pub struct RoleMatrixRouting {
    rows: RwLock<HashMap<String, RoleProfile>>,
    /// Where a project's agents work, chosen explicitly by the user when a
    /// run starts. A worktree lease replaces this once allocation exists;
    /// until then nothing is ever run in an implicitly-guessed directory.
    workspaces: RwLock<HashMap<ProjectId, PathBuf>>,
    /// Where one specific run's agents work: a leased worktree path, set at
    /// `start_workflow_run` when worktree isolation was requested. Overrides
    /// the per-project directory so every node of that run works in the same
    /// isolated tree and the primary checkout is never touched.
    run_workspaces: RwLock<HashMap<WorkflowRunId, PathBuf>>,
}

impl RoleMatrixRouting {
    /// Build from the persisted rows. Only enabled rows are routable -- a
    /// disabled profile is the GUI's "not in service" switch, and treating it
    /// as routable would make that switch decorative.
    pub fn new(profiles: Vec<RoleProfile>) -> Self {
        let routing = Self::default();
        routing.replace(profiles);
        routing
    }

    pub fn replace(&self, profiles: Vec<RoleProfile>) {
        let mut rows = self.rows.write().unwrap_or_else(|e| e.into_inner());
        rows.clear();
        for profile in profiles.into_iter().filter(|profile| profile.enabled) {
            // First enabled row for a role wins, deterministically: rows come
            // back sorted from storage, so this does not depend on hash
            // ordering or on which row was edited last.
            rows.entry(role_key(&profile.role_kind)).or_insert(profile);
        }
    }

    pub fn set_workspace(&self, project_id: ProjectId, path: impl Into<PathBuf>) {
        self.workspaces
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(project_id, path.into());
    }

    pub fn workspace_for_project(&self, project_id: ProjectId) -> Option<PathBuf> {
        self.workspaces
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&project_id)
            .cloned()
    }

    /// Pin one run to an explicit workspace (a leased worktree). While set,
    /// it wins over the per-project directory for that run only.
    pub fn set_run_workspace(&self, run_id: WorkflowRunId, path: impl Into<PathBuf>) {
        self.run_workspaces
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(run_id, path.into());
    }

    pub fn run_workspace_for(&self, run_id: WorkflowRunId) -> Option<PathBuf> {
        self.run_workspaces
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&run_id)
            .cloned()
    }

    /// Drop a run's workspace override -- called when its lease is released,
    /// so a finished run's path cannot outlive its worktree.
    pub fn clear_run_workspace(&self, run_id: WorkflowRunId) {
        self.run_workspaces
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&run_id);
    }

    pub fn settings_for(&self, role: &RoleKind) -> Option<RoleSettings> {
        self.rows
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&role_key(role))
            .map(|profile| RoleSettings {
                reasoning: profile.reasoning_level,
                thinking: profile.thinking_mode,
            })
    }

    /// How many roles are routable right now -- what the GUI shows as
    /// "roles ready to run".
    pub fn routable_role_count(&self) -> usize {
        self.rows.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Reload the persisted Role Matrix into this snapshot. Returns how many
    /// roles became routable, which is what the startup log reports.
    pub async fn refresh_from(&self, storage: &nacc_storage::Database) -> Result<usize, String> {
        let profiles = storage
            .list_role_profiles()
            .await
            .map_err(|e| e.to_string())?;
        self.replace(profiles);
        Ok(self.routable_role_count())
    }

    /// Validate a candidate workspace before a run is allowed to use it. A
    /// relative path or a missing directory would make every resulting
    /// "workspace" meaningless, so they are refused here rather than
    /// discovered by an agent writing to the wrong place.
    pub fn validate_workspace(path: &Path) -> Result<PathBuf, String> {
        if !path.is_absolute() {
            return Err(format!(
                "workspace must be an absolute path: {}",
                path.display()
            ));
        }
        if !path.is_dir() {
            return Err(format!(
                "workspace is not an existing directory: {}",
                path.display()
            ));
        }
        Ok(path.to_path_buf())
    }
}

impl RoleRouting for RoleMatrixRouting {
    fn provider_for(&self, role: &RoleKind) -> Option<ProviderId> {
        self.rows
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&role_key(role))
            .and_then(|profile| profile.provider_id)
    }

    fn model_for(&self, role: &RoleKind) -> Option<ModelId> {
        self.rows
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&role_key(role))
            .and_then(|profile| profile.model_id.clone())
    }

    fn permission_profile_for(&self, role: &RoleKind) -> Option<PermissionProfile> {
        self.rows
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&role_key(role))
            .map(|profile| profile.permission_profile)
    }

    /// The role row's configured fallback chain (master plan S11), consulted
    /// by the engine only when the row has no primary provider and the node
    /// declares none of its own. Empty for unassigned rows.
    fn fallback_chain_for(&self, role: &RoleKind) -> Vec<nacc_domain::NodeFallback> {
        self.rows
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(&role_key(role))
            .map(|profile| profile.fallbacks.clone())
            .unwrap_or_default()
    }

    fn workspace_for(
        &self,
        project_id: ProjectId,
        run_id: nacc_domain::WorkflowRunId,
        _node_key: &str,
    ) -> Option<PathBuf> {
        self.run_workspace_for(run_id)
            .or_else(|| self.workspace_for_project(project_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nacc_domain::RoleProfileId;

    fn profile(role: RoleKind, provider: Option<ProviderId>, enabled: bool) -> RoleProfile {
        RoleProfile {
            id: RoleProfileId::new(),
            name: "row".to_string(),
            role_kind: role,
            provider_id: provider,
            model_id: Some("user-model".into()),
            thinking_mode: ThinkingMode::On,
            reasoning_level: ReasoningLevel::High,
            permission_profile: PermissionProfile::ReadOnly,
            account_label: None,
            fallbacks: vec![],
            enabled,
            created_at_millis: 1,
            updated_at_millis: 1,
        }
    }

    #[test]
    fn enabled_rows_route_and_disabled_rows_do_not() {
        let routing = RoleMatrixRouting::new(vec![
            profile(RoleKind::RepositoryExplorer, Some(ProviderId::Claude), true),
            profile(RoleKind::TestEngineer, Some(ProviderId::Codex), false),
        ]);

        assert_eq!(
            routing.provider_for(&RoleKind::RepositoryExplorer),
            Some(ProviderId::Claude)
        );
        assert_eq!(
            routing.provider_for(&RoleKind::TestEngineer),
            None,
            "a disabled profile must not be routable"
        );
        assert_eq!(routing.routable_role_count(), 1);
    }

    #[test]
    fn an_unassigned_role_routes_to_nothing_rather_than_a_default() {
        let routing = RoleMatrixRouting::new(vec![profile(RoleKind::SecurityReviewer, None, true)]);
        assert_eq!(routing.provider_for(&RoleKind::SecurityReviewer), None);
        assert_eq!(
            routing.model_for(&RoleKind::SecurityReviewer).map(|m| m.0),
            Some("user-model".to_string()),
            "the model is still reported so the GUI can show what is missing"
        );
    }

    #[test]
    fn settings_come_from_the_same_row_as_routing() {
        let routing = RoleMatrixRouting::new(vec![profile(
            RoleKind::Integrator,
            Some(ProviderId::Codex),
            true,
        )]);
        assert_eq!(
            routing.settings_for(&RoleKind::Integrator),
            Some(RoleSettings {
                reasoning: ReasoningLevel::High,
                thinking: ThinkingMode::On,
            })
        );
        assert_eq!(routing.settings_for(&RoleKind::ReleaseManager), None);
    }

    #[test]
    fn replacing_rows_drops_stale_roles() {
        let routing = RoleMatrixRouting::new(vec![profile(
            RoleKind::Integrator,
            Some(ProviderId::Codex),
            true,
        )]);
        routing.replace(vec![]);
        assert_eq!(routing.provider_for(&RoleKind::Integrator), None);
        assert_eq!(routing.routable_role_count(), 0);
    }

    #[test]
    fn workspace_validation_refuses_relative_and_missing_paths() {
        assert!(RoleMatrixRouting::validate_workspace(Path::new("relative/dir")).is_err());
        assert!(RoleMatrixRouting::validate_workspace(Path::new("/does/not/exist/xyz")).is_err());

        let temp = std::env::temp_dir();
        assert_eq!(
            RoleMatrixRouting::validate_workspace(&temp).unwrap(),
            temp,
            "an existing absolute directory is accepted as-is"
        );
    }

    #[test]
    fn a_workspace_is_per_project() {
        let routing = RoleMatrixRouting::default();
        let first = ProjectId::new();
        let second = ProjectId::new();
        routing.set_workspace(first, "C:\\repo-a");
        assert_eq!(
            routing.workspace_for_project(first),
            Some(PathBuf::from("C:\\repo-a"))
        );
        assert_eq!(routing.workspace_for_project(second), None);
    }
}
