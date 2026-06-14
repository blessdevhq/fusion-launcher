use super::*;

impl RepositoryStore {
    pub fn get_config(&self, key: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT value FROM app_config WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn set_config(&self, key: &str, value: &str) -> Result<String, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            INSERT INTO app_config (key, value, updated_at)
            VALUES (?1, ?2, ?3)
            ON CONFLICT(key) DO UPDATE SET
              value = excluded.value,
              updated_at = excluded.updated_at
            "#,
                params![key, value, now],
            )
            .map_err(|error| error.to_string())?;
        Ok(value.to_string())
    }
}
