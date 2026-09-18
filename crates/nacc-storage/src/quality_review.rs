//! Durable Phase 9 acceptance evidence (master plan S4.4, S17.10, S17.11).
//! Quality gates and review findings are append-only facts correlated to the
//! workflow/node/attempt that produced them. Reads are explicitly bounded
//! and newest-first so operational UI surfaces never materialize an entire
//! evidence history into memory.

use rusqlite::{params, Row};

use nacc_domain::{AttemptId, NodeRunId, WorkflowRunId};
use nacc_quality::QualityEvidence;
use nacc_review::{ReviewFinding, Severity};

use crate::{lock, Database, Result, StorageError};

const QUALITY_COLUMNS: &str = "workflow_run_id, node_run_id, attempt_id, gate, command, passed, \
     exit_code, timed_out, duration_ms, log_tail, created_at_millis";
const REVIEW_COLUMNS: &str = "workflow_run_id, node_run_id, attempt_id, node_key, file, line, \
     severity, summary, evidence, created_at_millis";

#[derive(Clone, Debug, PartialEq)]
pub struct QualityGateRecord {
    pub workflow_run_id: WorkflowRunId,
    pub node_run_id: NodeRunId,
    pub attempt_id: Option<AttemptId>,
    pub evidence: QualityEvidence,
    pub created_at_millis: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReviewFindingRecord {
    pub workflow_run_id: WorkflowRunId,
    pub node_run_id: NodeRunId,
    pub attempt_id: Option<AttemptId>,
    pub finding: ReviewFinding,
    pub created_at_millis: u64,
}

struct RawQualityRow {
    workflow_run_id: String,
    node_run_id: String,
    attempt_id: Option<String>,
    gate: String,
    command: String,
    passed: i64,
    exit_code: Option<i32>,
    timed_out: i64,
    duration_ms: i64,
    log_tail: String,
    created_at_millis: i64,
}

struct RawReviewRow {
    workflow_run_id: String,
    node_run_id: String,
    attempt_id: Option<String>,
    node_key: String,
    file: String,
    line: Option<i64>,
    severity: String,
    summary: String,
    evidence: String,
    created_at_millis: i64,
}

fn quality_row(row: &Row<'_>) -> rusqlite::Result<RawQualityRow> {
    Ok(RawQualityRow {
        workflow_run_id: row.get(0)?,
        node_run_id: row.get(1)?,
        attempt_id: row.get(2)?,
        gate: row.get(3)?,
        command: row.get(4)?,
        passed: row.get(5)?,
        exit_code: row.get(6)?,
        timed_out: row.get(7)?,
        duration_ms: row.get(8)?,
        log_tail: row.get(9)?,
        created_at_millis: row.get(10)?,
    })
}

fn review_row(row: &Row<'_>) -> rusqlite::Result<RawReviewRow> {
    Ok(RawReviewRow {
        workflow_run_id: row.get(0)?,
        node_run_id: row.get(1)?,
        attempt_id: row.get(2)?,
        node_key: row.get(3)?,
        file: row.get(4)?,
        line: row.get(5)?,
        severity: row.get(6)?,
        summary: row.get(7)?,
        evidence: row.get(8)?,
        created_at_millis: row.get(9)?,
    })
}

fn parse_id<T: std::str::FromStr>(entity: &'static str, value: &str) -> Result<T>
where
    T::Err: std::fmt::Display,
{
    value
        .parse::<T>()
        .map_err(|error| StorageError::CorruptStoredValue {
            entity,
            value: value.to_string(),
            detail: error.to_string(),
        })
}

fn parse_bool(entity: &'static str, value: i64) -> Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(StorageError::CorruptStoredValue {
            entity,
            value: other.to_string(),
            detail: "expected SQLite boolean 0 or 1".to_string(),
        }),
    }
}

fn parse_u64(entity: &'static str, value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|error| StorageError::CorruptStoredValue {
        entity,
        value: value.to_string(),
        detail: error.to_string(),
    })
}

fn parse_optional_line(value: Option<i64>) -> Result<Option<u32>> {
    value
        .map(|line| {
            u32::try_from(line).map_err(|error| StorageError::CorruptStoredValue {
                entity: "review_findings.line",
                value: line.to_string(),
                detail: error.to_string(),
            })
        })
        .transpose()
}

fn severity_to_str(severity: Severity) -> &'static str {
    match severity {
        Severity::Blocker => "blocker",
        Severity::Major => "major",
        Severity::Minor => "minor",
        Severity::Note => "note",
    }
}

fn parse_severity(value: &str) -> Result<Severity> {
    match value {
        "blocker" => Ok(Severity::Blocker),
        "major" => Ok(Severity::Major),
        "minor" => Ok(Severity::Minor),
        "note" => Ok(Severity::Note),
        other => Err(StorageError::CorruptStoredValue {
            entity: "review_findings.severity",
            value: other.to_string(),
            detail: "expected blocker, major, minor, or note".to_string(),
        }),
    }
}

fn quality_from_raw(raw: RawQualityRow) -> Result<QualityGateRecord> {
    Ok(QualityGateRecord {
        workflow_run_id: parse_id("quality_gate_results.workflow_run_id", &raw.workflow_run_id)?,
        node_run_id: parse_id("quality_gate_results.node_run_id", &raw.node_run_id)?,
        attempt_id: raw
            .attempt_id
            .as_deref()
            .map(|value| parse_id("quality_gate_results.attempt_id", value))
            .transpose()?,
        evidence: QualityEvidence {
            gate: raw.gate,
            command: raw.command,
            passed: parse_bool("quality_gate_results.passed", raw.passed)?,
            exit_code: raw.exit_code,
            timed_out: parse_bool("quality_gate_results.timed_out", raw.timed_out)?,
            duration_ms: parse_u64("quality_gate_results.duration_ms", raw.duration_ms)?,
            log_tail: raw.log_tail,
        },
        created_at_millis: parse_u64(
            "quality_gate_results.created_at_millis",
            raw.created_at_millis,
        )?,
    })
}

fn review_from_raw(raw: RawReviewRow) -> Result<ReviewFindingRecord> {
    Ok(ReviewFindingRecord {
        workflow_run_id: parse_id("review_findings.workflow_run_id", &raw.workflow_run_id)?,
        node_run_id: parse_id("review_findings.node_run_id", &raw.node_run_id)?,
        attempt_id: raw
            .attempt_id
            .as_deref()
            .map(|value| parse_id("review_findings.attempt_id", value))
            .transpose()?,
        finding: ReviewFinding {
            node_key: raw.node_key,
            file: raw.file,
            line: parse_optional_line(raw.line)?,
            severity: parse_severity(&raw.severity)?,
            summary: raw.summary,
            evidence: raw.evidence,
        },
        created_at_millis: parse_u64("review_findings.created_at_millis", raw.created_at_millis)?,
    })
}

impl Database {
    pub async fn append_quality_gate_result(&self, record: &QualityGateRecord) -> Result<()> {
        let conn = self.connection();
        let record = record.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                &format!(
                    "INSERT INTO quality_gate_results ({QUALITY_COLUMNS}) VALUES \
                     (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
                ),
                params![
                    record.workflow_run_id.to_string(),
                    record.node_run_id.to_string(),
                    record.attempt_id.map(|id| id.to_string()),
                    record.evidence.gate,
                    record.evidence.command,
                    record.evidence.passed,
                    record.evidence.exit_code,
                    record.evidence.timed_out,
                    i64::try_from(record.evidence.duration_ms).map_err(|error| {
                        StorageError::InvalidRecord {
                            entity: "quality gate result duration",
                            detail: error.to_string(),
                        }
                    })?,
                    record.evidence.log_tail,
                    i64::try_from(record.created_at_millis).map_err(|error| {
                        StorageError::InvalidRecord {
                            entity: "quality gate result timestamp",
                            detail: error.to_string(),
                        }
                    })?,
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn append_review_finding(&self, record: &ReviewFindingRecord) -> Result<()> {
        record
            .finding
            .validate()
            .map_err(|detail| StorageError::InvalidRecord {
                entity: "review finding",
                detail,
            })?;

        let conn = self.connection();
        let record = record.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                &format!(
                    "INSERT INTO review_findings ({REVIEW_COLUMNS}) VALUES \
                     (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
                ),
                params![
                    record.workflow_run_id.to_string(),
                    record.node_run_id.to_string(),
                    record.attempt_id.map(|id| id.to_string()),
                    record.finding.node_key,
                    record.finding.file,
                    record.finding.line,
                    severity_to_str(record.finding.severity),
                    record.finding.summary,
                    record.finding.evidence,
                    i64::try_from(record.created_at_millis).map_err(|error| {
                        StorageError::InvalidRecord {
                            entity: "review finding timestamp",
                            detail: error.to_string(),
                        }
                    })?,
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn list_recent_quality_gate_results(
        &self,
        workflow_run_id: Option<WorkflowRunId>,
        node_run_id: Option<NodeRunId>,
        limit: u32,
    ) -> Result<Vec<QualityGateRecord>> {
        let conn = self.connection();
        let workflow_run_id = workflow_run_id.map(|id| id.to_string());
        let node_run_id = node_run_id.map(|id| id.to_string());
        let limit = i64::from(limit);
        let raws = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<RawQualityRow>> {
            let conn = lock(&conn);
            let rows = match (workflow_run_id, node_run_id) {
                (Some(run), Some(node)) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {QUALITY_COLUMNS} FROM quality_gate_results \
                         WHERE workflow_run_id = ?1 AND node_run_id = ?2 \
                         ORDER BY created_at_millis DESC, id DESC LIMIT ?3"
                    ))?;
                    let rows = stmt
                        .query_map(params![run, node, limit], quality_row)?
                        .collect();
                    rows
                }
                (Some(run), None) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {QUALITY_COLUMNS} FROM quality_gate_results \
                         WHERE workflow_run_id = ?1 \
                         ORDER BY created_at_millis DESC, id DESC LIMIT ?2"
                    ))?;
                    let rows = stmt.query_map(params![run, limit], quality_row)?.collect();
                    rows
                }
                (None, Some(node)) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {QUALITY_COLUMNS} FROM quality_gate_results \
                         WHERE node_run_id = ?1 \
                         ORDER BY created_at_millis DESC, id DESC LIMIT ?2"
                    ))?;
                    let rows = stmt.query_map(params![node, limit], quality_row)?.collect();
                    rows
                }
                (None, None) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {QUALITY_COLUMNS} FROM quality_gate_results \
                         ORDER BY created_at_millis DESC, id DESC LIMIT ?1"
                    ))?;
                    let rows = stmt.query_map([limit], quality_row)?.collect();
                    rows
                }
            };
            rows
        })
        .await??;

        raws.into_iter().map(quality_from_raw).collect()
    }

    pub async fn list_recent_review_findings(
        &self,
        workflow_run_id: Option<WorkflowRunId>,
        node_run_id: Option<NodeRunId>,
        limit: u32,
    ) -> Result<Vec<ReviewFindingRecord>> {
        let conn = self.connection();
        let workflow_run_id = workflow_run_id.map(|id| id.to_string());
        let node_run_id = node_run_id.map(|id| id.to_string());
        let limit = i64::from(limit);
        let raws = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<RawReviewRow>> {
            let conn = lock(&conn);
            let rows = match (workflow_run_id, node_run_id) {
                (Some(run), Some(node)) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {REVIEW_COLUMNS} FROM review_findings \
                         WHERE workflow_run_id = ?1 AND node_run_id = ?2 \
                         ORDER BY created_at_millis DESC, id DESC LIMIT ?3"
                    ))?;
                    let rows = stmt
                        .query_map(params![run, node, limit], review_row)?
                        .collect();
                    rows
                }
                (Some(run), None) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {REVIEW_COLUMNS} FROM review_findings \
                         WHERE workflow_run_id = ?1 \
                         ORDER BY created_at_millis DESC, id DESC LIMIT ?2"
                    ))?;
                    let rows = stmt.query_map(params![run, limit], review_row)?.collect();
                    rows
                }
                (None, Some(node)) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {REVIEW_COLUMNS} FROM review_findings \
                         WHERE node_run_id = ?1 \
                         ORDER BY created_at_millis DESC, id DESC LIMIT ?2"
                    ))?;
                    let rows = stmt.query_map(params![node, limit], review_row)?.collect();
                    rows
                }
                (None, None) => {
                    let mut stmt = conn.prepare(&format!(
                        "SELECT {REVIEW_COLUMNS} FROM review_findings \
                         ORDER BY created_at_millis DESC, id DESC LIMIT ?1"
                    ))?;
                    let rows = stmt.query_map([limit], review_row)?.collect();
                    rows
                }
            };
            rows
        })
        .await??;

        raws.into_iter().map(review_from_raw).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quality(
        workflow_run_id: WorkflowRunId,
        node_run_id: NodeRunId,
        attempt_id: Option<AttemptId>,
        gate: &str,
        passed: bool,
        created_at_millis: u64,
    ) -> QualityGateRecord {
        QualityGateRecord {
            workflow_run_id,
            node_run_id,
            attempt_id,
            evidence: QualityEvidence {
                gate: gate.to_string(),
                command: format!("cargo test -p {gate}"),
                passed,
                exit_code: Some(if passed { 0 } else { 1 }),
                timed_out: false,
                duration_ms: 25,
                log_tail: format!("{gate} output"),
            },
            created_at_millis,
        }
    }

    fn finding(
        workflow_run_id: WorkflowRunId,
        node_run_id: NodeRunId,
        attempt_id: Option<AttemptId>,
        severity: Severity,
        summary: &str,
        created_at_millis: u64,
    ) -> ReviewFindingRecord {
        ReviewFindingRecord {
            workflow_run_id,
            node_run_id,
            attempt_id,
            finding: ReviewFinding {
                node_key: "review".to_string(),
                file: "src/lib.rs".to_string(),
                line: Some(42),
                severity,
                summary: summary.to_string(),
                evidence: format!("concrete evidence for {summary}"),
            },
            created_at_millis,
        }
    }

    #[tokio::test]
    async fn quality_results_round_trip_and_list_newest_first_with_limit() {
        let db = Database::open_in_memory().unwrap();
        let run = WorkflowRunId::new();
        let node = NodeRunId::new();
        let attempt = AttemptId::new();
        for record in [
            quality(run, node, Some(attempt), "fmt", true, 10),
            quality(run, node, Some(attempt), "clippy", false, 20),
            quality(run, node, Some(attempt), "test", true, 30),
        ] {
            db.append_quality_gate_result(&record).await.unwrap();
        }

        let rows = db
            .list_recent_quality_gate_results(None, None, 2)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].evidence.gate, "test");
        assert_eq!(rows[1].evidence.gate, "clippy");
        assert!(!rows[1].evidence.passed);
        assert_eq!(rows[0].attempt_id, Some(attempt));
    }

    #[tokio::test]
    async fn quality_results_filter_by_workflow_and_node() {
        let db = Database::open_in_memory().unwrap();
        let run_a = WorkflowRunId::new();
        let run_b = WorkflowRunId::new();
        let node_a = NodeRunId::new();
        let node_b = NodeRunId::new();
        for record in [
            quality(run_a, node_a, None, "a", true, 10),
            quality(run_a, node_b, None, "b", true, 20),
            quality(run_b, node_b, None, "c", true, 30),
        ] {
            db.append_quality_gate_result(&record).await.unwrap();
        }

        let rows = db
            .list_recent_quality_gate_results(Some(run_a), Some(node_b), 10)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].evidence.gate, "b");

        let node_rows = db
            .list_recent_quality_gate_results(None, Some(node_b), 10)
            .await
            .unwrap();
        assert_eq!(node_rows.len(), 2);
    }

    #[tokio::test]
    async fn review_findings_round_trip_filter_order_and_limit() {
        let db = Database::open_in_memory().unwrap();
        let run_a = WorkflowRunId::new();
        let run_b = WorkflowRunId::new();
        let node = NodeRunId::new();
        let attempt = AttemptId::new();
        for record in [
            finding(run_a, node, Some(attempt), Severity::Minor, "minor", 10),
            finding(run_a, node, Some(attempt), Severity::Major, "major", 20),
            finding(run_b, node, None, Severity::Blocker, "blocker", 30),
        ] {
            db.append_review_finding(&record).await.unwrap();
        }

        let run_rows = db
            .list_recent_review_findings(Some(run_a), None, 1)
            .await
            .unwrap();
        assert_eq!(run_rows.len(), 1);
        assert_eq!(run_rows[0].finding.summary, "major");
        assert_eq!(run_rows[0].finding.severity, Severity::Major);
        assert_eq!(run_rows[0].attempt_id, Some(attempt));

        let node_rows = db
            .list_recent_review_findings(None, Some(node), 10)
            .await
            .unwrap();
        assert_eq!(node_rows.len(), 3);
        assert_eq!(node_rows[0].finding.summary, "blocker");
    }

    #[tokio::test]
    async fn invalid_review_finding_is_rejected_before_persistence() {
        let db = Database::open_in_memory().unwrap();
        let mut record = finding(
            WorkflowRunId::new(),
            NodeRunId::new(),
            None,
            Severity::Major,
            "missing evidence",
            10,
        );
        record.finding.evidence.clear();

        let error = db.append_review_finding(&record).await.unwrap_err();
        assert!(matches!(error, StorageError::InvalidRecord { .. }));
        assert!(db
            .list_recent_review_findings(None, None, 10)
            .await
            .unwrap()
            .is_empty());
    }
}
