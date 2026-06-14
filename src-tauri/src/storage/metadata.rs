use super::rows::*;
use super::*;

impl RepositoryStore {
    pub fn upsert_rom_hashes(&self, game_id: &str, hashes: &RomHashes) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            INSERT INTO rom_hashes (game_id, crc32, md5, sha1, sha256, file_size, hashed_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(game_id) DO UPDATE SET
              crc32 = excluded.crc32,
              md5 = excluded.md5,
              sha1 = excluded.sha1,
              sha256 = excluded.sha256,
              file_size = excluded.file_size,
              hashed_at = excluded.hashed_at
            "#,
                params![
                    game_id,
                    hashes.crc32,
                    hashes.md5,
                    hashes.sha1,
                    hashes.sha256,
                    u64_to_i64(hashes.size),
                    now
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn upsert_rom_hash_sha256(
        &self,
        game_id: &str,
        sha256: &str,
        file_size: u64,
    ) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            INSERT INTO rom_hashes (game_id, sha256, file_size, hashed_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(game_id) DO UPDATE SET
              sha256 = excluded.sha256,
              file_size = excluded.file_size,
              hashed_at = excluded.hashed_at
            "#,
                params![game_id, sha256, u64_to_i64(file_size), now],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn get_rom_crc32(&self, game_id: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT crc32 FROM rom_hashes WHERE game_id = ?1",
                params![game_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|value| value.flatten().filter(|crc32| !crc32.trim().is_empty()))
            .map_err(|error| error.to_string())
    }

    pub fn get_metadata_by_key(
        &self,
        rom_key: &str,
        provider: &str,
    ) -> Result<Option<MetadataCacheEntry>, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .query_row(
                r#"
            SELECT payload_json, match_kind
            FROM metadata_cache
            WHERE rom_key = ?1
              AND provider = ?2
              AND (expires_at IS NULL OR expires_at > ?3)
            "#,
                params![rom_key, provider, now],
                |row| {
                    let payload_json: String = row.get(0)?;
                    let payload = serde_json::from_str::<ScrapedGamePayload>(&payload_json)
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?;
                    Ok(MetadataCacheEntry {
                        payload,
                        match_kind: row
                            .get::<_, Option<String>>(1)?
                            .unwrap_or_else(|| "hash".to_string()),
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn put_metadata(
        &self,
        rom_key: &str,
        provider: &str,
        payload: &ScrapedGamePayload,
        match_kind: &str,
    ) -> Result<(), String> {
        let now = Utc::now();
        let fetched_at = now.to_rfc3339();
        let expires_at = (now + Duration::days(30)).to_rfc3339();
        let payload_json = serde_json::to_string(payload).map_err(|error| error.to_string())?;
        self.conn
            .execute(
                r#"
            INSERT INTO metadata_cache (rom_key, provider, payload_json, match_kind, fetched_at, expires_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(rom_key) DO UPDATE SET
              provider = excluded.provider,
              payload_json = excluded.payload_json,
              match_kind = excluded.match_kind,
              fetched_at = excluded.fetched_at,
              expires_at = excluded.expires_at
            "#,
                params![rom_key, provider, payload_json, match_kind, fetched_at, expires_at],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn get_override(&self, game_id: &str) -> Result<Option<ScrapeOverrideRecord>, String> {
        self.conn
            .query_row(
                r#"
            SELECT provider_game_id, payload_json, locked, updated_at
            FROM scrape_overrides
            WHERE game_id = ?1
            "#,
                params![game_id],
                |row| {
                    let payload_json: Option<String> = row.get(1)?;
                    let payload = payload_json
                        .as_deref()
                        .filter(|json| !json.trim().is_empty() && json.trim() != "null")
                        .map(serde_json::from_str)
                        .transpose()
                        .map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                1,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?;
                    Ok(ScrapeOverrideRecord {
                        provider_game_id: row.get(0)?,
                        payload,
                        locked: row.get::<_, i64>(2)? == 1,
                        updated_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn put_override(
        &self,
        game_id: &str,
        provider_game_id: &str,
        payload: &ScrapedGamePayload,
    ) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        let payload_json = serde_json::to_string(payload).map_err(|error| error.to_string())?;
        self.conn
            .execute(
                r#"
            INSERT INTO scrape_overrides (game_id, provider_game_id, payload_json, locked, updated_at)
            VALUES (?1, ?2, ?3, 1, ?4)
            ON CONFLICT(game_id) DO UPDATE SET
              provider_game_id = excluded.provider_game_id,
              payload_json = excluded.payload_json,
              locked = 1,
              updated_at = excluded.updated_at
            "#,
                params![game_id, provider_game_id, payload_json, now],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn clear_override(&self, game_id: &str) -> Result<bool, String> {
        self.conn
            .execute(
                "DELETE FROM scrape_overrides WHERE game_id = ?1",
                params![game_id],
            )
            .map(|changed| changed > 0)
            .map_err(|error| error.to_string())
    }

    pub fn get_scrape_state(&self, game_id: &str) -> Result<Option<ScrapeStateView>, String> {
        self.conn
            .query_row(
                r#"
            SELECT game_id, status, match_kind, candidates_json, message, updated_at
            FROM scrape_state
            WHERE game_id = ?1
            "#,
                params![game_id],
                map_scrape_state_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn set_scrape_state(
        &self,
        game_id: &str,
        status: &str,
        match_kind: Option<&str>,
        candidates: &[ScrapeCandidate],
        message: Option<&str>,
    ) -> Result<ScrapeStateView, String> {
        let now = Utc::now().to_rfc3339();
        let candidates_json =
            serde_json::to_string(candidates).map_err(|error| error.to_string())?;
        self.conn
            .execute(
                r#"
            INSERT INTO scrape_state (game_id, status, match_kind, candidates_json, message, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(game_id) DO UPDATE SET
              status = excluded.status,
              match_kind = excluded.match_kind,
              candidates_json = excluded.candidates_json,
              message = excluded.message,
              updated_at = excluded.updated_at
            "#,
                params![game_id, status, match_kind, candidates_json, message, now],
            )
            .map_err(|error| error.to_string())?;

        self.get_scrape_state(game_id)?
            .ok_or_else(|| "Scrape state was not persisted.".to_string())
    }

    pub fn delete_scrape_artifacts(&self, game_id: &str) -> Result<bool, String> {
        let mut changed = false;
        changed = self
            .conn
            .execute(
                "DELETE FROM rom_hashes WHERE game_id = ?1",
                params![game_id],
            )
            .map_err(|error| error.to_string())?
            > 0
            || changed;
        changed = self
            .conn
            .execute(
                "DELETE FROM scrape_state WHERE game_id = ?1",
                params![game_id],
            )
            .map_err(|error| error.to_string())?
            > 0
            || changed;
        changed = self
            .conn
            .execute(
                "DELETE FROM scrape_overrides WHERE game_id = ?1",
                params![game_id],
            )
            .map_err(|error| error.to_string())?
            > 0
            || changed;
        Ok(changed)
    }

    pub fn consume_screenscraper_request(&self, daily_limit: u32) -> Result<u32, String> {
        let today = Utc::now().date_naive().to_string();
        let configured_day = self.get_config("screenscraper.request_day")?;
        let mut count = if configured_day.as_deref() == Some(today.as_str()) {
            self.get_config("screenscraper.request_count")?
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0)
        } else {
            0
        };

        if count >= daily_limit {
            return Err(format!(
                "ScreenScraper daily request limit reached: {count}/{daily_limit}"
            ));
        }

        count += 1;
        self.set_config("screenscraper.request_day", &today)?;
        self.set_config("screenscraper.request_count", &count.to_string())?;
        Ok(count)
    }

    pub fn get_screenscraper_request_count(&self) -> Result<u32, String> {
        let today = Utc::now().date_naive().to_string();
        if self.get_config("screenscraper.request_day")?.as_deref() != Some(today.as_str()) {
            return Ok(0);
        }
        Ok(self
            .get_config("screenscraper.request_count")?
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0))
    }

    pub fn consume_steamgriddb_request(&self, daily_limit: u32) -> Result<u32, String> {
        let today = Utc::now().date_naive().to_string();
        let configured_day = self.get_config("steamgriddb.request_day")?;
        let mut count = if configured_day.as_deref() == Some(today.as_str()) {
            self.get_config("steamgriddb.request_count")?
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(0)
        } else {
            0
        };

        if count >= daily_limit {
            return Err(format!(
                "SteamGridDB daily request limit reached: {count}/{daily_limit}"
            ));
        }

        count += 1;
        self.set_config("steamgriddb.request_day", &today)?;
        self.set_config("steamgriddb.request_count", &count.to_string())?;
        Ok(count)
    }

    pub fn get_steamgriddb_request_count(&self) -> Result<u32, String> {
        let today = Utc::now().date_naive().to_string();
        if self.get_config("steamgriddb.request_day")?.as_deref() != Some(today.as_str()) {
            return Ok(0);
        }
        Ok(self
            .get_config("steamgriddb.request_count")?
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0))
    }

    pub fn mark_installed_games_pending_for_scrape(&self) -> Result<usize, String> {
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT DISTINCT g.id
            FROM catalog_games g
            LEFT JOIN downloads d
              ON d.subject_id = g.id
             AND d.subject_type = 'game'
             AND d.status IN ('ready', 'completed')
             AND d.local_path IS NOT NULL
             AND TRIM(d.local_path) <> ''
            LEFT JOIN torrent_downloads t
              ON t.game_id = g.id
             AND t.status = 'completed'
            WHERE d.subject_id IS NOT NULL OR t.game_id IS NOT NULL
            ORDER BY g.title
            "#,
            )
            .map_err(|error| error.to_string())?;
        let game_ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;

        for game_id in &game_ids {
            self.set_scrape_state(
                game_id,
                "pending",
                None,
                &[],
                Some("Queued for library metadata scrape."),
            )?;
        }

        Ok(game_ids.len())
    }

    pub fn list_pending_scrape_game_ids(&self) -> Result<Vec<String>, String> {
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT s.game_id
            FROM scrape_state s
            JOIN catalog_games g ON g.id = s.game_id
            WHERE s.status = 'pending'
            ORDER BY s.updated_at, g.title
            "#,
            )
            .map_err(|error| error.to_string())?;
        let game_ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        Ok(game_ids)
    }

    pub fn count_pending_scrape_states(&self) -> Result<usize, String> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM scrape_state WHERE status = 'pending'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count.max(0) as usize)
            .map_err(|error| error.to_string())
    }
}
