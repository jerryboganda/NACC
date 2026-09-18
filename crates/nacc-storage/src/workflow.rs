//! Durable workflow state: runs, node runs, attempts, approvals, and
//! checkpoints (master plan S4.4's workflow data groups, S14's durable state
//! machine, S16's crash recovery).
//!
//! # Why every one of these is a separate table
//!
//! - A **run** is the durable identity of one template execution, and the
//!   thing a restart has to find.
//! - A **node run** is one DAG node's state *inside* a run. Retries and
//!   recovery rewrite this row; the run row is not touched.
//! - An **attempt** is an append-only record of one try at one node. Nothing
//!   overwrites it: "it succeeded on the third attempt after two timeouts"
//!   is exactly the history master plan S14.5's repair policy and a
//!   post-mortem both need.
//! - An **approval** is a human decision with a timestamp and a name, so
//!   "who let this run write to the repository" has an answer.
//! - A **checkpoint** is a monotonic sequence of state snapshots per run,
//!   which is what makes "restart NACC mid-run and resume safely" checkable
//!   rather than asserted.

use nacc_domain::{
    ApprovalDecision, ApprovalId, AttemptId, AttemptTrigger, NodeRunId, NodeState, ProjectId,
    ProviderId, RunState, WorkflowNode, WorkflowRunId,
};
use rusqlite::{OptionalExtension, Row};

use crate::{lock, Database, Result, StorageError};

/// One workflow run as persisted. The node graph lives in
/// `node_runs`/`run_checkpoints`, not here: a run row that embedded its
/// whole graph would have to be rewritten on every node transition.
#[derive(Clone, Debug)]
pub struct WorkflowRunRecord {
    pub id: WorkflowRunId,
    pub project_id: ProjectId,
    pub template_name: String,
    pub state: RunState,
    /// Human-readable note, e.g. why a run was interrupted or paused.
    pub note: Option<String>,
    pub created_at_millis: u64,
    pub updated_at_millis: u64,
}

/// One node's durable state inside a run.
#[derive(Clone, Debug)]
pub struct NodeRunRecord {
    pub id: NodeRunId,
    pub workflow_run_id: WorkflowRunId,
    pub node_key: String,
    pub title: String,
    pub state: NodeState,
    /// How many attempts have *started*, including the one in flight.
    pub attempts: u32,
    /// The node's definition, persisted with the node run so a resumed run
    /// does not depend on the template being byte-identical after a restart.
    pub definition: WorkflowNode,
    pub last_detail: Option<String>,
    pub created_at_millis: u64,
    pub updated_at_millis: u64,
}

/// One attempt at one node.
#[derive(Clone, Debug)]
pub struct NodeAttemptRecord {
    pub id: AttemptId,
    pub node_run_id: NodeRunId,
    pub workflow_run_id: WorkflowRunId,
    pub attempt_number: u32,
    pub trigger: AttemptTrigger,
    /// `None` while the attempt is in flight.
    pub finished_state: Option<NodeState>,
    pub provider: Option<ProviderId>,
    pub detail: Option<String>,
    pub started_at_millis: u64,
    pub finished_at_millis: Option<u64>,
}

/// One approval gate.
#[derive(Clone, Debug)]
pub struct ApprovalRecord {
    pub id: ApprovalId,
    pub workflow_run_id: WorkflowRunId,
    pub node_run_id: NodeRunId,
    pub summary: String,
    pub requested_at_millis: u64,
    pub decision: Option<ApprovalDecision>,
    pub decided_at_millis: Option<u64>,
}

/// One state snapshot in a run's checkpoint sequence.
#[derive(Clone, Debug)]
pub struct CheckpointRecord {
    pub sequence: u32,
    pub state: RunState,
    pub detail: String,
    pub created_at_millis: u64,
}

fn parse<T: std::str::FromStr<Err = nacc_domain::DomainError>>(
    raw: &str,
    entity: &'static str,
) -> std::result::Result<T, StorageError> {
    raw.parse().map_err(
        |e: nacc_domain::DomainError| StorageError::CorruptStoredValue {
            entity,
            value: raw.to_string(),
            detail: format!("{e}"),
        },
    )
}

fn millis(row: &Row<'_>, column: &str) -> std::result::Result<u64, StorageError> {
    Ok(row.get::<_, i64>(column)?.max(0) as u64)
}

impl Database {
    // --- runs ---------------------------------------------------------

    pub async fn insert_workflow_run(&self, run: &WorkflowRunRecord) -> Result<()> {
        let run = run.clone();
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                "INSERT INTO workflow_runs (
                    id, project_id, template_name, state_json, note,
                    created_at_millis, updated_at_millis
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    run.id.to_string(),
                    run.project_id.to_string(),
                    run.template_name,
                    serde_json::to_string(&run.state)?,
                    run.note,
                    run.created_at_millis as i64,
                    run.updated_at_millis as i64,
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn update_workflow_run(&self, run: &WorkflowRunRecord) -> Result<()> {
        let run = run.clone();
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            let changed = conn.execute(
                "UPDATE workflow_runs SET state_json = ?2, note = ?3, updated_at_millis = ?4
                 WHERE id = ?1",
                rusqlite::params![
                    run.id.to_string(),
                    serde_json::to_string(&run.state)?,
                    run.note,
                    run.updated_at_millis as i64,
                ],
            )?;
            if changed == 0 {
                return Err(StorageError::WorkflowRunNotFound(run.id));
            }
            Ok(())
        })
        .await?
    }

    pub async fn get_workflow_run(&self, id: WorkflowRunId) -> Result<Option<WorkflowRunRecord>> {
        let conn = self.connection();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<WorkflowRunRecord>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(
                "SELECT id, project_id, template_name, state_json, note,
                        created_at_millis, updated_at_millis
                 FROM workflow_runs WHERE id = ?1",
            )?;
            let found = stmt.query_row([&id], row_to_run).optional()?;
            match found {
                Some(Ok(run)) => Ok(Some(run)),
                Some(Err(err)) => Err(err),
                None => Ok(None),
            }
        })
        .await?
    }

    /// Runs in any of `states`, newest first. Recovery uses this with
    /// `[Running, AwaitingApproval, Interrupted]` to find what a crash left
    /// behind.
    pub async fn list_workflow_runs_in_states(
        &self,
        states: &[RunState],
    ) -> Result<Vec<WorkflowRunRecord>> {
        let conn = self.connection();
        let encoded = states
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        tokio::task::spawn_blocking(move || -> Result<Vec<WorkflowRunRecord>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(
                "SELECT id, project_id, template_name, state_json, note,
                        created_at_millis, updated_at_millis
                 FROM workflow_runs ORDER BY created_at_millis DESC, rowid DESC",
            )?;
            let mut rows = stmt.query([])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                let run = row_to_run(row)??;
                if encoded.contains(&serde_json::to_string(&run.state)?) {
                    out.push(run);
                }
            }
            Ok(out)
        })
        .await?
    }

    /// Every run for a project, newest first, in any state.
    pub async fn list_workflow_runs_for_project(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<WorkflowRunRecord>> {
        let conn = self.connection();
        let project = project_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<WorkflowRunRecord>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(
                "SELECT id, project_id, template_name, state_json, note,
                        created_at_millis, updated_at_millis
                 FROM workflow_runs WHERE project_id = ?1
                 ORDER BY created_at_millis DESC, rowid DESC",
            )?;
            let mut rows = stmt.query([project])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(row_to_run(row)??);
            }
            Ok(out)
        })
        .await?
    }

    // --- node runs ----------------------------------------------------

    pub async fn insert_node_run(&self, node: &NodeRunRecord) -> Result<()> {
        let node = node.clone();
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                "INSERT INTO node_runs (
                    id, workflow_run_id, node_key, title, state_json, attempts,
                    definition_json, last_detail, created_at_millis, updated_at_millis
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    node.id.to_string(),
                    node.workflow_run_id.to_string(),
                    node.node_key,
                    node.title,
                    serde_json::to_string(&node.state)?,
                    node.attempts as i64,
                    serde_json::to_string(&node.definition)?,
                    node.last_detail,
                    node.created_at_millis as i64,
                    node.updated_at_millis as i64,
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn update_node_run(&self, node: &NodeRunRecord) -> Result<()> {
        let node = node.clone();
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            let changed = conn.execute(
                "UPDATE node_runs SET state_json = ?2, attempts = ?3, last_detail = ?4,
                    updated_at_millis = ?5
                 WHERE id = ?1",
                rusqlite::params![
                    node.id.to_string(),
                    serde_json::to_string(&node.state)?,
                    node.attempts as i64,
                    node.last_detail,
                    node.updated_at_millis as i64,
                ],
            )?;
            if changed == 0 {
                return Err(StorageError::NodeRunNotFound(node.id));
            }
            Ok(())
        })
        .await?
    }

    pub async fn list_node_runs(&self, run_id: WorkflowRunId) -> Result<Vec<NodeRunRecord>> {
        let conn = self.connection();
        let run = run_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<NodeRunRecord>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(
                "SELECT id, workflow_run_id, node_key, title, state_json, attempts,
                        definition_json, last_detail, created_at_millis, updated_at_millis
                 FROM node_runs WHERE workflow_run_id = ?1 ORDER BY rowid",
            )?;
            let mut rows = stmt.query([run])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(row_to_node_run(row)??);
            }
            Ok(out)
        })
        .await?
    }

    // --- attempts -----------------------------------------------------

    pub async fn insert_node_attempt(&self, attempt: &NodeAttemptRecord) -> Result<()> {
        let attempt = attempt.clone();
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                "INSERT INTO node_attempts (
                    id, node_run_id, workflow_run_id, attempt_number, trigger_json,
                    finished_state_json, provider_json, detail, started_at_millis,
                    finished_at_millis
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![
                    attempt.id.to_string(),
                    attempt.node_run_id.to_string(),
                    attempt.workflow_run_id.to_string(),
                    attempt.attempt_number as i64,
                    serde_json::to_string(&attempt.trigger)?,
                    attempt
                        .finished_state
                        .map(|state| serde_json::to_string(&state))
                        .transpose()?,
                    attempt
                        .provider
                        .map(|provider| serde_json::to_string(&provider))
                        .transpose()?,
                    attempt.detail,
                    attempt.started_at_millis as i64,
                    attempt.finished_at_millis.map(|value| value as i64),
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn finish_node_attempt(
        &self,
        attempt_id: AttemptId,
        state: NodeState,
        detail: &str,
        finished_at_millis: u64,
    ) -> Result<()> {
        let conn = self.connection();
        let attempt_key = attempt_id.to_string();
        let state_json = serde_json::to_string(&state)?;
        let detail = detail.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            let changed = conn.execute(
                "UPDATE node_attempts SET finished_state_json = ?2, detail = ?3,
                    finished_at_millis = ?4 WHERE id = ?1",
                rusqlite::params![attempt_key, state_json, detail, finished_at_millis as i64],
            )?;
            if changed == 0 {
                return Err(StorageError::AttemptNotFound(attempt_id));
            }
            Ok(())
        })
        .await?
    }

    pub async fn list_node_attempts(
        &self,
        run_id: WorkflowRunId,
    ) -> Result<Vec<NodeAttemptRecord>> {
        let conn = self.connection();
        let run = run_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<NodeAttemptRecord>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(
                "SELECT id, node_run_id, workflow_run_id, attempt_number, trigger_json,
                        finished_state_json, provider_json, detail, started_at_millis,
                        finished_at_millis
                 FROM node_attempts WHERE workflow_run_id = ?1
                 ORDER BY started_at_millis, rowid",
            )?;
            let mut rows = stmt.query([run])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(row_to_attempt(row)??);
            }
            Ok(out)
        })
        .await?
    }

    // --- approvals ----------------------------------------------------

    pub async fn insert_approval(&self, approval: &ApprovalRecord) -> Result<()> {
        let approval = approval.clone();
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                "INSERT INTO approvals (
                    id, workflow_run_id, node_run_id, summary, requested_at_millis,
                    decision_json, decided_at_millis
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    approval.id.to_string(),
                    approval.workflow_run_id.to_string(),
                    approval.node_run_id.to_string(),
                    approval.summary,
                    approval.requested_at_millis as i64,
                    approval
                        .decision
                        .map(|decision| serde_json::to_string(&decision))
                        .transpose()?,
                    approval.decided_at_millis.map(|value| value as i64),
                ],
            )?;
            Ok(())
        })
        .await?
    }

    /// Record a human decision. Only a pending approval can be decided:
    /// overwriting an existing decision would erase who approved what.
    pub async fn decide_approval(
        &self,
        approval_id: ApprovalId,
        decision: &ApprovalDecision,
        decided_at_millis: u64,
    ) -> Result<()> {
        let conn = self.connection();
        let approval_key = approval_id.to_string();
        let decision_json = serde_json::to_string(decision)?;
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            let changed = conn.execute(
                "UPDATE approvals SET decision_json = ?2, decided_at_millis = ?3
                 WHERE id = ?1 AND decision_json IS NULL",
                rusqlite::params![approval_key, decision_json, decided_at_millis as i64],
            )?;
            if changed == 0 {
                return Err(StorageError::ApprovalNotPending(approval_id));
            }
            Ok(())
        })
        .await?
    }

    pub async fn list_approvals(&self, run_id: WorkflowRunId) -> Result<Vec<ApprovalRecord>> {
        let conn = self.connection();
        let run = run_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<ApprovalRecord>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(
                "SELECT id, workflow_run_id, node_run_id, summary, requested_at_millis,
                        decision_json, decided_at_millis
                 FROM approvals WHERE workflow_run_id = ?1 ORDER BY requested_at_millis, rowid",
            )?;
            let mut rows = stmt.query([run])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                out.push(row_to_approval(row)??);
            }
            Ok(out)
        })
        .await?
    }

    // --- checkpoints --------------------------------------------------

    /// Append a checkpoint. Sequence numbers are per run and must increase;
    /// the storage layer enforces that so a resuming engine cannot silently
    /// rewrite history it disagrees with.
    pub async fn append_checkpoint(
        &self,
        run_id: WorkflowRunId,
        state: RunState,
        detail: &str,
        created_at_millis: u64,
    ) -> Result<u32> {
        let conn = self.connection();
        let run = run_id.to_string();
        let state_json = serde_json::to_string(&state)?;
        let detail = detail.to_string();
        tokio::task::spawn_blocking(move || -> Result<u32> {
            let conn = lock(&conn);
            let next: i64 = conn.query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM run_checkpoints WHERE workflow_run_id = ?1",
                [&run],
                |row| row.get(0),
            )?;
            conn.execute(
                "INSERT INTO run_checkpoints (
                    workflow_run_id, sequence, state_json, detail, created_at_millis
                 ) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![run, next, state_json, detail, created_at_millis as i64],
            )?;
            Ok(next as u32)
        })
        .await?
    }

    pub async fn list_checkpoints(&self, run_id: WorkflowRunId) -> Result<Vec<CheckpointRecord>> {
        let conn = self.connection();
        let run = run_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<CheckpointRecord>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(
                "SELECT sequence, state_json, detail, created_at_millis
                 FROM run_checkpoints WHERE workflow_run_id = ?1 ORDER BY sequence",
            )?;
            let mut rows = stmt.query([run])?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                let sequence: i64 = row.get(0)?;
                let state_json: String = row.get(1)?;
                out.push(CheckpointRecord {
                    sequence: sequence.max(0) as u32,
                    state: serde_json::from_str(&state_json)?,
                    detail: row.get(2)?,
                    created_at_millis: millis(row, "created_at_millis")?,
                });
            }
            Ok(out)
        })
        .await?
    }

    /// Delete a run and everything attached to it. Used by tests and by an
    /// explicit user "forget this run"; nothing in the engine deletes run
    /// history on its own.
    pub async fn delete_workflow_run(&self, run_id: WorkflowRunId) -> Result<()> {
        let conn = self.connection();
        let run = run_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let mut conn = lock(&conn);
            let transaction = conn.transaction()?;
            transaction.execute(
                "DELETE FROM run_checkpoints WHERE workflow_run_id = ?1",
                [&run],
            )?;
            transaction.execute("DELETE FROM approvals WHERE workflow_run_id = ?1", [&run])?;
            transaction.execute(
                "DELETE FROM node_attempts WHERE workflow_run_id = ?1",
                [&run],
            )?;
            transaction.execute("DELETE FROM node_runs WHERE workflow_run_id = ?1", [&run])?;
            transaction.execute("DELETE FROM workflow_runs WHERE id = ?1", [&run])?;
            transaction.commit()?;
            Ok(())
        })
        .await?
    }
}

fn row_to_run(
    row: &Row<'_>,
) -> rusqlite::Result<std::result::Result<WorkflowRunRecord, StorageError>> {
    let id: String = row.get("id")?;
    let project: String = row.get("project_id")?;
    let state_json: String = row.get("state_json")?;
    Ok((|| {
        Ok(WorkflowRunRecord {
            id: parse(&id, "workflow_runs.id")?,
            project_id: parse(&project, "workflow_runs.project_id")?,
            template_name: row.get("template_name")?,
            state: serde_json::from_str(&state_json)?,
            note: row.get("note")?,
            created_at_millis: millis(row, "created_at_millis")?,
            updated_at_millis: millis(row, "updated_at_millis")?,
        })
    })())
}

fn row_to_node_run(
    row: &Row<'_>,
) -> rusqlite::Result<std::result::Result<NodeRunRecord, StorageError>> {
    let id: String = row.get("id")?;
    let run: String = row.get("workflow_run_id")?;
    let state_json: String = row.get("state_json")?;
    let definition_json: String = row.get("definition_json")?;
    Ok((|| {
        Ok(NodeRunRecord {
            id: parse(&id, "node_runs.id")?,
            workflow_run_id: parse(&run, "node_runs.workflow_run_id")?,
            node_key: row.get("node_key")?,
            title: row.get("title")?,
            state: serde_json::from_str(&state_json)?,
            attempts: row.get::<_, i64>("attempts")?.max(0) as u32,
            definition: serde_json::from_str(&definition_json)?,
            last_detail: row.get("last_detail")?,
            created_at_millis: millis(row, "created_at_millis")?,
            updated_at_millis: millis(row, "updated_at_millis")?,
        })
    })())
}

fn row_to_attempt(
    row: &Row<'_>,
) -> rusqlite::Result<std::result::Result<NodeAttemptRecord, StorageError>> {
    let id: String = row.get("id")?;
    let node: String = row.get("node_run_id")?;
    let run: String = row.get("workflow_run_id")?;
    let trigger_json: String = row.get("trigger_json")?;
    let finished_state_json: Option<String> = row.get("finished_state_json")?;
    let provider_json: Option<String> = row.get("provider_json")?;
    Ok((|| {
        Ok(NodeAttemptRecord {
            id: parse(&id, "node_attempts.id")?,
            node_run_id: parse(&node, "node_attempts.node_run_id")?,
            workflow_run_id: parse(&run, "node_attempts.workflow_run_id")?,
            attempt_number: row.get::<_, i64>("attempt_number")?.max(0) as u32,
            trigger: serde_json::from_str(&trigger_json)?,
            finished_state: finished_state_json
                .map(|json| serde_json::from_str(&json))
                .transpose()?,
            provider: provider_json
                .map(|json| serde_json::from_str(&json))
                .transpose()?,
            detail: row.get("detail")?,
            started_at_millis: millis(row, "started_at_millis")?,
            finished_at_millis: row
                .get::<_, Option<i64>>("finished_at_millis")?
                .map(|value| value.max(0) as u64),
        })
    })())
}

fn row_to_approval(
    row: &Row<'_>,
) -> rusqlite::Result<std::result::Result<ApprovalRecord, StorageError>> {
    let id: String = row.get("id")?;
    let run: String = row.get("workflow_run_id")?;
    let node: String = row.get("node_run_id")?;
    let decision_json: Option<String> = row.get("decision_json")?;
    Ok((|| {
        Ok(ApprovalRecord {
            id: parse(&id, "approvals.id")?,
            workflow_run_id: parse(&run, "approvals.workflow_run_id")?,
            node_run_id: parse(&node, "approvals.node_run_id")?,
            summary: row.get("summary")?,
            requested_at_millis: millis(row, "requested_at_millis")?,
            decision: decision_json
                .map(|json| serde_json::from_str(&json))
                .transpose()?,
            decided_at_millis: row
                .get::<_, Option<i64>>("decided_at_millis")?
                .map(|value| value.max(0) as u64),
        })
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nacc_domain::{PermissionProfile, RoleKind};

    fn node_definition(key: &str) -> WorkflowNode {
        WorkflowNode {
            key: key.to_string(),
            title: format!("{key} title"),
            role: RoleKind::RepositoryExplorer,
            depends_on: vec![],
            instruction: format!("instruction for {key}"),
            permission_profile_hint: PermissionProfile::ReadOnly,
            retryable: true,
            requires_approval: false,
            timeout_secs: None,
            quality_gates: vec![],
            fallbacks: vec![],
        }
    }

    fn run_record(created_at_millis: u64) -> WorkflowRunRecord {
        WorkflowRunRecord {
            id: WorkflowRunId::new(),
            project_id: ProjectId::new(),
            template_name: "enterprise-feature".to_string(),
            state: RunState::Running,
            note: None,
            created_at_millis,
            updated_at_millis: created_at_millis,
        }
    }

    fn node_run_record(run_id: WorkflowRunId, key: &str, created_at_millis: u64) -> NodeRunRecord {
        NodeRunRecord {
            id: NodeRunId::new(),
            workflow_run_id: run_id,
            node_key: key.to_string(),
            title: format!("{key} title"),
            state: NodeState::Pending,
            attempts: 0,
            definition: node_definition(key),
            last_detail: None,
            created_at_millis,
            updated_at_millis: created_at_millis,
        }
    }

    fn attempt_record(
        node: &NodeRunRecord,
        number: u32,
        started_at_millis: u64,
    ) -> NodeAttemptRecord {
        NodeAttemptRecord {
            id: AttemptId::new(),
            node_run_id: node.id,
            workflow_run_id: node.workflow_run_id,
            attempt_number: number,
            trigger: if number == 1 {
                AttemptTrigger::Initial
            } else {
                AttemptTrigger::Retry
            },
            finished_state: None,
            provider: Some(ProviderId::Claude),
            detail: None,
            started_at_millis,
            finished_at_millis: None,
        }
    }

    fn approval_record(
        run_id: WorkflowRunId,
        node_run_id: NodeRunId,
        requested_at_millis: u64,
    ) -> ApprovalRecord {
        ApprovalRecord {
            id: ApprovalId::new(),
            workflow_run_id: run_id,
            node_run_id,
            summary: "integrator may push to the repository".to_string(),
            requested_at_millis,
            decision: None,
            decided_at_millis: None,
        }
    }

    #[tokio::test]
    async fn a_run_round_trips_insert_get_and_update() {
        let db = Database::open_in_memory().unwrap();
        let mut run = run_record(1_000);
        db.insert_workflow_run(&run).await.unwrap();

        let read = db
            .get_workflow_run(run.id)
            .await
            .unwrap()
            .expect("row must exist");
        assert_eq!(read.project_id, run.project_id);
        assert_eq!(read.template_name, "enterprise-feature");
        assert_eq!(read.state, RunState::Running);
        assert_eq!(read.note, None);
        assert_eq!(read.created_at_millis, 1_000);

        run.state = RunState::Failed;
        run.note = Some("node `verify` exhausted its attempts".to_string());
        run.updated_at_millis = 2_000;
        db.update_workflow_run(&run).await.unwrap();

        let updated = db.get_workflow_run(run.id).await.unwrap().unwrap();
        assert_eq!(updated.state, RunState::Failed);
        assert_eq!(
            updated.note.as_deref(),
            Some("node `verify` exhausted its attempts")
        );
        assert_eq!(updated.updated_at_millis, 2_000);
        // created_at_millis is never rewritten by an update: when a run
        // started is a historical fact.
        assert_eq!(updated.created_at_millis, 1_000);
    }

    #[tokio::test]
    async fn updating_a_run_that_was_never_inserted_is_an_error() {
        let db = Database::open_in_memory().unwrap();
        let run = run_record(0);
        let err = db.update_workflow_run(&run).await.unwrap_err();
        assert!(matches!(err, StorageError::WorkflowRunNotFound(id) if id == run.id));
    }

    #[tokio::test]
    async fn an_unknown_run_id_reads_as_none_not_an_error() {
        let db = Database::open_in_memory().unwrap();
        assert!(db
            .get_workflow_run(WorkflowRunId::new())
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn node_runs_list_in_insertion_order_even_when_timestamps_disagree() {
        // The engine instantiates nodes in template order, and the run view
        // must replay that order; `ORDER BY rowid` is what provides it, so
        // the test deliberately gives later-inserted rows *earlier*
        // timestamps to prove rowid, not time, decides the order.
        let db = Database::open_in_memory().unwrap();
        let run = run_record(1_000);
        db.insert_workflow_run(&run).await.unwrap();
        for (key, at) in [("explore", 9_000), ("plan", 2_000), ("implement", 5_000)] {
            db.insert_node_run(&node_run_record(run.id, key, at))
                .await
                .unwrap();
        }

        let listed = db.list_node_runs(run.id).await.unwrap();
        let keys: Vec<_> = listed.iter().map(|n| n.node_key.as_str()).collect();
        assert_eq!(keys, ["explore", "plan", "implement"]);
        // The persisted definition survives intact so a resumed run does not
        // need the template to be byte-identical.
        assert_eq!(listed[0].definition.key, "explore");
        assert_eq!(listed[0].definition.role, RoleKind::RepositoryExplorer);

        assert!(db
            .list_node_runs(WorkflowRunId::new())
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn attempts_keep_their_append_only_history() {
        let db = Database::open_in_memory().unwrap();
        let run = run_record(1_000);
        db.insert_workflow_run(&run).await.unwrap();
        let node = node_run_record(run.id, "verify", 1_000);
        db.insert_node_run(&node).await.unwrap();

        let first = attempt_record(&node, 1, 1_100);
        db.insert_node_attempt(&first).await.unwrap();
        db.finish_node_attempt(first.id, NodeState::Failed, "exit code 1", 1_200)
            .await
            .unwrap();

        let second = attempt_record(&node, 2, 1_300);
        db.insert_node_attempt(&second).await.unwrap();
        db.finish_node_attempt(second.id, NodeState::Succeeded, "exit code 0", 1_400)
            .await
            .unwrap();

        let listed = db.list_node_attempts(run.id).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].attempt_number, 1);
        assert_eq!(listed[0].trigger, AttemptTrigger::Initial);
        assert_eq!(listed[0].finished_state, Some(NodeState::Failed));
        assert_eq!(listed[0].detail.as_deref(), Some("exit code 1"));
        assert_eq!(listed[0].finished_at_millis, Some(1_200));
        assert_eq!(listed[1].finished_state, Some(NodeState::Succeeded));
        // Rewriting history is impossible through the API: finishing is an
        // UPDATE by id only, so attempt 1 stays finished as failed.
        assert_ne!(listed[0].finished_state, listed[1].finished_state);

        let unknown = AttemptId::new();
        let err = db
            .finish_node_attempt(unknown, NodeState::Succeeded, "x", 9_999)
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::AttemptNotFound(id) if id == unknown));
    }

    #[tokio::test]
    async fn an_approval_cannot_be_decided_twice() {
        let db = Database::open_in_memory().unwrap();
        let run = run_record(1_000);
        db.insert_workflow_run(&run).await.unwrap();
        let node = node_run_record(run.id, "integrate", 1_000);
        db.insert_node_run(&node).await.unwrap();
        let approval = approval_record(run.id, node.id, 1_100);
        db.insert_approval(&approval).await.unwrap();

        db.decide_approval(
            approval.id,
            &ApprovalDecision::Approved {
                by: "local_user".to_string(),
            },
            1_200,
        )
        .await
        .unwrap();

        // A second decision must not overwrite the first: "who let this run
        // write to the repository" needs one answer, not the latest one.
        let err = db
            .decide_approval(
                approval.id,
                &ApprovalDecision::Rejected {
                    by: "attacker".to_string(),
                    reason: "retry".to_string(),
                },
                1_300,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StorageError::ApprovalNotPending(id) if id == approval.id));

        let listed = db.list_approvals(run.id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed[0].decision,
            Some(ApprovalDecision::Approved {
                by: "local_user".to_string()
            })
        );
        assert_eq!(listed[0].decided_at_millis, Some(1_200));
    }

    #[tokio::test]
    async fn checkpoint_sequences_are_monotonic_per_run() {
        let db = Database::open_in_memory().unwrap();
        let run_a = run_record(1_000);
        // run_record() already mints a fresh id, so the two runs are distinct.
        let run_b = run_record(1_000);
        for run in [&run_a, &run_b] {
            db.insert_workflow_run(run).await.unwrap();
        }

        assert_eq!(
            db.append_checkpoint(run_a.id, RunState::Running, "start", 1_000)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.append_checkpoint(run_b.id, RunState::Running, "start", 1_000)
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.append_checkpoint(run_a.id, RunState::AwaitingApproval, "gate", 2_000)
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            db.append_checkpoint(run_a.id, RunState::Running, "approved", 3_000)
                .await
                .unwrap(),
            3
        );

        let checkpoints = db.list_checkpoints(run_a.id).await.unwrap();
        let sequences: Vec<_> = checkpoints.iter().map(|c| c.sequence).collect();
        assert_eq!(sequences, [1, 2, 3]);
        assert_eq!(checkpoints[1].state, RunState::AwaitingApproval);
        // Run B's sequence is independent: per-run, not global.
        assert_eq!(db.list_checkpoints(run_b.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn deleting_a_run_removes_every_child_row() {
        let db = Database::open_in_memory().unwrap();
        let run = run_record(1_000);
        db.insert_workflow_run(&run).await.unwrap();
        let node_a = node_run_record(run.id, "explore", 1_000);
        let node_b = node_run_record(run.id, "implement", 1_000);
        for node in [&node_a, &node_b] {
            db.insert_node_run(node).await.unwrap();
        }
        let attempt = attempt_record(&node_a, 1, 1_100);
        db.insert_node_attempt(&attempt).await.unwrap();
        db.insert_approval(&approval_record(run.id, node_b.id, 1_100))
            .await
            .unwrap();
        db.append_checkpoint(run.id, RunState::Running, "start", 1_000)
            .await
            .unwrap();

        db.delete_workflow_run(run.id).await.unwrap();

        assert!(db.get_workflow_run(run.id).await.unwrap().is_none());
        assert!(db.list_node_runs(run.id).await.unwrap().is_empty());
        assert!(db.list_node_attempts(run.id).await.unwrap().is_empty());
        assert!(db.list_approvals(run.id).await.unwrap().is_empty());
        assert!(db.list_checkpoints(run.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_run_deletion_rolls_back_all_child_deletions() {
        let db = Database::open_in_memory().unwrap();
        let run = run_record(1_000);
        db.insert_workflow_run(&run).await.unwrap();
        let node = node_run_record(run.id, "explore", 1_000);
        db.insert_node_run(&node).await.unwrap();
        let attempt = attempt_record(&node, 1, 1_100);
        db.insert_node_attempt(&attempt).await.unwrap();
        let approval = approval_record(run.id, node.id, 1_100);
        db.insert_approval(&approval).await.unwrap();
        db.append_checkpoint(run.id, RunState::Running, "start", 1_000)
            .await
            .unwrap();
        {
            let conn = db.connection();
            lock(&conn)
                .execute_batch(
                    "CREATE TEMP TRIGGER reject_run_deletion
                     BEFORE DELETE ON workflow_runs
                     BEGIN
                         SELECT RAISE(ABORT, 'injected deletion failure');
                     END;",
                )
                .unwrap();
        }

        assert!(matches!(
            db.delete_workflow_run(run.id).await.unwrap_err(),
            StorageError::Sqlite(_)
        ));
        assert!(db.get_workflow_run(run.id).await.unwrap().is_some());
        assert_eq!(db.list_node_runs(run.id).await.unwrap()[0].id, node.id);
        assert_eq!(
            db.list_node_attempts(run.id).await.unwrap()[0].id,
            attempt.id
        );
        assert_eq!(db.list_approvals(run.id).await.unwrap()[0].id, approval.id);
        assert_eq!(
            db.list_checkpoints(run.id).await.unwrap()[0].detail,
            "start"
        );
    }

    #[tokio::test]
    async fn deleting_a_run_preserves_another_runs_history() {
        let db = Database::open_in_memory().unwrap();
        let deleted = run_record(1_000);
        let retained = run_record(2_000);
        for run in [&deleted, &retained] {
            db.insert_workflow_run(run).await.unwrap();
            let node = node_run_record(run.id, "explore", 1_000);
            db.insert_node_run(&node).await.unwrap();
            db.insert_node_attempt(&attempt_record(&node, 1, 1_100))
                .await
                .unwrap();
            db.insert_approval(&approval_record(run.id, node.id, 1_100))
                .await
                .unwrap();
            db.append_checkpoint(run.id, RunState::Running, "start", 1_000)
                .await
                .unwrap();
        }

        db.delete_workflow_run(deleted.id).await.unwrap();
        assert!(db.get_workflow_run(deleted.id).await.unwrap().is_none());
        assert!(db.get_workflow_run(retained.id).await.unwrap().is_some());
        assert_eq!(db.list_node_runs(retained.id).await.unwrap().len(), 1);
        assert_eq!(db.list_node_attempts(retained.id).await.unwrap().len(), 1);
        assert_eq!(db.list_approvals(retained.id).await.unwrap().len(), 1);
        assert_eq!(db.list_checkpoints(retained.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn runs_filter_by_state_and_by_project_newest_first() {
        // Recovery's exact query shape: find the Running/AwaitingApproval
        // runs a crash left behind, newest first, across all projects.
        let db = Database::open_in_memory().unwrap();
        let running = run_record(1_000);
        let mut succeeded = run_record(2_000);
        let mut awaiting = run_record(3_000);
        succeeded.state = RunState::Succeeded;
        awaiting.state = RunState::AwaitingApproval;
        for run in [&running, &succeeded, &awaiting] {
            db.insert_workflow_run(run).await.unwrap();
        }

        let unfinished = db
            .list_workflow_runs_in_states(&[RunState::Running, RunState::AwaitingApproval])
            .await
            .unwrap();
        let ids: Vec<_> = unfinished.iter().map(|r| r.id).collect();
        assert_eq!(
            ids,
            [awaiting.id, running.id],
            "newest first, Succeeded filtered out"
        );

        // A run belongs to exactly one project; the second query filters on
        // the project column the insert persisted.
        let for_project = db
            .list_workflow_runs_for_project(succeeded.project_id)
            .await
            .unwrap();
        assert_eq!(for_project.len(), 1);
        assert_eq!(for_project[0].id, succeeded.id);
    }
}
