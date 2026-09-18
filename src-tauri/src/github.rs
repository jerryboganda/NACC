//! GitHub / GitHub Actions IPC (master plan S17.12/S19).
//! The webview receives typed summaries only. Authentication remains in the
//! user's `gh` credential store and mutations require explicit human approval.

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct GithubRepositoryArgs {
    pub repository: String,
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct GithubListArgs {
    pub repository: String,
    pub limit: u32,
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct GithubChecksArgs {
    pub repository: String,
    pub commit_sha: String,
    pub limit: u32,
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct GithubRunArgs {
    pub repository: String,
    pub run_id: String,
}

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct GithubRerunFailedArgs {
    pub repository: String,
    pub run_id: String,
    pub approved_by: String,
    pub reason: String,
}

fn client() -> nacc_github::GhClient {
    nacc_github::GhClient::default()
}

#[tauri::command]
#[specta::specta]
pub async fn get_github_auth_status() -> Result<nacc_github::GithubAuthStatus, String> {
    client()
        .auth_status()
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_github_repository(
    args: GithubRepositoryArgs,
) -> Result<nacc_github::RepositorySummary, String> {
    client()
        .repository(&args.repository)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_github_branches(
    args: GithubListArgs,
) -> Result<Vec<nacc_github::BranchSummary>, String> {
    client()
        .branches(&args.repository, args.limit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_github_pull_requests(
    args: GithubListArgs,
) -> Result<Vec<nacc_github::PullRequestSummary>, String> {
    client()
        .pull_requests(&args.repository, args.limit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_github_check_runs(
    args: GithubChecksArgs,
) -> Result<Vec<nacc_github::CheckRunSummary>, String> {
    client()
        .check_runs(&args.repository, &args.commit_sha, args.limit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_github_workflow_runs(
    args: GithubListArgs,
) -> Result<Vec<nacc_github::WorkflowRunSummary>, String> {
    client()
        .workflow_runs(&args.repository, args.limit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_github_workflow_jobs(
    args: GithubRunArgs,
) -> Result<Vec<nacc_github::WorkflowJobSummary>, String> {
    client()
        .workflow_jobs(&args.repository, &args.run_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_github_failed_run_evidence(
    args: GithubRunArgs,
) -> Result<Vec<nacc_github::FailedRunEvidence>, String> {
    client()
        .failed_run_evidence(&args.repository, &args.run_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_github_artifacts(
    args: GithubListArgs,
) -> Result<Vec<nacc_github::ArtifactSummary>, String> {
    client()
        .artifacts(&args.repository, args.limit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_github_environments(
    args: GithubListArgs,
) -> Result<Vec<nacc_github::EnvironmentSummary>, String> {
    client()
        .environments(&args.repository, args.limit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_github_pending_deployments(
    args: GithubRunArgs,
) -> Result<Vec<nacc_github::PendingDeploymentSummary>, String> {
    client()
        .pending_deployments(&args.repository, &args.run_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn rerun_failed_github_workflow(args: GithubRerunFailedArgs) -> Result<(), String> {
    let approval = nacc_github::ExplicitApproval::new(args.approved_by, args.reason)
        .map_err(|error| error.to_string())?;
    client()
        .rerun_failed(&args.repository, &args.run_id, &approval)
        .await
        .map_err(|error| error.to_string())
}
