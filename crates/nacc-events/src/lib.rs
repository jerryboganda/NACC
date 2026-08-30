//! Durable event model backing the audit trail and run history (master plan
//! S6, S22 -- Phase 2 scope for the real event store).
//!
//! This crate defines the shape of the normalized event stream; it does no
//! I/O itself. `nacc-storage`'s `EventRepository` (Phase 2) is the only
//! thing that persists an [`Event`], matching the same domain/storage split
//! `nacc-provider-core` (Phase 1) established for capability data: pure
//! types here, SQLite-backed durability there.

use serde::{Deserialize, Serialize};

use nacc_domain::{
    AttemptId, AuditEventId, EventId, ModelId, NodeRunId, PermissionProfile, ProjectId, ProviderId,
    ReasoningLevel, WorkflowRunId,
};

/// Errors constructing an [`Event`]. No I/O happens in this crate, so the
/// only failure mode is a structurally invalid event -- see
/// [`Event::new`].
#[derive(Debug, thiserror::Error)]
pub enum EventsError {
    /// An event correlated to nothing is very likely a bug: nobody could
    /// ever find it again by project, workflow run, node run, or attempt.
    /// Master plan S22 requires every event carry correlation IDs; this is
    /// that requirement enforced at construction rather than left as a
    /// convention callers might forget.
    #[error(
        "event has no correlation id set (need at least one of project_id, \
         workflow_run_id, node_run_id, attempt_id) -- an uncorrelated event \
         can never be found again by S22's audit/log filters"
    )]
    MissingCorrelation,
}

pub type Result<T> = std::result::Result<T, EventsError>;

/// The normalized event vocabulary (master plan S8.2): what every provider
/// adapter's raw output is mapped to before workflow logic or the audit
/// trail ever sees it. "Raw provider streams may be retained in redacted
/// diagnostic logs, but workflow decisions must use normalized events" --
/// this enum is deliberately closed (no `Other(String)` escape hatch) for
/// that reason: an adapter that produces something outside this vocabulary
/// has a real mapping gap to fix, not a case to silently pass through.
#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    SessionStarted,
    AssistantTextDelta,
    /// Reasoning *status* only -- master plan S18: "Do not store or expose
    /// hidden chain-of-thought." This variant exists to say reasoning is
    /// happening, never to carry its content.
    ReasoningStatus,
    ToolRequested,
    ToolApproved,
    ToolDenied,
    ToolStarted,
    ToolOutputDelta,
    FileChanged,
    CommandStarted,
    CommandOutput,
    CommandCompleted,
    PlanArtifactEmitted,
    HandoffEmitted,
    UsageUpdated,
    ApprovalRequested,
    Warning,
    RecoverableError,
    TerminalError,
    SessionCompleted,
    SessionCancelled,
}

/// One entry in the durable, normalized event stream. At least one
/// correlation ID must be set (enforced by [`Event::new`], not left to
/// convention) so every event can be found again by project, workflow run,
/// node run, or attempt -- the S22 correlation IDs this crate's Phase 2
/// scope covers. The remaining S22 correlation IDs (application session,
/// provider session, process, worktree, GitHub run) are added once their
/// owning entity is modeled, by that entity's own target phase.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct Event {
    pub id: EventId,
    pub project_id: Option<ProjectId>,
    pub workflow_run_id: Option<WorkflowRunId>,
    pub node_run_id: Option<NodeRunId>,
    pub attempt_id: Option<AttemptId>,
    pub event_type: EventType,
    /// Event-specific data. Deliberately a loose JSON value rather than a
    /// per-`EventType` struct: the exact payload shape each provider
    /// adapter fills in is Phase 5+/8 work, once real adapters exist to
    /// design it against. Never hidden reasoning content (S18).
    pub payload: serde_json::Value,
    /// Milliseconds since the Unix epoch -- see
    /// `nacc_provider_core::capability::CapabilitySnapshot::captured_at_millis`
    /// for why this is a plain integer rather than `std::time::SystemTime`.
    pub created_at_millis: u64,
}

impl Event {
    /// Construct a new event, generating a fresh [`EventId`]. Fails if no
    /// correlation ID is set -- see [`EventsError::MissingCorrelation`].
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        project_id: Option<ProjectId>,
        workflow_run_id: Option<WorkflowRunId>,
        node_run_id: Option<NodeRunId>,
        attempt_id: Option<AttemptId>,
        event_type: EventType,
        payload: serde_json::Value,
        created_at_millis: u64,
    ) -> Result<Self> {
        if project_id.is_none()
            && workflow_run_id.is_none()
            && node_run_id.is_none()
            && attempt_id.is_none()
        {
            return Err(EventsError::MissingCorrelation);
        }
        Ok(Self {
            id: EventId::new(),
            project_id,
            workflow_run_id,
            node_run_id,
            attempt_id,
            event_type,
            payload,
            created_at_millis,
        })
    }
}

/// One entry in the audit trail (master plan S7's `AuditEvent`; S22's
/// required audit-record fields: who/what initiated an action, requested
/// vs. actual provider/model, effective reasoning/permission profile,
/// command executable and redacted arguments, working directory). Distinct
/// from [`Event`]: an `Event` is the normalized operational stream a
/// workflow reacts to, an `AuditRecord` is the security/compliance trail a
/// human reviews after the fact -- richer, narrower, and never dropped for
/// noise-reduction the way an operational event stream might be.
#[derive(Clone, Debug, Serialize, Deserialize, specta::Type)]
pub struct AuditRecord {
    pub id: AuditEventId,
    pub actor: String,
    pub action: String,
    pub project_id: Option<ProjectId>,
    pub workflow_run_id: Option<WorkflowRunId>,
    pub node_run_id: Option<NodeRunId>,
    pub attempt_id: Option<AttemptId>,
    pub requested_provider: Option<ProviderId>,
    pub actual_provider: Option<ProviderId>,
    pub requested_model: Option<ModelId>,
    pub actual_model: Option<ModelId>,
    pub effective_reasoning_level: Option<ReasoningLevel>,
    pub effective_permission_profile: Option<PermissionProfile>,
    pub command_executable: Option<String>,
    /// Master plan S13.5's redaction layer is explicit Phase 11 scope; it
    /// does not exist yet. This field exists now so S22's required audit
    /// shape is real from Phase 2 rather than retrofitted later, but
    /// **until Phase 11 lands, callers must redact secrets themselves
    /// before constructing this record** -- nothing in this crate or in
    /// `nacc-storage` redacts anything on the way in.
    pub redacted_arguments: Vec<String>,
    pub working_directory: Option<String>,
    pub created_at_millis: u64,
}

impl AuditRecord {
    /// Construct a minimal audit record with a fresh [`AuditEventId`].
    /// Unlike [`Event::new`], this is infallible: `actor` and `action` are
    /// required positional arguments, so there is no missing-correlation
    /// state to reject. The provider/model/reasoning/permission/command
    /// fields default to `None`/empty; set them directly (all fields are
    /// `pub`) once the caller has that information.
    pub fn new(
        actor: String,
        action: String,
        project_id: Option<ProjectId>,
        workflow_run_id: Option<WorkflowRunId>,
        node_run_id: Option<NodeRunId>,
        attempt_id: Option<AttemptId>,
        created_at_millis: u64,
    ) -> Self {
        Self {
            id: AuditEventId::new(),
            actor,
            action,
            project_id,
            workflow_run_id,
            node_run_id,
            attempt_id,
            requested_provider: None,
            actual_provider: None,
            requested_model: None,
            actual_model: None,
            effective_reasoning_level: None,
            effective_permission_profile: None,
            command_executable: None,
            redacted_arguments: Vec::new(),
            working_directory: None,
            created_at_millis,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_event_generates_a_fresh_id_when_correlated() {
        let run_id = WorkflowRunId::new();
        let event = Event::new(
            None,
            Some(run_id),
            None,
            None,
            EventType::SessionStarted,
            serde_json::json!({"detail": "smoke test"}),
            1_735_000_000_000,
        )
        .expect("a workflow-run-correlated event must construct");
        assert_eq!(event.workflow_run_id, Some(run_id));
    }

    #[test]
    fn uncorrelated_event_is_rejected_not_silently_accepted() {
        let result = Event::new(
            None,
            None,
            None,
            None,
            EventType::Warning,
            serde_json::json!(null),
            1_735_000_000_000,
        );
        assert!(matches!(result, Err(EventsError::MissingCorrelation)));
    }

    #[test]
    fn event_roundtrips_through_json() {
        let event = Event::new(
            Some(ProjectId::new()),
            None,
            None,
            None,
            EventType::CommandCompleted,
            serde_json::json!({"exit_code": 0}),
            1_735_000_000_000,
        )
        .unwrap();
        let json = serde_json::to_string(&event).unwrap();
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, event.id);
        assert_eq!(back.event_type, EventType::CommandCompleted);
    }

    #[test]
    fn event_type_json_is_snake_case() {
        let json = serde_json::to_string(&EventType::AssistantTextDelta).unwrap();
        assert_eq!(json, "\"assistant_text_delta\"");
    }

    #[test]
    fn audit_record_defaults_are_absent_not_placeholder_values() {
        let record = AuditRecord::new(
            "local_user".to_string(),
            "role_profile.create".to_string(),
            None,
            None,
            None,
            None,
            1_735_000_000_000,
        );
        // A field NACC never observed must read as absent, never as a
        // silently-guessed default -- the same "never silently downgrade"
        // principle that governs CapabilitySnapshot (master plan S2.7).
        assert!(record.requested_provider.is_none());
        assert!(record.redacted_arguments.is_empty());
    }

    #[test]
    fn audit_record_roundtrips_through_json_with_fields_set() {
        let mut record = AuditRecord::new(
            "role_matrix".to_string(),
            "launch_attempt".to_string(),
            None,
            Some(WorkflowRunId::new()),
            None,
            None,
            1_735_000_000_000,
        );
        record.requested_provider = Some(ProviderId::Claude);
        record.effective_reasoning_level = Some(ReasoningLevel::High);
        record.redacted_arguments = vec!["--effort".to_string(), "high".to_string()];

        let json = serde_json::to_string(&record).unwrap();
        let back: AuditRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, record.id);
        assert_eq!(back.requested_provider, Some(ProviderId::Claude));
        assert_eq!(back.redacted_arguments, record.redacted_arguments);
    }
}
