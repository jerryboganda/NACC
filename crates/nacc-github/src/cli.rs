//! Safe typed wrappers around the installed `gh` CLI (master plan S4.6/S19).
//! Every invocation uses an executable plus argv; no shell is involved, and
//! authentication remains owned by GitHub CLI's native credential store.

use std::{path::PathBuf, process::Stdio};

use serde::de::DeserializeOwned;

use crate::{
    classify, ArtifactSummary, BranchSummary, CheckRunSummary, EnvironmentSummary,
    ExplicitApproval, FailedRunEvidence, GithubAuthStatus, GithubError, PendingDeploymentSummary,
    PullRequestSummary, RepositorySummary, Result, WorkflowJobSummary, WorkflowRunSummary,
};

const MAX_LIST_LIMIT: u32 = 100;
const MAX_EVIDENCE_BYTES: usize = 16 * 1024;
const COMMAND_TIMEOUT_SECS: u64 = 30;

#[derive(Clone, Debug)]
pub struct GhClient {
    executable: PathBuf,
}

impl Default for GhClient {
    fn default() -> Self {
        Self::new("gh")
    }
}

impl GhClient {
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    async fn run(&self, args: Vec<String>) -> Result<String> {
        let mut cmd = tokio::process::Command::new(&self.executable);
        cmd.args(&args).stdin(Stdio::null()).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(COMMAND_TIMEOUT_SECS),
            cmd.output(),
        )
        .await
        .map_err(|_| GithubError::Timeout {
            args: args.clone(),
            seconds: COMMAND_TIMEOUT_SECS,
        })?
        .map_err(|source| GithubError::Spawn {
            executable: self.executable.clone(),
            source,
        })?;
        if !output.status.success() {
            return Err(GithubError::CommandFailed {
                args,
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    async fn run_json<T: DeserializeOwned>(
        &self,
        args: Vec<String>,
        context: &'static str,
    ) -> Result<T> {
        let output = self.run(args).await?;
        serde_json::from_str(&output).map_err(|source| GithubError::Parse { context, source })
    }

    pub async fn version(&self) -> Result<String> {
        Ok(self
            .run(vec!["--version".to_string()])
            .await?
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string())
    }

    pub async fn auth_status(&self) -> Result<GithubAuthStatus> {
        self.run_json(
            vec![
                "auth".into(),
                "status".into(),
                "--active".into(),
                "--hostname".into(),
                "github.com".into(),
                "--json".into(),
                "hosts".into(),
                "--jq".into(),
                r#"[.hosts["github.com"][] | select(.active == true)][0] // {} | {authenticated:(.state == "success"), login:(.login // null)}"#.into(),
            ],
            "GitHub authentication status",
        )
        .await
    }

    pub async fn repository(&self, repository: &str) -> Result<RepositorySummary> {
        let repository = validated_repository(repository)?;
        self.run_json(
            vec![
                "repo".into(),
                "view".into(),
                repository,
                "--json".into(),
                "nameWithOwner,defaultBranchRef,isPrivate,url".into(),
                "--jq".into(),
                r#"{name_with_owner:.nameWithOwner,default_branch:(.defaultBranchRef.name // null),is_private:.isPrivate,url:.url}"#.into(),
            ],
            "repository summary",
        )
        .await
    }

    pub async fn branches(&self, repository: &str, limit: u32) -> Result<Vec<BranchSummary>> {
        let repository = validated_repository(repository)?;
        self.run_json(
            vec![
                "api".into(),
                format!("repos/{repository}/branches"),
                "--method".into(),
                "GET".into(),
                "-f".into(),
                format!("per_page={}", bounded_limit(limit)),
                "--jq".into(),
                r#"[.[] | {name,commit_sha:.commit.sha,protected}]"#.into(),
            ],
            "branch list",
        )
        .await
    }

    pub async fn pull_requests(
        &self,
        repository: &str,
        limit: u32,
    ) -> Result<Vec<PullRequestSummary>> {
        let repository = validated_repository(repository)?;
        self.run_json(
            vec![
                "pr".into(),
                "list".into(),
                "--repo".into(),
                repository,
                "--limit".into(),
                bounded_limit(limit).to_string(),
                "--json".into(),
                "number,title,state,isDraft,headRefName,baseRefName,url,author,mergeStateStatus".into(),
                "--jq".into(),
                r#"[.[] | {number,title,state,is_draft:.isDraft,head_ref_name:.headRefName,base_ref_name:.baseRefName,url,author_login:(.author.login // null),merge_state_status:(.mergeStateStatus // null)}]"#.into(),
            ],
            "pull request list",
        )
        .await
    }

    pub async fn check_runs(
        &self,
        repository: &str,
        commit_sha: &str,
        limit: u32,
    ) -> Result<Vec<CheckRunSummary>> {
        let repository = validated_repository(repository)?;
        let commit_sha = validated_commit_sha(commit_sha)?;
        self.run_json(
            vec![
                "api".into(),
                format!("repos/{repository}/commits/{commit_sha}/check-runs"),
                "--method".into(),
                "GET".into(),
                "-f".into(),
                format!("per_page={}", bounded_limit(limit)),
                "-H".into(),
                "Accept: application/vnd.github+json".into(),
                "--jq".into(),
                r#"[.check_runs[] | {id:(.id|tostring),name,status,conclusion,details_url,started_at,completed_at,app_name:(.app.name // null)}]"#.into(),
            ],
            "check run list",
        )
        .await
    }

    pub async fn workflow_runs(
        &self,
        repository: &str,
        limit: u32,
    ) -> Result<Vec<WorkflowRunSummary>> {
        let repository = validated_repository(repository)?;
        self.run_json(
            vec![
                "run".into(),
                "list".into(),
                "--repo".into(),
                repository,
                "--limit".into(),
                bounded_limit(limit).to_string(),
                "--json".into(),
                "databaseId,name,workflowName,status,conclusion,event,headBranch,headSha,url,createdAt,updatedAt".into(),
                "--jq".into(),
                r#"[.[] | {database_id:(.databaseId|tostring),name,workflow_name:.workflowName,status,conclusion,event,head_branch:(.headBranch // null),head_sha:.headSha,url,created_at:.createdAt,updated_at:.updatedAt}]"#.into(),
            ],
            "workflow run list",
        )
        .await
    }

    pub async fn workflow_jobs(
        &self,
        repository: &str,
        run_id: &str,
    ) -> Result<Vec<WorkflowJobSummary>> {
        let repository = validated_repository(repository)?;
        let run_id = validated_run_id(run_id)?;
        self.run_json(
            vec![
                "run".into(),
                "view".into(),
                run_id.clone(),
                "--repo".into(),
                repository,
                "--json".into(),
                "jobs".into(),
                "--jq".into(),
                r#"[.jobs[] | {database_id:(.databaseId|tostring),name,status,conclusion,started_at:(.startedAt // null),completed_at:(.completedAt // null),url,steps:[.steps[] | {number,name,status,conclusion,started_at:(.startedAt // null),completed_at:(.completedAt // null)}]}]"#.into(),
            ],
            "workflow job list",
        )
        .await
    }

    pub async fn failed_run_evidence(
        &self,
        repository: &str,
        run_id: &str,
    ) -> Result<Vec<FailedRunEvidence>> {
        let repository = validated_repository(repository)?;
        let run_id = validated_run_id(run_id)?;
        let jobs = self.workflow_jobs(&repository, &run_id).await?;
        let mut evidence = Vec::new();
        for job in jobs
            .into_iter()
            .filter(|job| job.conclusion.as_deref() == Some("failure"))
        {
            let log = self
                .run(vec![
                    "run".into(),
                    "view".into(),
                    run_id.clone(),
                    "--repo".into(),
                    repository.clone(),
                    "--job".into(),
                    job.database_id.clone(),
                    "--log-failed".into(),
                ])
                .await?;
            let excerpt = bounded_excerpt(&log);
            let (classification, matched_evidence) = classify(&job.name, &excerpt);
            evidence.push(FailedRunEvidence {
                run_id: run_id.clone(),
                job_id: job.database_id,
                job_name: job.name,
                classification,
                matched_evidence,
                log_excerpt: excerpt,
            });
        }
        Ok(evidence)
    }

    pub async fn artifacts(&self, repository: &str, limit: u32) -> Result<Vec<ArtifactSummary>> {
        let repository = validated_repository(repository)?;
        self.run_json(
            vec![
                "api".into(),
                format!("repos/{repository}/actions/artifacts"),
                "--method".into(),
                "GET".into(),
                "-f".into(),
                format!("per_page={}", bounded_limit(limit)),
                "--jq".into(),
                r#"[.artifacts[] | {id:(.id|tostring),name,size_in_bytes:(.size_in_bytes|tostring),expired,created_at,expires_at,workflow_run_id:(if .workflow_run.id == null then null else (.workflow_run.id|tostring) end),head_sha:(.workflow_run.head_sha // null),archive_download_url}]"#.into(),
            ],
            "artifact list",
        )
        .await
    }

    pub async fn environments(
        &self,
        repository: &str,
        limit: u32,
    ) -> Result<Vec<EnvironmentSummary>> {
        let repository = validated_repository(repository)?;
        self.run_json(
            vec![
                "api".into(),
                format!("repos/{repository}/environments"),
                "--method".into(),
                "GET".into(),
                "-f".into(),
                format!("per_page={}", bounded_limit(limit)),
                "--jq".into(),
                r#"[.environments[] | {id:(.id|tostring),name,url,html_url,can_admins_bypass:(.can_admins_bypass // null),protection_rule_count:(.protection_rules | length)}]"#.into(),
            ],
            "environment list",
        )
        .await
    }

    pub async fn pending_deployments(
        &self,
        repository: &str,
        run_id: &str,
    ) -> Result<Vec<PendingDeploymentSummary>> {
        let repository = validated_repository(repository)?;
        let run_id = validated_run_id(run_id)?;
        self.run_json(
            vec![
                "api".into(),
                format!("repos/{repository}/actions/runs/{run_id}/pending_deployments"),
                "--method".into(),
                "GET".into(),
                "--jq".into(),
                r#"[.[] | {environment_id:(.environment.id|tostring),environment_name:.environment.name,wait_timer,current_user_can_approve,reviewer_logins:[.reviewers[]? | (.reviewer.login // .reviewer.name // "unknown")]}]"#.into(),
            ],
            "pending deployment list",
        )
        .await
    }

    pub async fn rerun_failed(
        &self,
        repository: &str,
        run_id: &str,
        approval: &ExplicitApproval,
    ) -> Result<()> {
        approval.validate()?;
        let repository = validated_repository(repository)?;
        let run_id = validated_run_id(run_id)?;
        self.run(vec![
            "run".into(),
            "rerun".into(),
            run_id,
            "--repo".into(),
            repository,
            "--failed".into(),
        ])
        .await?;
        Ok(())
    }
}

pub async fn gh_version() -> Result<String> {
    GhClient::default().version().await
}

pub async fn gh_installed() -> bool {
    matches!(gh_version().await, Ok(version) if !version.is_empty())
}

fn validated_repository(repository: &str) -> Result<String> {
    let repository = repository.trim();
    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    let valid_part = |part: &str| {
        !part.is_empty()
            && part != "."
            && part != ".."
            && part
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    };
    if parts.next().is_some() || !valid_part(owner) || !valid_part(name) {
        return Err(GithubError::InvalidRequest(
            "repository must be an owner/name slug".to_string(),
        ));
    }
    Ok(repository.to_string())
}

fn validated_commit_sha(commit_sha: &str) -> Result<String> {
    let commit_sha = commit_sha.trim();
    if !(7..=64).contains(&commit_sha.len()) || !commit_sha.chars().all(|ch| ch.is_ascii_hexdigit())
    {
        return Err(GithubError::InvalidRequest(
            "commit SHA must be 7-64 hexadecimal characters".to_string(),
        ));
    }
    Ok(commit_sha.to_ascii_lowercase())
}

fn validated_run_id(run_id: &str) -> Result<String> {
    let run_id = run_id.trim();
    if run_id.is_empty()
        || run_id.len() > 20
        || !run_id.chars().all(|ch| ch.is_ascii_digit())
        || run_id.chars().all(|ch| ch == '0')
    {
        return Err(GithubError::InvalidRequest(
            "workflow run id must be a positive decimal identifier".to_string(),
        ));
    }
    Ok(run_id.to_string())
}

fn bounded_limit(limit: u32) -> u32 {
    limit.clamp(1, MAX_LIST_LIMIT)
}

fn bounded_excerpt(log: &str) -> String {
    let bytes = log.as_bytes();
    if bytes.len() <= MAX_EVIDENCE_BYTES {
        return log.to_string();
    }
    let mut start = bytes.len() - MAX_EVIDENCE_BYTES;
    while start < bytes.len() && !log.is_char_boundary(start) {
        start += 1;
    }
    format!(
        "[truncated to last {MAX_EVIDENCE_BYTES} bytes]\n{}",
        &log[start..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_validation_blocks_shell_like_or_ambiguous_input() {
        assert_eq!(validated_repository("owner/repo").unwrap(), "owner/repo");
        assert!(validated_repository("owner/repo;rm").is_err());
        assert!(validated_repository("owner/repo/extra").is_err());
        assert!(validated_repository("owner/").is_err());
    }

    #[test]
    fn sha_and_limits_are_bounded() {
        assert_eq!(validated_commit_sha("ABCDEF1").unwrap(), "abcdef1");
        assert!(validated_commit_sha("not-a-sha").is_err());
        assert_eq!(bounded_limit(0), 1);
        assert_eq!(bounded_limit(10_000), MAX_LIST_LIMIT);
    }

    #[test]
    fn workflow_run_ids_are_decimal_strings() {
        assert_eq!(
            validated_run_id("12345678901234567890").unwrap(),
            "12345678901234567890"
        );
        assert!(validated_run_id("0").is_err());
        assert!(validated_run_id("12.3").is_err());
        assert!(validated_run_id("123456789012345678901").is_err());
    }

    #[test]
    fn explicit_approval_requires_actor_and_reason() {
        assert!(ExplicitApproval::new("human", "diagnosed CI failure").is_ok());
        assert!(ExplicitApproval::new("", "reason").is_err());
        assert!(ExplicitApproval::new("human", " ").is_err());
    }

    #[test]
    fn evidence_excerpt_is_bounded_from_the_tail() {
        let log = "x".repeat(MAX_EVIDENCE_BYTES + 100);
        let excerpt = bounded_excerpt(&log);
        assert!(excerpt.len() < log.len());
        assert!(excerpt.ends_with(&"x".repeat(100)));
    }
}
