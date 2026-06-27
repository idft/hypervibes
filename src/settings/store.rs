use crate::db::DbPool;
use anyhow::{Context, Result};
use sqlx::query_as;

use super::model::SystemSettingRow;

pub async fn get_setting(pool: &DbPool, key: &str) -> Result<Option<SystemSettingRow>> {
    let row = query_as::<_, SystemSettingRow>(
        "SELECT key, value, description, updated_at
           FROM system_settings
          WHERE key = $1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("failed to load system setting {key}"))?;
    Ok(row)
}

pub async fn upsert_setting(
    pool: &DbPool,
    key: &str,
    value: &str,
    description: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO system_settings (key, value, description, updated_at)
         VALUES ($1, $2, $3, now())
         ON CONFLICT (key)
         DO UPDATE SET value = EXCLUDED.value,
                       description = EXCLUDED.description,
                       updated_at = now()",
    )
    .bind(key)
    .bind(value)
    .bind(description)
    .execute(pool)
    .await
    .with_context(|| format!("failed to upsert system setting {key}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_setting_returns_none_for_missing_key() {
        let pool = crate::test_db::pool().await;
        let row = get_setting(&pool, "does_not_exist")
            .await
            .expect("get missing setting");
        assert!(row.is_none());
    }

    #[tokio::test]
    async fn upsert_setting_inserts_and_updates() {
        let pool = crate::test_db::pool().await;
        let key = "test_setting";

        upsert_setting(&pool, key, "first", Some("desc"))
            .await
            .expect("insert setting");
        let first = get_setting(&pool, key)
            .await
            .expect("get setting")
            .expect("row present");
        assert_eq!(first.value, "first");
        assert_eq!(first.description.as_deref(), Some("desc"));

        upsert_setting(&pool, key, "second", None)
            .await
            .expect("update setting");
        let second = get_setting(&pool, key)
            .await
            .expect("get setting")
            .expect("row present");
        assert_eq!(second.value, "second");
        assert!(second.updated_at >= first.updated_at);
    }
}
