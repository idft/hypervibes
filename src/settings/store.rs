use crate::db::DbPool;
use anyhow::{Context, Result};
use sqlx::query_as;

use super::model::UserSettingsRow;
use uuid::Uuid;

pub async fn get_user_settings(pool: &DbPool, user_id: Uuid) -> Result<Option<UserSettingsRow>> {
    let row = query_as::<_, UserSettingsRow>(
        "SELECT opencode_system_prompt
           FROM user_settings
          WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("failed to load user settings")?;
    Ok(row)
}

pub async fn upsert_user_system_prompt(pool: &DbPool, user_id: Uuid, prompt: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO user_settings (user_id, opencode_system_prompt, updated_at)
         VALUES ($1, $2, now())
         ON CONFLICT (user_id)
         DO UPDATE SET opencode_system_prompt = EXCLUDED.opencode_system_prompt,
                       updated_at = now()",
    )
    .bind(user_id)
    .bind(prompt)
    .execute(pool)
    .await
    .context("failed to upsert user settings")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn get_user_settings_returns_none_for_missing_user() {
        let pool = crate::test_db::pool().await;
        let row = get_user_settings(&pool, Uuid::new_v4())
            .await
            .expect("get missing setting");
        assert!(row.is_none());
    }

    #[tokio::test]
    async fn user_prompts_are_isolated_and_update_independently() {
        let pool = crate::test_db::pool().await;
        let first_user = crate::test_db::test_user_id();
        let second_user = Uuid::new_v4();
        sqlx::query("INSERT INTO users (id, wallet_address) VALUES ($1, $2)")
            .bind(second_user)
            .bind(format!("0x{:040x}", second_user.as_u128()))
            .execute(&pool)
            .await
            .expect("insert second user");

        upsert_user_system_prompt(&pool, first_user, "first")
            .await
            .expect("insert setting");
        upsert_user_system_prompt(&pool, second_user, "other")
            .await
            .expect("insert second setting");

        upsert_user_system_prompt(&pool, first_user, "second")
            .await
            .expect("update setting");
        let first = get_user_settings(&pool, first_user)
            .await
            .expect("get setting")
            .expect("row present");
        let second = get_user_settings(&pool, second_user)
            .await
            .expect("get second setting")
            .expect("second row present");
        assert_eq!(first.opencode_system_prompt, "second");
        assert_eq!(second.opencode_system_prompt, "other");
    }
}
