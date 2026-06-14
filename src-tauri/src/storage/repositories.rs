use super::rows::*;
use super::*;

impl RepositoryStore {
    pub fn store_repository(
        &mut self,
        url: &str,
        repo: &RepositorySchema,
    ) -> Result<RepositorySummary, String> {
        let tx = self.conn.transaction().map_err(|error| error.to_string())?;
        let now = Utc::now().to_rfc3339();
        let raw_json = serde_json::to_string(repo).map_err(|error| error.to_string())?;
        let content_hash = repo
            .metadata
            .content_hash
            .clone()
            .unwrap_or_else(|| hex::encode(Sha256::digest(raw_json.as_bytes())));
        let trust_level = repo
            .metadata
            .trust_level
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("unknown");

        tx.execute(
            "DELETE FROM repository_assets WHERE repository_id = ?1",
            params![repo.metadata.id],
        )
        .map_err(|error| error.to_string())?;
        tx.execute(
            "DELETE FROM catalog_games WHERE repository_id = ?1",
            params![repo.metadata.id],
        )
        .map_err(|error| error.to_string())?;

        tx.execute(
            r#"
            INSERT INTO repositories (
              id, url, name, version, schema_version, maintainer, homepage_url, license,
              trust_level, content_hash, last_refreshed_at, updated_at, connected_at, raw_json
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            ON CONFLICT(id) DO UPDATE SET
              url = excluded.url,
              name = excluded.name,
              version = excluded.version,
              schema_version = excluded.schema_version,
              maintainer = excluded.maintainer,
              homepage_url = excluded.homepage_url,
              license = excluded.license,
              trust_level = excluded.trust_level,
              content_hash = excluded.content_hash,
              last_refreshed_at = excluded.last_refreshed_at,
              updated_at = excluded.updated_at,
              connected_at = excluded.connected_at,
              raw_json = excluded.raw_json
            "#,
            params![
                repo.metadata.id,
                url,
                repo.metadata.name,
                repo.metadata.version,
                repo.metadata.schema_version,
                repo.metadata.maintainer,
                repo.metadata.homepage_url,
                repo.metadata.license,
                trust_level,
                content_hash,
                now,
                repo.metadata.updated_at,
                now,
                raw_json
            ],
        )
        .map_err(|error| error.to_string())?;

        for asset in &repo.system_files {
            let storage_id = global_id(&repo.metadata.id, &asset.id);
            let asset_kind = serde_json::to_string(&asset.asset_kind)
                .map_err(|error| error.to_string())?
                .trim_matches('"')
                .to_string();
            tx.execute(
                r#"
                INSERT INTO repository_assets (
                  id, source_id, repository_id, platform, asset_kind, display_name,
                  sources_json, install_hint_json, executable
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                "#,
                params![
                    storage_id,
                    asset.id,
                    repo.metadata.id,
                    asset.platform,
                    asset_kind,
                    asset.display_name,
                    serde_json::to_string(&asset.sources).map_err(|error| error.to_string())?,
                    serde_json::to_string(&asset.install_hint).map_err(|error| error.to_string())?,
                    if asset.executable { 1_i64 } else { 0_i64 }
                ],
            )
            .map_err(|error| error.to_string())?;
        }

        for game in &repo.catalog {
            let storage_id = global_id(&repo.metadata.id, &game.id);
            let required = game
                .required_system_file_ids
                .iter()
                .map(|asset_id| global_id(&repo.metadata.id, asset_id))
                .collect::<Vec<_>>();
            let expected_extensions = if game.expected_extensions.is_empty() {
                game.setup_profile_id
                    .as_deref()
                    .and_then(setup_profiles::get_platform_setup_profile)
                    .map(|profile| profile.game_files.expected_extensions)
                    .unwrap_or_default()
            } else {
                game.expected_extensions.clone()
            };
            tx.execute(
                r#"
                INSERT INTO catalog_games (
                  id, source_id, repository_id, platform, title, description, cover_image_url,
                  trailer_url, artwork_json, metadata_json, content_mode, setup_profile_id,
                  downloads_json, expected_extensions_json, required_system_file_ids_json,
                  launch_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                "#,
                params![
                    storage_id,
                    game.id,
                    repo.metadata.id,
                    game.platform,
                    game.title,
                    game.description,
                    game.cover_image_url,
                    game.trailer_url,
                    serde_json::to_string(&game.artwork).map_err(|error| error.to_string())?,
                    serde_json::to_string(&game.metadata).map_err(|error| error.to_string())?,
                    game.content_mode.as_deref(),
                    game.setup_profile_id.as_deref(),
                    serde_json::to_string(&game.downloads).map_err(|error| error.to_string())?,
                    serde_json::to_string(&expected_extensions)
                        .map_err(|error| error.to_string())?,
                    serde_json::to_string(&required).map_err(|error| error.to_string())?,
                    serde_json::to_string(&game.launch).map_err(|error| error.to_string())?
                ],
            )
            .map_err(|error| error.to_string())?;
        }

        tx.commit().map_err(|error| error.to_string())?;
        self.get_repository_summary(&repo.metadata.id)?
            .ok_or_else(|| "Repository was stored but could not be read back.".to_string())
    }

    pub fn list_repositories(&self) -> Result<Vec<RepositorySummary>, String> {
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT
              r.id, r.name, r.version, r.url, r.connected_at,
              r.maintainer, r.homepage_url, r.license, r.trust_level, r.content_hash, r.last_refreshed_at,
              COUNT(DISTINCT g.id) AS catalog_count,
              COUNT(DISTINCT a.id) AS system_file_count,
              MAX(CASE WHEN a.executable = 1 THEN 1 ELSE 0 END) AS has_executable_assets
            FROM repositories r
            LEFT JOIN catalog_games g ON g.repository_id = r.id
            LEFT JOIN repository_assets a ON a.repository_id = r.id
            GROUP BY r.id
            ORDER BY r.connected_at DESC
            "#,
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], map_repository_summary_row)
            .map_err(|error| error.to_string())?;

        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    pub fn disconnect_repository(&self, repository_id: &str) -> Result<bool, String> {
        self.conn
            .execute(
                "DELETE FROM asset_installations WHERE asset_id IN (
                    SELECT id FROM repository_assets WHERE repository_id = ?1
                )",
                params![repository_id],
            )
            .map_err(|error| error.to_string())?;
        let changed = self
            .conn
            .execute(
                "DELETE FROM repositories WHERE id = ?1",
                params![repository_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed > 0)
    }

    pub fn delete_game_download_state(&self, game_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM downloads WHERE subject_id = ?1",
                params![game_id],
            )
            .map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "DELETE FROM torrent_downloads WHERE game_id = ?1",
                params![game_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn delete_asset_state(&self, asset_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM downloads WHERE subject_id = ?1",
                params![asset_id],
            )
            .map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "DELETE FROM asset_installations WHERE asset_id = ?1",
                params![asset_id],
            )
            .map_err(|error| error.to_string())?;
        self.conn
            .execute(
                "DELETE FROM trusted_executables WHERE asset_id = ?1",
                params![asset_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn get_repository_url(&self, repository_id: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT url FROM repositories WHERE id = ?1",
                params![repository_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    fn get_repository_summary(
        &self,
        repository_id: &str,
    ) -> Result<Option<RepositorySummary>, String> {
        self.conn
            .query_row(
                r#"
            SELECT
              r.id, r.name, r.version, r.url, r.connected_at,
              r.maintainer, r.homepage_url, r.license, r.trust_level, r.content_hash, r.last_refreshed_at,
              COUNT(DISTINCT g.id) AS catalog_count,
              COUNT(DISTINCT a.id) AS system_file_count,
              MAX(CASE WHEN a.executable = 1 THEN 1 ELSE 0 END) AS has_executable_assets
            FROM repositories r
            LEFT JOIN catalog_games g ON g.repository_id = r.id
            LEFT JOIN repository_assets a ON a.repository_id = r.id
            WHERE r.id = ?1
            GROUP BY r.id
            "#,
                params![repository_id],
                map_repository_summary_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }
}
