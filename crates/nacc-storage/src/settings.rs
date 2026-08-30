//! Application settings: a plain string key-value store (master plan
//! S4.4's "application settings" data group). Deliberately untyped on the
//! Rust side -- unlike role profiles or events, there is no fixed settings
//! schema yet (that is Phase 6's Setup Wizard/Role Matrix GUI work); this
//! is the durable primitive it will be built on.

use rusqlite::OptionalExtension;

use crate::{lock, now_millis, Database, Result};

impl Database {
    pub async fn get_setting(&self, key: &str) -> Result<Option<String>> {
        let conn = self.connection();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<String>> {
            let conn = lock(&conn);
            let value = conn
                .query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    [&key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            Ok(value)
        })
        .await
        .expect("storage worker thread panicked")
    }

    pub async fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        let conn = self.connection();
        let key = key.to_string();
        let value = value.to_string();
        let now = now_millis() as i64;
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = lock(&conn);
            conn.execute(
                "INSERT INTO app_settings (key, value, updated_at_millis) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at_millis = excluded.updated_at_millis",
                rusqlite::params![key, value, now],
            )?;
            Ok(())
        })
        .await
        .expect("storage worker thread panicked")
    }

    pub async fn list_settings(&self) -> Result<Vec<(String, String)>> {
        let conn = self.connection();
        tokio::task::spawn_blocking(move || -> Result<Vec<(String, String)>> {
            let conn = lock(&conn);
            let mut stmt = conn.prepare("SELECT key, value FROM app_settings ORDER BY key")?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
        .expect("storage worker thread panicked")
    }

    /// Returns whether a setting actually existed to delete.
    pub async fn delete_setting(&self, key: &str) -> Result<bool> {
        let conn = self.connection();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool> {
            let conn = lock(&conn);
            let changed = conn.execute("DELETE FROM app_settings WHERE key = ?1", [key])?;
            Ok(changed > 0)
        })
        .await
        .expect("storage worker thread panicked")
    }
}

#[cfg(test)]
mod tests {
    use crate::Database;

    #[tokio::test]
    async fn unset_key_reads_as_none_not_empty_string() {
        let db = Database::open_in_memory().unwrap();
        assert_eq!(db.get_setting("theme").await.unwrap(), None);
    }

    #[tokio::test]
    async fn set_then_get_round_trips_the_value() {
        let db = Database::open_in_memory().unwrap();
        db.set_setting("theme", "dark").await.unwrap();
        assert_eq!(
            db.get_setting("theme").await.unwrap().as_deref(),
            Some("dark")
        );
    }

    #[tokio::test]
    async fn setting_a_key_twice_overwrites_rather_than_duplicating() {
        let db = Database::open_in_memory().unwrap();
        db.set_setting("theme", "dark").await.unwrap();
        db.set_setting("theme", "light").await.unwrap();
        assert_eq!(
            db.get_setting("theme").await.unwrap().as_deref(),
            Some("light")
        );
        assert_eq!(db.list_settings().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_reports_whether_a_key_actually_existed() {
        let db = Database::open_in_memory().unwrap();
        assert!(!db.delete_setting("theme").await.unwrap());
        db.set_setting("theme", "dark").await.unwrap();
        assert!(db.delete_setting("theme").await.unwrap());
        assert_eq!(db.get_setting("theme").await.unwrap(), None);
    }

    #[tokio::test]
    async fn list_settings_is_sorted_by_key() {
        let db = Database::open_in_memory().unwrap();
        db.set_setting("zzz", "1").await.unwrap();
        db.set_setting("aaa", "2").await.unwrap();
        let all = db.list_settings().await.unwrap();
        assert_eq!(
            all,
            vec![
                ("aaa".to_string(), "2".to_string()),
                ("zzz".to_string(), "1".to_string())
            ]
        );
    }
}
