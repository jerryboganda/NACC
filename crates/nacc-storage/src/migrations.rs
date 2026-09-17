//! Versioned, embedded, transactional schema migrations (master plan
//! S4.4). Applied via `rusqlite_migration`, which tracks the applied
//! version in SQLite's own `user_version` pragma and applies each pending
//! migration inside a transaction.
//!
//! Column convention used throughout this schema: an enum-typed NACC
//! domain value (`ProviderId`, `ThinkingMode`, `ReasoningLevel`,
//! `PermissionProfile`, `RoleKind`, `EventType`, ...) is stored as the
//! `serde_json` encoding of that value in a `..._json TEXT` column, read
//! back via `serde_json::from_str`. This reuses each type's existing
//! `Serialize`/`Deserialize` derive as the single source of truth for its
//! wire *and* storage representation, instead of hand-writing a second,
//! parallel `Display`/`FromStr` mapping per enum that could silently drift
//! from the JSON one. A plain string value (`ModelId`, a UUID-backed
//! strong ID) is stored as plain `TEXT` instead, via that type's own
//! `Display`/`FromStr` -- there is no JSON-specific meaning to preserve for
//! those.
use rusqlite_migration::{Migrations, M};

const V1_INITIAL_SCHEMA: &str = r#"
CREATE TABLE app_settings (
    key                 TEXT PRIMARY KEY,
    value               TEXT NOT NULL,
    updated_at_millis   INTEGER NOT NULL
);

CREATE TABLE role_profiles (
    id                              TEXT PRIMARY KEY,
    name                            TEXT NOT NULL,
    role_kind_json                  TEXT NOT NULL,
    provider_id_json                TEXT,
    model_id                        TEXT,
    thinking_mode_json              TEXT NOT NULL,
    reasoning_level_json            TEXT NOT NULL,
    permission_profile_json         TEXT NOT NULL,
    enabled                         INTEGER NOT NULL DEFAULT 1,
    created_at_millis               INTEGER NOT NULL,
    updated_at_millis               INTEGER NOT NULL
);

CREATE TABLE events (
    id                  TEXT PRIMARY KEY,
    project_id          TEXT,
    workflow_run_id     TEXT,
    node_run_id         TEXT,
    attempt_id          TEXT,
    event_type_json     TEXT NOT NULL,
    payload_json        TEXT NOT NULL,
    created_at_millis   INTEGER NOT NULL
);

CREATE TABLE audit_events (
    id                                      TEXT PRIMARY KEY,
    actor                                   TEXT NOT NULL,
    action                                  TEXT NOT NULL,
    project_id                              TEXT,
    workflow_run_id                         TEXT,
    node_run_id                             TEXT,
    attempt_id                              TEXT,
    requested_provider_json                 TEXT,
    actual_provider_json                    TEXT,
    requested_model                         TEXT,
    actual_model                            TEXT,
    effective_reasoning_level_json          TEXT,
    effective_permission_profile_json       TEXT,
    command_executable                      TEXT,
    redacted_arguments_json                 TEXT NOT NULL,
    working_directory                       TEXT,
    created_at_millis                       INTEGER NOT NULL
);
"#;

/// Real, additive second migration -- not a placeholder no-op -- so the
/// upgrade-from-an-earlier-version path (master plan S4.4: "tested from
/// every supported prior schema version") is exercised by an actual schema
/// change, proven by this module's own tests below.
const V2_CORRELATION_INDEXES: &str = r#"
CREATE INDEX idx_events_workflow_run_id ON events(workflow_run_id);
CREATE INDEX idx_events_node_run_id ON events(node_run_id);
CREATE INDEX idx_audit_events_workflow_run_id ON audit_events(workflow_run_id);
CREATE INDEX idx_audit_events_node_run_id ON audit_events(node_run_id);
"#;

/// Phase 3's worktree lease table (master plan S16's lifecycle: allocate,
/// integrate, release, quarantine -- plus the owner process id startup
/// reconciliation needs to tell "still running" from "orphaned by a
/// crash"). Additive, like V2: an existing V1/V2 database upgrades in
/// place, proven by the upgrade tests below.
const V3_WORKTREE_LEASES: &str = r#"
CREATE TABLE worktree_leases (
    id                      TEXT PRIMARY KEY,
    project_id              TEXT NOT NULL,
    workflow_run_id         TEXT,
    node_run_id             TEXT,
    path                    TEXT NOT NULL,
    branch                  TEXT NOT NULL,
    base_commit             TEXT NOT NULL,
    head_commit             TEXT,
    state_json              TEXT NOT NULL,
    owner_process_id        INTEGER,
    quarantine_reason       TEXT,
    created_at_millis       INTEGER NOT NULL,
    updated_at_millis       INTEGER NOT NULL
);

CREATE INDEX idx_worktree_leases_project_id ON worktree_leases(project_id);
CREATE INDEX idx_worktree_leases_workflow_run_id ON worktree_leases(workflow_run_id);
"#;

/// Phase 4's provider detection and capability records (master plan S4.4's
/// "provider installations, discovered models" data group). Installations
/// are one row per (provider, runtime) because detection is a repeated
/// observation of the same fact; snapshots are append-only because a
/// provider's reported capabilities genuinely change over time and
/// `captured_at_millis` exists precisely to show that.
const V4_PROVIDER_CAPABILITIES: &str = r#"
CREATE TABLE provider_installations (
    provider_json           TEXT NOT NULL,
    runtime_json            TEXT NOT NULL,
    installed               INTEGER NOT NULL,
    executable_path         TEXT,
    version                 TEXT,
    detected_at_millis      INTEGER NOT NULL,
    PRIMARY KEY (provider_json, runtime_json)
);

CREATE TABLE capability_snapshots (
    id                      TEXT PRIMARY KEY,
    provider_json           TEXT NOT NULL,
    runtime_json            TEXT NOT NULL,
    snapshot_json           TEXT NOT NULL,
    captured_at_millis      INTEGER NOT NULL
);

CREATE INDEX idx_capability_snapshots_provider
    ON capability_snapshots(provider_json, captured_at_millis);
"#;

/// Phase 7's durable workflow state (master plan S14.1's state machine,
/// S16's crash recovery). Runs, node runs, attempts, approvals, and a
/// per-run checkpoint sequence -- each a separate table because they have
/// different lifetimes: a node transition rewrites one node row, an attempt
/// is append-only, an approval carries a human decision, and checkpoints are
/// a monotonic snapshot sequence.
const V5_WORKFLOW_STATE: &str = r#"
CREATE TABLE workflow_runs (
    id                      TEXT PRIMARY KEY,
    project_id              TEXT NOT NULL,
    template_name           TEXT NOT NULL,
    state_json              TEXT NOT NULL,
    note                    TEXT,
    created_at_millis       INTEGER NOT NULL,
    updated_at_millis       INTEGER NOT NULL
);

CREATE INDEX idx_workflow_runs_project_id ON workflow_runs(project_id);
CREATE INDEX idx_workflow_runs_state ON workflow_runs(state_json);

CREATE TABLE node_runs (
    id                      TEXT PRIMARY KEY,
    workflow_run_id         TEXT NOT NULL,
    node_key                TEXT NOT NULL,
    title                   TEXT NOT NULL,
    state_json              TEXT NOT NULL,
    attempts                INTEGER NOT NULL,
    definition_json         TEXT NOT NULL,
    last_detail             TEXT,
    created_at_millis       INTEGER NOT NULL,
    updated_at_millis       INTEGER NOT NULL
);

CREATE INDEX idx_node_runs_run ON node_runs(workflow_run_id);

CREATE TABLE node_attempts (
    id                      TEXT PRIMARY KEY,
    node_run_id             TEXT NOT NULL,
    workflow_run_id         TEXT NOT NULL,
    attempt_number          INTEGER NOT NULL,
    trigger_json            TEXT NOT NULL,
    finished_state_json     TEXT,
    provider_json           TEXT,
    detail                  TEXT,
    started_at_millis       INTEGER NOT NULL,
    finished_at_millis      INTEGER
);

CREATE INDEX idx_node_attempts_run ON node_attempts(workflow_run_id);

CREATE TABLE approvals (
    id                      TEXT PRIMARY KEY,
    workflow_run_id         TEXT NOT NULL,
    node_run_id             TEXT NOT NULL,
    summary                 TEXT NOT NULL,
    requested_at_millis     INTEGER NOT NULL,
    decision_json           TEXT,
    decided_at_millis       INTEGER
);

CREATE INDEX idx_approvals_run ON approvals(workflow_run_id);

CREATE TABLE run_checkpoints (
    workflow_run_id         TEXT NOT NULL,
    sequence                INTEGER NOT NULL,
    state_json              TEXT NOT NULL,
    detail                  TEXT NOT NULL,
    created_at_millis       INTEGER NOT NULL,
    PRIMARY KEY (workflow_run_id, sequence)
);
"#;

/// V6: workflow templates become durable data (master plan S24 Phase 7's
/// "versioned DAG templates"). Built-ins are synced from code at startup so
/// the table is always a complete catalog, and user-authored templates live
/// beside them under the same schema. `definition_json` holds the whole
/// [`nacc_domain::WorkflowTemplate`] -- the same shape the engine already
/// instantiates, so a stored template needs no translation to run.
const V6_WORKFLOW_TEMPLATES: &str = r#"
CREATE TABLE workflow_templates (
    name                    TEXT PRIMARY KEY,
    description             TEXT NOT NULL,
    version                 INTEGER NOT NULL,
    is_built_in             INTEGER NOT NULL,
    definition_json         TEXT NOT NULL,
    created_at_millis       INTEGER NOT NULL,
    updated_at_millis       INTEGER NOT NULL
);
"#;

pub(crate) fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(V1_INITIAL_SCHEMA),
        M::up(V2_CORRELATION_INDEXES),
        M::up(V3_WORKTREE_LEASES),
        M::up(V4_PROVIDER_CAPABILITIES),
        M::up(V5_WORKFLOW_STATE),
        M::up(V6_WORKFLOW_TEMPLATES),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use rusqlite_migration::SchemaVersion;

    #[test]
    fn migrations_are_internally_consistent() {
        // rusqlite_migration's own structural validation (ordering,
        // non-empty SQL, ...) -- cheap to run and catches a malformed
        // migration list before it ever touches a real connection.
        migrations()
            .validate()
            .expect("the migration list itself must be well-formed");
    }

    #[test]
    fn fresh_database_lands_on_the_latest_schema_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrations().to_latest(&mut conn).unwrap();
        let version = migrations().current_version(&conn).unwrap();
        assert!(
            matches!(version, SchemaVersion::Inside(n) if n.get() == 6),
            "expected schema version 6, got {version:?}"
        );
    }

    #[test]
    fn database_left_at_an_earlier_schema_version_upgrades_without_losing_data() {
        // Master plan S4.4: "tested from every supported prior schema
        // version." Simulate a database that only ever saw V1 (an
        // existing NACC install being upgraded), write real data into it
        // using V1's shape, then apply the full migration list and
        // confirm both that the data survived and that the schema
        // actually advanced.
        let mut conn = Connection::open_in_memory().unwrap();
        let v1_only = Migrations::new(vec![M::up(V1_INITIAL_SCHEMA)]);
        v1_only.to_latest(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO app_settings (key, value, updated_at_millis) VALUES ('k', 'v', 0)",
            [],
        )
        .unwrap();

        migrations().to_latest(&mut conn).unwrap();

        let value: String = conn
            .query_row("SELECT value FROM app_settings WHERE key = 'k'", [], |r| {
                r.get(0)
            })
            .expect("data written under the V1 schema must survive the upgrade to V2");
        assert_eq!(value, "v");
        assert!(matches!(
            migrations().current_version(&conn).unwrap(),
            SchemaVersion::Inside(n) if n.get() == 6
        ));

        // And the V2 index must actually exist now -- proves V2 really
        // ran, not just that current_version reports 5.
        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_events_workflow_run_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 1);
    }

    #[test]
    fn a_database_left_at_v2_upgrades_to_v3_keeping_its_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        let up_to_v2 = Migrations::new(vec![
            M::up(V1_INITIAL_SCHEMA),
            M::up(V2_CORRELATION_INDEXES),
        ]);
        up_to_v2.to_latest(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO app_settings (key, value, updated_at_millis) VALUES ('schema', 'v2', 7)",
            [],
        )
        .unwrap();

        migrations().to_latest(&mut conn).unwrap();

        let value: String = conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = 'schema'",
                [],
                |r| r.get(0),
            )
            .expect("a row written at V2 must survive the V3 upgrade");
        assert_eq!(value, "v2");
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'worktree_leases'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 1, "V3 must have created the lease table");
    }

    #[test]
    fn a_database_left_at_v3_upgrades_to_v4_keeping_its_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        let up_to_v3 = Migrations::new(vec![
            M::up(V1_INITIAL_SCHEMA),
            M::up(V2_CORRELATION_INDEXES),
            M::up(V3_WORKTREE_LEASES),
        ]);
        up_to_v3.to_latest(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO worktree_leases (
                id, project_id, path, branch, base_commit, state_json,
                created_at_millis, updated_at_millis
             ) VALUES ('lease-1', 'project-1', 'D:\\repo\\.nacc-worktrees\\wt', 'nacc/fix-1', 'abc123', '\"Allocated\"', 10, 10)",
            [],
        )
        .unwrap();

        migrations().to_latest(&mut conn).unwrap();

        let branch: String = conn
            .query_row(
                "SELECT branch FROM worktree_leases WHERE id = 'lease-1'",
                [],
                |r| r.get(0),
            )
            .expect("a lease written at V3 must survive the V4 upgrade");
        assert_eq!(branch, "nacc/fix-1");
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'provider_installations'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            tables, 1,
            "V4 must have created the provider-installation table"
        );
    }

    #[test]
    fn a_database_left_at_v4_upgrades_to_v5_keeping_its_rows() {
        let mut conn = Connection::open_in_memory().unwrap();
        let up_to_v4 = Migrations::new(vec![
            M::up(V1_INITIAL_SCHEMA),
            M::up(V2_CORRELATION_INDEXES),
            M::up(V3_WORKTREE_LEASES),
            M::up(V4_PROVIDER_CAPABILITIES),
        ]);
        up_to_v4.to_latest(&mut conn).unwrap();
        // Both V4 tables get a real row: installations are the (provider,
        // runtime)-keyed detection facts, snapshots the append-only
        // capability observations, and neither may be lost by the upgrade.
        conn.execute(
            "INSERT INTO provider_installations (
                provider_json, runtime_json, installed, executable_path, version,
                detected_at_millis
             ) VALUES ('\"claude\"', '\"native\"', 1, 'C:/bin/claude.exe', '1.0', 20)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO capability_snapshots (
                id, provider_json, runtime_json, snapshot_json, captured_at_millis
             ) VALUES ('snap-1', '\"claude\"', '\"native\"', '{}', 21)",
            [],
        )
        .unwrap();

        migrations().to_latest(&mut conn).unwrap();

        let installed: i64 = conn
            .query_row(
                "SELECT installed FROM provider_installations WHERE provider_json = '\"claude\"'",
                [],
                |r| r.get(0),
            )
            .expect("an installation written at V4 must survive the V5 upgrade");
        assert_eq!(installed, 1);
        let snapshots: i64 = conn
            .query_row("SELECT COUNT(*) FROM capability_snapshots", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(snapshots, 1);
        let tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'workflow_runs'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 1, "V5 must have created the workflow-runs table");
        assert!(matches!(
            migrations().current_version(&conn).unwrap(),
            SchemaVersion::Inside(n) if n.get() == 6
        ));
    }
}
