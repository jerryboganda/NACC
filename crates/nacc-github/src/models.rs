use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct GithubAuthStatus {
    pub authenticated: bool,
    pub login: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct RepositorySummary {
    pub name_with_owner: String,
    pub default_branch: Option<String>,
    pub is_private: bool,
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct BranchSummary {
    pub name: String,
    pub commit_sha: String,
    pub protected: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct PullRequestSummary {
    pub number: u32,
    pub title: String,
    pub state: String,
    pub is_draft: bool,
    pub head_ref_name: String,
    pub base_ref_name: String,
    pub url: String,
    pub author_login: Option<String>,
    pub merge_state_status: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct CheckRunSummary {
    pub id: String,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub details_url: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub app_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct WorkflowRunSummary {
    pub database_id: String,
    pub name: String,
    pub workflow_name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub event: String,
    pub head_branch: Option<String>,
    pub head_sha: String,
    pub url: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct WorkflowStepSummary {
    pub number: u32,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct WorkflowJobSummary {
    pub database_id: String,
    pub name: String,
    pub status: String,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub url: String,
    pub steps: Vec<WorkflowStepSummary>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct ArtifactSummary {
    pub id: String,
    pub name: String,
    pub size_in_bytes: String,
    pub expired: bool,
    pub created_at: String,
    pub expires_at: String,
    pub workflow_run_id: Option<String>,
    pub head_sha: Option<String>,
    pub archive_download_url: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct EnvironmentSummary {
    pub id: String,
    pub name: String,
    pub url: String,
    pub html_url: String,
    pub can_admins_bypass: Option<bool>,
    pub protection_rule_count: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct PendingDeploymentSummary {
    pub environment_id: String,
    pub environment_name: String,
    pub wait_timer: u32,
    pub current_user_can_approve: bool,
    pub reviewer_logins: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct FailedRunEvidence {
    pub run_id: String,
    pub job_id: String,
    pub job_name: String,
    pub classification: crate::FailureClass,
    pub matched_evidence: Option<String>,
    pub log_excerpt: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, specta::Type)]
pub struct ExplicitApproval {
    pub approved_by: String,
    pub reason: String,
}

impl ExplicitApproval {
    pub fn new(approved_by: impl Into<String>, reason: impl Into<String>) -> crate::Result<Self> {
        let approved_by = approved_by.into().trim().to_string();
        let reason = reason.into().trim().to_string();
        if approved_by.is_empty() || reason.is_empty() {
            return Err(crate::GithubError::ApprovalRequired);
        }
        Ok(Self {
            approved_by,
            reason,
        })
    }

    pub(crate) fn validate(&self) -> crate::Result<()> {
        if self.approved_by.trim().is_empty() || self.reason.trim().is_empty() {
            return Err(crate::GithubError::ApprovalRequired);
        }
        Ok(())
    }
}
