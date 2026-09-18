//! Read-only Phase 9 Quality/Review evidence IPC. Evidence creation remains
//! behind privileged backend execution/review paths; the webview can only
//! retrieve bounded persisted facts for inspection.

use serde::{Deserialize, Serialize};
use tauri::State;

use nacc_domain::{AttemptId, NodeRunId, WorkflowRunId};
use nacc_review::Severity;

use crate::AppState;

const DEFAULT_EVIDENCE_LIMIT: u32 = 100;
const MAX_EVIDENCE_LIMIT: u32 = 500;

#[derive(Clone, Debug, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct ListEvidenceArgs {
    pub workflow_run_id: Option<WorkflowRunId>,
    pub node_run_id: Option<NodeRunId>,
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct QualityGateResultView {
    pub workflow_run_id: WorkflowRunId,
    pub node_run_id: NodeRunId,
    pub attempt_id: Option<AttemptId>,
    pub gate: String,
    pub command: String,
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub duration_ms: String,
    pub log_tail: String,
    pub created_at_millis: String,
}

impl From<nacc_storage::QualityGateRecord> for QualityGateResultView {
    fn from(record: nacc_storage::QualityGateRecord) -> Self {
        Self {
            workflow_run_id: record.workflow_run_id,
            node_run_id: record.node_run_id,
            attempt_id: record.attempt_id,
            gate: record.evidence.gate,
            command: record.evidence.command,
            passed: record.evidence.passed,
            exit_code: record.evidence.exit_code,
            timed_out: record.evidence.timed_out,
            duration_ms: record.evidence.duration_ms.to_string(),
            log_tail: record.evidence.log_tail,
            created_at_millis: record.created_at_millis.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ReviewSeverityView {
    Blocker,
    Major,
    Minor,
    Note,
}

impl From<Severity> for ReviewSeverityView {
    fn from(severity: Severity) -> Self {
        match severity {
            Severity::Blocker => Self::Blocker,
            Severity::Major => Self::Major,
            Severity::Minor => Self::Minor,
            Severity::Note => Self::Note,
        }
    }
}

#[derive(Clone, Debug, Serialize, specta::Type)]
pub struct ReviewFindingView {
    pub workflow_run_id: WorkflowRunId,
    pub node_run_id: NodeRunId,
    pub attempt_id: Option<AttemptId>,
    pub node_key: String,
    pub file: String,
    pub line: Option<u32>,
    pub severity: ReviewSeverityView,
    pub summary: String,
    pub evidence: String,
    pub created_at_millis: String,
}

impl From<nacc_storage::ReviewFindingRecord> for ReviewFindingView {
    fn from(record: nacc_storage::ReviewFindingRecord) -> Self {
        Self {
            workflow_run_id: record.workflow_run_id,
            node_run_id: record.node_run_id,
            attempt_id: record.attempt_id,
            node_key: record.finding.node_key,
            file: record.finding.file,
            line: record.finding.line,
            severity: record.finding.severity.into(),
            summary: record.finding.summary,
            evidence: record.finding.evidence,
            created_at_millis: record.created_at_millis.to_string(),
        }
    }
}

fn bounded_limit(limit: Option<u32>) -> u32 {
    limit
        .unwrap_or(DEFAULT_EVIDENCE_LIMIT)
        .clamp(1, MAX_EVIDENCE_LIMIT)
}

#[tauri::command]
#[specta::specta]
pub async fn list_quality_gate_results(
    args: ListEvidenceArgs,
    state: State<'_, AppState>,
) -> Result<Vec<QualityGateResultView>, String> {
    state
        .storage
        .list_recent_quality_gate_results(
            args.workflow_run_id,
            args.node_run_id,
            bounded_limit(args.limit),
        )
        .await
        .map(|records| records.into_iter().map(Into::into).collect())
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn list_review_findings(
    args: ListEvidenceArgs,
    state: State<'_, AppState>,
) -> Result<Vec<ReviewFindingView>, String> {
    state
        .storage
        .list_recent_review_findings(
            args.workflow_run_id,
            args.node_run_id,
            bounded_limit(args.limit),
        )
        .await
        .map(|records| records.into_iter().map(Into::into).collect())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_limits_are_defaulted_and_bounded() {
        assert_eq!(bounded_limit(None), DEFAULT_EVIDENCE_LIMIT);
        assert_eq!(bounded_limit(Some(0)), 1);
        assert_eq!(bounded_limit(Some(999)), MAX_EVIDENCE_LIMIT);
    }
}
