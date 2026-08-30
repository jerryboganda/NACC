//! Durable persistence for `nacc_events::Event` (master plan S4.4's "event
//! stream" data group; S6/S8.2/S22). This module is pure I/O -- the event
//! shape itself, and the closed normalized vocabulary it carries, live in
//! `nacc-events`, which has no SQLite dependency at all.

use rusqlite::{params, Row};

use nacc_domain::{AttemptId, EventId, NodeRunId, ProjectId, WorkflowRunId};
use nacc_events::{Event, EventType};

use crate::{lock, Database, Result, StorageError};

const SELECT_COLUMNS: &str = "id, project_id, workflow_run_id, node_run_id, attempt_id, \
     event_type_json, payload_json, created_at_millis";

struct RawEventRow {
    id: String,
    project_id: Option<String>,
    workflow_run_id: Option<String>,
    node_run_id: Option<String>,
    attempt_id: Option<String>,
    event_type_json: String,
    payload_json: String,
    created_at_millis: i64,
}

fn row_to_raw(row: &Row<'_>) -> rusqlite::Result<RawEventRow> {
    Ok(RawEventRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        workflow_run_id: row.get(2)?,
        node_run_id: row.get(3)?,
        attempt_id: row.get(4)?,
        event_type_json: row.get(5)?,
        payload_json: row.get(6)?,
        created_at_millis: row.get(7)?,
    })
}

fn parse_id<T: std::str::FromStr>(entity: &'static str, value: &str) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    value.parse().map_err(|e| StorageError::CorruptStoredValue {
        entity,
        value: value.to_string(),
        detail: format!("{e}"),
    })
}

fn raw_to_event(raw: RawEventRow) -> Result<Event> {
    Ok(Event {
        id: parse_id::<EventId>("Event.id", &raw.id)?,
        project_id: raw
            .project_id
            .as_deref()
            .map(|s| parse_id::<ProjectId>("Event.project_id", s))
            .transpose()?,
        workflow_run_id: raw
            .workflow_run_id
            .as_deref()
            .map(|s| parse_id::<WorkflowRunId>("Event.workflow_run_id", s))
            .transpose()?,
        node_run_id: raw
            .node_run_id
            .as_deref()
            .map(|s| parse_id::<NodeRunId>("Event.node_run_id", s))
            .transpose()?,
        attempt_id: raw
            .attempt_id
            .as_deref()
            .map(|s| parse_id::<AttemptId>("Event.attempt_id", s))
            .transpose()?,
        event_type: serde_json::from_str::<EventType>(&raw.event_type_json)?,
        payload: serde_json::from_str(&raw.payload_json)?,
        created_at_millis: raw.created_at_millis as u64,
    })
}

impl Database {
    pub async fn append_event(&self, event: &Event) -> Result<()> {
        let conn = self.connection();
        let event = event.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                &format!(
                    "INSERT INTO events ({SELECT_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"
                ),
                params![
                    event.id.to_string(),
                    event.project_id.map(|id| id.to_string()),
                    event.workflow_run_id.map(|id| id.to_string()),
                    event.node_run_id.map(|id| id.to_string()),
                    event.attempt_id.map(|id| id.to_string()),
                    serde_json::to_string(&event.event_type)?,
                    serde_json::to_string(&event.payload)?,
                    event.created_at_millis as i64,
                ],
            )?;
            Ok(())
        })
        .await
        .expect("storage worker thread panicked")
    }

    /// Events for one workflow run, oldest first -- the shape a run-history
    /// or live-activity view needs (master plan S22's "workflow run"
    /// correlation ID).
    pub async fn list_events_for_workflow_run(
        &self,
        workflow_run_id: WorkflowRunId,
    ) -> Result<Vec<Event>> {
        let conn = self.connection();
        let id_str = workflow_run_id.to_string();
        let raws = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<RawEventRow>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(&format!(
                "SELECT {SELECT_COLUMNS} FROM events WHERE workflow_run_id = ?1 ORDER BY created_at_millis"
            ))?;
            // See the identical comment in audit.rs's list function: `?`
            // in tail position here creates a temporary that would
            // otherwise outlive `conn`/`stmt` per Rust's drop order --
            // real E0597, caught by this workspace's own CI.
            let rows = stmt.query_map([id_str], row_to_raw)?.collect();
            rows
        })
        .await
        .expect("storage worker thread panicked")?;

        raws.into_iter().map(raw_to_event).collect()
    }
}

#[cfg(test)]
mod tests {
    use nacc_domain::WorkflowRunId;
    use nacc_events::{Event, EventType};

    use crate::Database;

    #[tokio::test]
    async fn appended_event_is_returned_by_workflow_run_lookup() {
        let db = Database::open_in_memory().unwrap();
        let run_id = WorkflowRunId::new();
        let event = Event::new(
            None,
            Some(run_id),
            None,
            None,
            EventType::SessionStarted,
            serde_json::json!({"provider": "claude"}),
            1_735_000_000_000,
        )
        .unwrap();

        db.append_event(&event).await.unwrap();

        let found = db.list_events_for_workflow_run(run_id).await.unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, event.id);
        assert_eq!(found[0].event_type, EventType::SessionStarted);
        assert_eq!(found[0].payload, serde_json::json!({"provider": "claude"}));
    }

    #[tokio::test]
    async fn events_for_a_different_workflow_run_are_not_returned() {
        let db = Database::open_in_memory().unwrap();
        let target_run = WorkflowRunId::new();
        let other_run = WorkflowRunId::new();

        db.append_event(
            &Event::new(
                None,
                Some(other_run),
                None,
                None,
                EventType::Warning,
                serde_json::json!(null),
                1,
            )
            .unwrap(),
        )
        .await
        .unwrap();

        assert!(db
            .list_events_for_workflow_run(target_run)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn events_for_one_workflow_run_are_ordered_oldest_first() {
        let db = Database::open_in_memory().unwrap();
        let run_id = WorkflowRunId::new();
        db.append_event(
            &Event::new(
                None,
                Some(run_id),
                None,
                None,
                EventType::SessionStarted,
                serde_json::json!(null),
                100,
            )
            .unwrap(),
        )
        .await
        .unwrap();
        db.append_event(
            &Event::new(
                None,
                Some(run_id),
                None,
                None,
                EventType::SessionCompleted,
                serde_json::json!(null),
                200,
            )
            .unwrap(),
        )
        .await
        .unwrap();

        let found = db.list_events_for_workflow_run(run_id).await.unwrap();
        assert_eq!(found[0].event_type, EventType::SessionStarted);
        assert_eq!(found[1].event_type, EventType::SessionCompleted);
    }
}
