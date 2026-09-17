//! Setup Wizard IPC (master plan S17.1): prerequisite detection and
//! read-only workspace validation for the first-run flow. Every step here
//! reports *observations* -- what is installed, at what version, whether a
//! directory is a repository -- and never performs an installation or a
//! mutation. Provider detection, authentication checks, and capability
//! probes live in `providers.rs`; the wizard composes them.

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::AppState;

/// One prerequisite's honest state: detected with a version, or explicitly
/// not detected. There is no "assumed fine" third state.
#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct PrerequisiteView {
    pub name: String,
    pub detected: bool,
    /// The tool's own version string, verbatim (e.g. "git version 2.53.0").
    pub version: Option<String>,
    /// Why detection did not run or what failed, when worth showing.
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct CheckPrerequisitesArgs {
    /// Probe `gh` too? The wizard asks, because `gh` detection shells out
    /// and the wizard should only spend that on steps the user is on.
    pub include_gh: bool,
}

/// Detect the local prerequisites the Setup Wizard names (master plan
/// S17.1 step 2): git, optionally the GitHub CLI, and the registered agent
/// CLIs (those are `providers::detect_provider`'s job -- the wizard lists
/// them from the persisted installations rather than re-probing here).
#[tauri::command]
#[specta::specta]
pub async fn check_prerequisites(
    args: CheckPrerequisitesArgs,
    _state: State<'_, AppState>,
) -> Result<Vec<PrerequisiteView>, String> {
    let mut rows = Vec::new();

    match nacc_git::git_version().await {
        Ok(version) => rows.push(PrerequisiteView {
            name: "git".to_string(),
            detected: true,
            version: Some(version),
            detail: None,
        }),
        Err(err) => rows.push(PrerequisiteView {
            name: "git".to_string(),
            detected: false,
            version: None,
            detail: Some(err.to_string()),
        }),
    }

    if args.include_gh {
        match nacc_github::gh_version().await {
            Ok(version) if !version.is_empty() => rows.push(PrerequisiteView {
                name: "gh".to_string(),
                detected: true,
                version: Some(version),
                detail: None,
            }),
            Ok(_) => rows.push(PrerequisiteView {
                name: "gh".to_string(),
                detected: false,
                version: None,
                detail: Some("gh produced no version output".to_string()),
            }),
            Err(err) => rows.push(PrerequisiteView {
                name: "gh".to_string(),
                detected: false,
                version: None,
                detail: Some(err.to_string()),
            }),
        }
    }

    Ok(rows)
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct CheckWorkspaceArgs {
    /// Absolute path to validate.
    pub path: String,
}

/// Read-only validation of a candidate workspace (master plan S17.1 step 12):
/// is it an absolute existing directory, and is it inside a git repository?
/// Writes nothing anywhere.
#[tauri::command]
#[specta::specta]
pub async fn check_workspace(
    args: CheckWorkspaceArgs,
    _state: State<'_, AppState>,
) -> Result<WorkspaceCheckView, String> {
    let path =
        crate::routing::RoleMatrixRouting::validate_workspace(std::path::Path::new(&args.path))?;
    let is_git_repo = nacc_git::GitRepository::open(&path).await.is_ok();
    Ok(WorkspaceCheckView {
        path: path.to_string_lossy().into_owned(),
        is_git_repo,
    })
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct WorkspaceCheckView {
    pub path: String,
    pub is_git_repo: bool,
}
