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

pub(crate) fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(V1_INITIAL_SCHEMA),
        M::up(V2_CORRELATION_INDEXES),
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
            matches!(version, SchemaVersion::Inside(n) if n.get() == 2),
            "expected schema version 2, got {version:?}"
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
            SchemaVersion::Inside(n) if n.get() == 2
        ));

        // And the V2 index must actually exist now -- proves V2 really
        // ran, not just that current_version reports 2.
        let index_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_events_workflow_run_id'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(index_count, 1);
    }
}
