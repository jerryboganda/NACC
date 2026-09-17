//! Role Matrix persistence (master plan S4.4's "role profiles" data
//! group). Each row is one [`nacc_domain::RoleProfile`]: the four
//! independently settable switches (role / model / thinking / reasoning
//! effort) plus a permission profile -- see that type's own doc comment
//! for the binding Phase 0 constraint this repository must never violate
//! (a role must stay assignable to any provider, including no provider at
//! all, at any time).

use rusqlite::{params, OptionalExtension, Row};

use nacc_domain::{
    ModelId, NodeFallback, PermissionProfile, ProviderId, ReasoningLevel, RoleKind, RoleProfile,
    RoleProfileId, RoleProfileUpdate, ThinkingMode,
};

use crate::{lock, now_millis, Database, Result, StorageError};

/// Column values read out of a `role_profiles` row before any JSON/UUID
/// parsing happens. Kept separate from `RoleProfile` itself so the
/// `rusqlite::Row` closure (which must return `rusqlite::Result<T>`) never
/// needs to smuggle a `StorageError` through a `rusqlite::Error` -- parsing
/// that can fail happens afterward, outside the row closure, and reports
/// real `StorageError`s directly.
struct RawRoleProfileRow {
    id: String,
    name: String,
    role_kind_json: String,
    provider_id_json: Option<String>,
    model_id: Option<String>,
    thinking_mode_json: String,
    reasoning_level_json: String,
    permission_profile_json: String,
    account_label: Option<String>,
    fallbacks_json: String,
    enabled: bool,
    created_at_millis: i64,
    updated_at_millis: i64,
}

const SELECT_COLUMNS: &str = "id, name, role_kind_json, provider_id_json, model_id, \
     thinking_mode_json, reasoning_level_json, permission_profile_json, \
     account_label, fallbacks_json, enabled, created_at_millis, updated_at_millis";

fn row_to_raw(row: &Row<'_>) -> rusqlite::Result<RawRoleProfileRow> {
    Ok(RawRoleProfileRow {
        id: row.get(0)?,
        name: row.get(1)?,
        role_kind_json: row.get(2)?,
        provider_id_json: row.get(3)?,
        model_id: row.get(4)?,
        thinking_mode_json: row.get(5)?,
        reasoning_level_json: row.get(6)?,
        permission_profile_json: row.get(7)?,
        account_label: row.get(8)?,
        fallbacks_json: row.get(9)?,
        enabled: row.get(10)?,
        created_at_millis: row.get(11)?,
        updated_at_millis: row.get(12)?,
    })
}

fn raw_to_role_profile(raw: RawRoleProfileRow) -> Result<RoleProfile> {
    let id = raw
        .id
        .parse()
        .map_err(|e| StorageError::CorruptStoredValue {
            entity: "RoleProfile.id",
            value: raw.id.clone(),
            detail: format!("{e}"),
        })?;
    let provider_id: Option<ProviderId> = raw
        .provider_id_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()?;
    let model_id = raw.model_id.map(ModelId::from);
    let role_kind: RoleKind = serde_json::from_str(&raw.role_kind_json)?;
    let thinking_mode: ThinkingMode = serde_json::from_str(&raw.thinking_mode_json)?;
    let reasoning_level: ReasoningLevel = serde_json::from_str(&raw.reasoning_level_json)?;
    let permission_profile: PermissionProfile = serde_json::from_str(&raw.permission_profile_json)?;
    let fallbacks: Vec<NodeFallback> = serde_json::from_str(&raw.fallbacks_json)?;

    Ok(RoleProfile {
        id,
        name: raw.name,
        role_kind,
        provider_id,
        model_id,
        thinking_mode,
        reasoning_level,
        permission_profile,
        account_label: raw.account_label,
        fallbacks,
        enabled: raw.enabled,
        created_at_millis: raw.created_at_millis as u64,
        updated_at_millis: raw.updated_at_millis as u64,
    })
}

impl Database {
    #[allow(clippy::too_many_arguments)]
    pub async fn create_role_profile(
        &self,
        name: String,
        role_kind: RoleKind,
        provider_id: Option<ProviderId>,
        model_id: Option<ModelId>,
        thinking_mode: ThinkingMode,
        reasoning_level: ReasoningLevel,
        permission_profile: PermissionProfile,
        account_label: Option<String>,
        fallbacks: Vec<NodeFallback>,
    ) -> Result<RoleProfile> {
        let now = now_millis();
        let profile = RoleProfile {
            id: RoleProfileId::new(),
            name,
            role_kind,
            provider_id,
            model_id,
            thinking_mode,
            reasoning_level,
            permission_profile,
            account_label,
            fallbacks,
            enabled: true,
            created_at_millis: now,
            updated_at_millis: now,
        };

        let conn = self.connection();
        let insert = profile.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                &format!(
                    "INSERT INTO role_profiles ({SELECT_COLUMNS}) VALUES \
                     (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
                ),
                params![
                    insert.id.to_string(),
                    insert.name,
                    serde_json::to_string(&insert.role_kind)?,
                    insert
                        .provider_id
                        .map(|p| serde_json::to_string(&p))
                        .transpose()?,
                    insert.model_id.as_ref().map(|m| m.0.clone()),
                    serde_json::to_string(&insert.thinking_mode)?,
                    serde_json::to_string(&insert.reasoning_level)?,
                    serde_json::to_string(&insert.permission_profile)?,
                    insert.account_label,
                    serde_json::to_string(&insert.fallbacks)?,
                    insert.enabled,
                    insert.created_at_millis as i64,
                    insert.updated_at_millis as i64,
                ],
            )?;
            Ok(())
        })
        .await
        .expect("storage worker thread panicked")?;

        Ok(profile)
    }

    pub async fn update_role_profile(
        &self,
        id: RoleProfileId,
        update: RoleProfileUpdate,
    ) -> Result<RoleProfile> {
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<RoleProfile> {
            let conn = lock(&conn);
            let raw = conn
                .query_row(
                    &format!(
                        "UPDATE role_profiles SET name = ?2, role_kind_json = ?3,
                         provider_id_json = ?4, model_id = ?5, thinking_mode_json = ?6,
                         reasoning_level_json = ?7, permission_profile_json = ?8,
                         account_label = ?9, fallbacks_json = ?10,
                         enabled = ?11, updated_at_millis = ?12
                         WHERE id = ?1 RETURNING {SELECT_COLUMNS}"
                    ),
                    params![
                        id.to_string(),
                        update.name,
                        serde_json::to_string(&update.role_kind)?,
                        update
                            .provider_id
                            .map(|p| serde_json::to_string(&p))
                            .transpose()?,
                        update.model_id.as_ref().map(|m| m.0.as_str()),
                        serde_json::to_string(&update.thinking_mode)?,
                        serde_json::to_string(&update.reasoning_level)?,
                        serde_json::to_string(&update.permission_profile)?,
                        update.account_label,
                        serde_json::to_string(&update.fallbacks)?,
                        update.enabled,
                        now_millis() as i64,
                    ],
                    row_to_raw,
                )
                .optional()?
                .ok_or(StorageError::RoleProfileNotFound(id))?;
            raw_to_role_profile(raw)
        })
        .await
        .expect("storage worker thread panicked")
    }

    pub async fn get_role_profile(&self, id: RoleProfileId) -> Result<Option<RoleProfile>> {
        let conn = self.connection();
        let id_str = id.to_string();
        let raw =
            tokio::task::spawn_blocking(move || -> rusqlite::Result<Option<RawRoleProfileRow>> {
                let conn = lock(&conn);
                conn.query_row(
                    &format!("SELECT {SELECT_COLUMNS} FROM role_profiles WHERE id = ?1"),
                    [id_str],
                    row_to_raw,
                )
                .optional()
            })
            .await
            .expect("storage worker thread panicked")?;

        raw.map(raw_to_role_profile).transpose()
    }

    pub async fn list_role_profiles(&self) -> Result<Vec<RoleProfile>> {
        let conn = self.connection();
        let raws =
            tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<RawRoleProfileRow>> {
                let conn = lock(&conn);
                let mut stmt = conn.prepare(&format!(
                    "SELECT {SELECT_COLUMNS} FROM role_profiles ORDER BY created_at_millis"
                ))?;
                // See the identical comment in audit.rs's list function:
                // `?` in tail position here creates a temporary that
                // would otherwise outlive `conn`/`stmt` per Rust's drop
                // order -- real E0597, caught by this workspace's own CI.
                let rows = stmt.query_map([], row_to_raw)?.collect();
                rows
            })
            .await
            .expect("storage worker thread panicked")?;

        raws.into_iter().map(raw_to_role_profile).collect()
    }

    /// Errors with [`StorageError::RoleProfileNotFound`] if `id` does not
    /// exist -- toggling a role that was already deleted is a real error
    /// to surface, not something to silently no-op.
    pub async fn set_role_profile_enabled(&self, id: RoleProfileId, enabled: bool) -> Result<()> {
        let conn = self.connection();
        let id_str = id.to_string();
        let now = now_millis() as i64;
        let changed = tokio::task::spawn_blocking(move || -> rusqlite::Result<usize> {
            let conn = lock(&conn);
            conn.execute(
                "UPDATE role_profiles SET enabled = ?1, updated_at_millis = ?2 WHERE id = ?3",
                params![enabled, now, id_str],
            )
        })
        .await
        .expect("storage worker thread panicked")?;

        if changed == 0 {
            return Err(StorageError::RoleProfileNotFound(id));
        }
        Ok(())
    }

    /// Returns whether a profile actually existed to delete.
    pub async fn delete_role_profile(&self, id: RoleProfileId) -> Result<bool> {
        let conn = self.connection();
        let id_str = id.to_string();
        let changed = tokio::task::spawn_blocking(move || -> rusqlite::Result<usize> {
            let conn = lock(&conn);
            conn.execute("DELETE FROM role_profiles WHERE id = ?1", [id_str])
        })
        .await
        .expect("storage worker thread panicked")?;
        Ok(changed > 0)
    }
}

#[cfg(test)]
mod tests {
    use nacc_domain::{PermissionProfile, ReasoningLevel, RoleKind, RoleProfileId, ThinkingMode};

    use crate::{Database, StorageError};

    async fn create_test_profile(db: &Database) -> nacc_domain::RoleProfile {
        db.create_role_profile(
            "Primary Security Reviewer".to_string(),
            RoleKind::SecurityReviewer,
            None,
            None,
            ThinkingMode::Auto,
            ReasoningLevel::High,
            PermissionProfile::ReadOnly,
            None,
            vec![],
        )
        .await
        .expect("create_role_profile should succeed against a fresh database")
    }

    #[tokio::test]
    async fn created_profile_round_trips_through_get() {
        let db = Database::open_in_memory().unwrap();
        let created = create_test_profile(&db).await;

        let fetched = db
            .get_role_profile(created.id)
            .await
            .unwrap()
            .expect("just-created profile must be gettable");
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.role_kind, RoleKind::SecurityReviewer);
        assert_eq!(fetched.reasoning_level, ReasoningLevel::High);
        assert!(
            fetched.provider_id.is_none(),
            "unassigned role must stay unassigned"
        );
        assert!(fetched.enabled);
    }

    fn profile_update() -> nacc_domain::RoleProfileUpdate {
        nacc_domain::RoleProfileUpdate {
            name: "Backend implementer".to_string(),
            role_kind: RoleKind::BackendImplementer,
            provider_id: Some(nacc_domain::ProviderId::Codex),
            model_id: Some(nacc_domain::ModelId::from("test-model")),
            thinking_mode: ThinkingMode::Off,
            reasoning_level: ReasoningLevel::Medium,
            permission_profile: PermissionProfile::AutonomousWorktree,
            account_label: Some("work account".to_string()),
            fallbacks: vec![nacc_domain::NodeFallback {
                provider_id: nacc_domain::ProviderId::Claude,
                model_id: None,
                reason: "primary rate limited".to_string(),
            }],
            enabled: false,
        }
    }

    #[tokio::test]
    async fn updating_fields_preserves_identity_and_creation_time() {
        let db = Database::open_in_memory().unwrap();
        let created = create_test_profile(&db).await;
        let untouched = create_test_profile(&db).await;
        let update = profile_update();
        let updated = db
            .update_role_profile(created.id, update.clone())
            .await
            .unwrap();
        let fetched = db.get_role_profile(created.id).await.unwrap().unwrap();

        assert_eq!(updated.id, created.id);
        assert_eq!(fetched.id, created.id);
        assert_eq!(fetched.created_at_millis, created.created_at_millis);
        assert!(fetched.updated_at_millis >= created.updated_at_millis);
        assert_eq!(fetched.name, update.name);
        assert_eq!(fetched.role_kind, update.role_kind);
        assert_eq!(fetched.provider_id, update.provider_id);
        assert_eq!(fetched.model_id, update.model_id);
        assert_eq!(fetched.thinking_mode, update.thinking_mode);
        assert_eq!(fetched.reasoning_level, update.reasoning_level);
        assert_eq!(fetched.permission_profile, update.permission_profile);
        assert!(!fetched.enabled);
        assert_eq!(
            db.get_role_profile(untouched.id)
                .await
                .unwrap()
                .unwrap()
                .name,
            untouched.name
        );
    }

    #[tokio::test]
    async fn updating_can_clear_provider_and_model_assignments() {
        let db = Database::open_in_memory().unwrap();
        let created = create_test_profile(&db).await;
        db.update_role_profile(created.id, profile_update())
            .await
            .unwrap();
        let mut update = profile_update();
        update.provider_id = None;
        update.model_id = None;
        db.update_role_profile(created.id, update).await.unwrap();
        let fetched = db.get_role_profile(created.id).await.unwrap().unwrap();
        assert!(fetched.provider_id.is_none());
        assert!(fetched.model_id.is_none());
    }

    #[tokio::test]
    async fn updating_a_missing_profile_returns_a_typed_error() {
        let db = Database::open_in_memory().unwrap();
        let id = RoleProfileId::new();
        assert!(matches!(
            db.update_role_profile(id, profile_update()).await,
            Err(StorageError::RoleProfileNotFound(missing)) if missing == id
        ));
        assert!(db.list_role_profiles().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn account_label_and_fallback_chain_round_trip() {
        // S11's "account profile" + "fallback chain" row fields: persisted,
        // read back whole, and updatable back to "no account, no fallbacks".
        let db = Database::open_in_memory().unwrap();
        let created = create_test_profile(&db).await;
        assert!(created.account_label.is_none());
        assert!(created.fallbacks.is_empty());

        db.update_role_profile(created.id, profile_update())
            .await
            .unwrap();
        let fetched = db.get_role_profile(created.id).await.unwrap().unwrap();
        assert_eq!(fetched.account_label.as_deref(), Some("work account"));
        assert_eq!(fetched.fallbacks.len(), 1);
        assert_eq!(
            fetched.fallbacks[0].provider_id,
            nacc_domain::ProviderId::Claude
        );
        assert_eq!(fetched.fallbacks[0].reason, "primary rate limited");

        let mut cleared = profile_update();
        cleared.account_label = None;
        cleared.fallbacks = vec![];
        db.update_role_profile(created.id, cleared).await.unwrap();
        let fetched = db.get_role_profile(created.id).await.unwrap().unwrap();
        assert!(fetched.account_label.is_none());
        assert!(fetched.fallbacks.is_empty());
    }

    #[tokio::test]
    async fn unknown_id_reads_as_none_not_an_error() {
        let db = Database::open_in_memory().unwrap();
        let result = db.get_role_profile(RoleProfileId::new()).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_returns_every_created_profile() {
        let db = Database::open_in_memory().unwrap();
        create_test_profile(&db).await;
        create_test_profile(&db).await;
        assert_eq!(db.list_role_profiles().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn disabling_a_profile_persists_and_updates_the_timestamp() {
        let db = Database::open_in_memory().unwrap();
        let created = create_test_profile(&db).await;

        db.set_role_profile_enabled(created.id, false)
            .await
            .unwrap();

        let fetched = db.get_role_profile(created.id).await.unwrap().unwrap();
        assert!(!fetched.enabled);
        assert!(fetched.updated_at_millis >= created.updated_at_millis);
    }

    #[tokio::test]
    async fn toggling_a_nonexistent_profile_is_a_real_error() {
        let db = Database::open_in_memory().unwrap();
        let result = db
            .set_role_profile_enabled(RoleProfileId::new(), true)
            .await;
        assert!(matches!(result, Err(StorageError::RoleProfileNotFound(_))));
    }

    #[tokio::test]
    async fn delete_reports_whether_a_profile_actually_existed() {
        let db = Database::open_in_memory().unwrap();
        let created = create_test_profile(&db).await;

        assert!(db.delete_role_profile(created.id).await.unwrap());
        assert!(!db.delete_role_profile(created.id).await.unwrap());
        assert!(db.get_role_profile(created.id).await.unwrap().is_none());
    }
}
