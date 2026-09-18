//! Workflow templates as durable data (master plan S24 Phase 7's "versioned
//! DAG templates"). Built-ins are synced from code by the app at startup, so
//! this table is always a complete catalog; user-authored templates live
//! beside them under the same schema and are the only rows the GUI may
//! replace or delete. `definition_json` stores the whole
//! [`nacc_domain::WorkflowTemplate`] -- the exact type the engine
//! instantiates -- so a stored template needs no translation to run, and a
//! resumed run keeps following the graph its nodes were created from.

use nacc_domain::WorkflowTemplate;
use rusqlite::OptionalExtension;

use crate::{lock, Database, Result, StorageError};

/// One stored workflow template. `version` starts at 1 and increments on
/// every content change, so "which edition of this graph produced this run"
/// is answerable from `workflow_runs`' timestamps plus this history.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowTemplateRecord {
    pub name: String,
    pub description: String,
    pub version: u32,
    pub is_built_in: bool,
    pub definition: WorkflowTemplate,
    pub created_at_millis: u64,
    pub updated_at_millis: u64,
}

fn row_to_template(
    name: String,
    description: String,
    version: u32,
    is_built_in: bool,
    definition_json: String,
    created_at_millis: u64,
    updated_at_millis: u64,
) -> Result<WorkflowTemplateRecord> {
    let definition: WorkflowTemplate = serde_json::from_str(&definition_json)?;
    Ok(WorkflowTemplateRecord {
        name,
        description,
        version,
        is_built_in,
        definition,
        created_at_millis,
        updated_at_millis,
    })
}

impl Database {
    /// Insert or update one template. Built-in protection is the caller's
    /// discipline at the GUI boundary AND enforced here: a row that already
    /// exists as a built-in can only be written with `is_built_in` still
    /// true (the startup sync), never demoted or shadowed by a custom
    /// template of the same name.
    pub async fn upsert_workflow_template(&self, record: &WorkflowTemplateRecord) -> Result<()> {
        let conn = self.connection();
        let name = record.name.clone();
        let description = record.description.clone();
        let version = record.version as i64;
        let is_built_in = record.is_built_in as i64;
        let definition_json = serde_json::to_string(&record.definition)?;
        let created_at_millis = record.created_at_millis as i64;
        let updated_at_millis = record.updated_at_millis as i64;
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            let existing_built_in: Option<i64> = conn
                .query_row(
                    "SELECT is_built_in FROM workflow_templates WHERE name = ?1",
                    [&name],
                    |row| row.get(0),
                )
                .optional()?;
            if existing_built_in == Some(1) && is_built_in == 0 {
                return Err(StorageError::BuiltinTemplateProtected(name));
            }
            conn.execute(
                "INSERT INTO workflow_templates
                    (name, description, version, is_built_in, definition_json, created_at_millis, updated_at_millis)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(name) DO UPDATE SET
                    description = excluded.description,
                    version = excluded.version,
                    is_built_in = excluded.is_built_in,
                    definition_json = excluded.definition_json,
                    updated_at_millis = excluded.updated_at_millis",
                rusqlite::params![
                    name,
                    description,
                    version,
                    is_built_in,
                    definition_json,
                    created_at_millis,
                    updated_at_millis
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn list_workflow_templates(&self) -> Result<Vec<WorkflowTemplateRecord>> {
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<Vec<WorkflowTemplateRecord>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare(
                "SELECT name, description, version, is_built_in, definition_json,
                        created_at_millis, updated_at_millis
                 FROM workflow_templates
                 ORDER BY is_built_in DESC, name",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter()
                .map(
                    |(
                        name,
                        description,
                        version,
                        is_built_in,
                        definition_json,
                        created,
                        updated,
                    )| {
                        row_to_template(
                            name,
                            description,
                            version.max(0) as u32,
                            is_built_in != 0,
                            definition_json,
                            created.max(0) as u64,
                            updated.max(0) as u64,
                        )
                    },
                )
                .collect()
        })
        .await?
    }

    pub async fn get_workflow_template(
        &self,
        name: &str,
    ) -> Result<Option<WorkflowTemplateRecord>> {
        let conn = self.connection();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<WorkflowTemplateRecord>> {
            let conn = lock(&conn);
            let row = conn
                .query_row(
                    "SELECT name, description, version, is_built_in, definition_json,
                            created_at_millis, updated_at_millis
                     FROM workflow_templates WHERE name = ?1",
                    [&name],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, i64>(5)?,
                            row.get::<_, i64>(6)?,
                        ))
                    },
                )
                .optional()?;
            row.map(
                |(name, description, version, is_built_in, definition_json, created, updated)| {
                    row_to_template(
                        name,
                        description,
                        version.max(0) as u32,
                        is_built_in != 0,
                        definition_json,
                        created.max(0) as u64,
                        updated.max(0) as u64,
                    )
                },
            )
            .transpose()
        })
        .await?
    }

    /// Delete a custom template. Built-ins refuse: they are the product's
    /// presets, and the catalog must stay honest about what ships with NACC.
    pub async fn delete_workflow_template(&self, name: &str) -> Result<bool> {
        let conn = self.connection();
        let name = name.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let conn = lock(&conn);
            let built_in: Option<i64> = conn
                .query_row(
                    "SELECT is_built_in FROM workflow_templates WHERE name = ?1",
                    [&name],
                    |row| row.get(0),
                )
                .optional()?;
            if built_in == Some(1) {
                return Err(StorageError::BuiltinTemplateProtected(name));
            }
            let deleted =
                conn.execute("DELETE FROM workflow_templates WHERE name = ?1", [&name])?;
            Ok(deleted > 0)
        })
        .await?
    }

    /// The next version number for a template: one past its current stored
    /// version, or 1 for a new name. Callers use this so a content change
    /// always advances the version instead of silently overwriting.
    pub async fn next_workflow_template_version(&self, name: &str) -> Result<u32> {
        Ok(self
            .get_workflow_template(name)
            .await?
            .map(|record| record.version + 1)
            .unwrap_or(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nacc_domain::{PermissionProfile, RoleKind, WorkflowNode};

    fn node(key: &str) -> WorkflowNode {
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

    fn record(name: &str, is_built_in: bool) -> WorkflowTemplateRecord {
        WorkflowTemplateRecord {
            name: name.to_string(),
            description: format!("{name} description"),
            version: 1,
            is_built_in,
            definition: WorkflowTemplate {
                name: name.to_string(),
                description: format!("{name} description"),
                nodes: vec![node("explore"), {
                    let mut implement = node("implement");
                    implement.depends_on = vec!["explore".to_string()];
                    implement
                }],
            },
            created_at_millis: 1_000,
            updated_at_millis: 1_000,
        }
    }

    #[tokio::test]
    async fn templates_roundtrip_with_their_whole_definitions() {
        let db = Database::open_in_memory().unwrap();
        db.upsert_workflow_template(&record("audit_flow", false))
            .await
            .unwrap();

        let listed = db.list_workflow_templates().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "audit_flow");
        assert!(!listed[0].is_built_in);
        assert_eq!(listed[0].definition.nodes.len(), 2);
        assert_eq!(listed[0].definition.nodes[1].key, "implement");
        assert_eq!(listed[0].definition.nodes[1].depends_on, vec!["explore"]);

        let fetched = db
            .get_workflow_template("audit_flow")
            .await
            .unwrap()
            .expect("the row was just written");
        assert_eq!(fetched, listed[0]);
        assert!(db.get_workflow_template("no-such").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_built_in_row_cannot_be_shadowed_or_deleted_from_the_gui_side() {
        let db = Database::open_in_memory().unwrap();
        db.upsert_workflow_template(&record("fast_bug_fix", true))
            .await
            .unwrap();

        let mut custom = record("fast_bug_fix", false);
        custom.version = 9;
        let err = db
            .upsert_workflow_template(&custom)
            .await
            .expect_err("shadowing a built-in must refuse");
        assert!(matches!(err, StorageError::BuiltinTemplateProtected(_)));

        let err = db
            .delete_workflow_template("fast_bug_fix")
            .await
            .expect_err("deleting a built-in must refuse");
        assert!(matches!(err, StorageError::BuiltinTemplateProtected(_)));
        assert!(db
            .get_workflow_template("fast_bug_fix")
            .await
            .unwrap()
            .is_some());

        // A custom template of a different name deletes normally, and a
        // missing name reports `false` rather than erroring.
        db.upsert_workflow_template(&record("my_flow", false))
            .await
            .unwrap();
        assert!(db.delete_workflow_template("my_flow").await.unwrap());
        assert!(!db.delete_workflow_template("my_flow").await.unwrap());
    }

    #[tokio::test]
    async fn versions_start_at_one_and_advance_per_name() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.next_workflow_template_version("fresh").await.unwrap(), 1);

        let mut first = record("fresh", false);
        first.version = db.next_workflow_template_version("fresh").await.unwrap();
        db.upsert_workflow_template(&first).await.unwrap();
        assert_eq!(db.next_workflow_template_version("fresh").await.unwrap(), 2);
    }
}
