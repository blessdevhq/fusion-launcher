use super::rows::*;
use super::*;

impl RepositoryStore {
    pub fn get_catalog(&self) -> Result<Vec<CatalogGameView>, String> {
        let mut games = {
            let mut statement = self
                .conn
                .prepare(
                    r#"
            SELECT
              g.id, g.source_id, g.repository_id, r.name, g.platform, g.title, g.description,
              g.cover_image_url, g.trailer_url, g.artwork_json, g.metadata_json, g.content_mode,
              g.setup_profile_id, g.downloads_json, g.expected_extensions_json,
              g.required_system_file_ids_json, g.launch_json
            FROM catalog_games g
            JOIN repositories r ON r.id = g.repository_id
            ORDER BY r.name, g.title
            "#,
                )
                .map_err(|error| error.to_string())?;

            let rows = statement
                .query_map([], map_game_row)
                .map_err(|error| error.to_string())?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?
        };

        self.enrich_games_batch(&mut games)?;

        Ok(games)
    }

    /// Enriches a whole catalog page with scraped/override metadata using a
    /// fixed number of queries (three table scans + in-memory joins) instead of
    /// the ~5 per-game lookups in [`Self::enrich_game_with_scraped`]. For large
    /// catalogs this turns an O(N) query storm into O(1).
    fn enrich_games_batch(&self, views: &mut [CatalogGameView]) -> Result<(), String> {
        if views.is_empty() {
            return Ok(());
        }

        let crc32_by_game = self.load_all_rom_crc32()?;
        let metadata_by_key = self.load_all_metadata_cache()?;
        let override_by_game = self.load_all_overrides()?;

        for view in views.iter_mut() {
            let scraped = crc32_by_game
                .get(&view.id)
                .and_then(|crc32| {
                    metadata_by_key.get(&("screenscraper".to_string(), crc32.clone()))
                })
                .or_else(|| {
                    metadata_by_key.get(&(
                        "screenscraper".to_string(),
                        metadata_name_key(&view.platform, &view.title),
                    ))
                });
            let steamgriddb =
                metadata_by_key.get(&("steamgriddb".to_string(), steamgriddb_cache_key(&view.id)));
            let override_ = override_by_game
                .get(&view.id)
                .and_then(|payload| payload.as_ref());

            enrich_with_scraped(
                view,
                scraped.map(|entry| &entry.payload),
                steamgriddb.map(|entry| &entry.payload),
                override_,
            );
        }

        Ok(())
    }

    fn load_all_rom_crc32(&self) -> Result<HashMap<String, String>, String> {
        let mut statement = self
            .conn
            .prepare("SELECT game_id, crc32 FROM rom_hashes")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|error| error.to_string())?;

        let mut map = HashMap::new();
        for row in rows {
            let (game_id, crc32) = row.map_err(|error| error.to_string())?;
            if let Some(crc32) = crc32 {
                if !crc32.trim().is_empty() {
                    map.insert(game_id, crc32);
                }
            }
        }
        Ok(map)
    }

    fn load_all_metadata_cache(
        &self,
    ) -> Result<HashMap<(String, String), MetadataCacheEntry>, String> {
        let now = Utc::now().to_rfc3339();
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT rom_key, provider, payload_json, match_kind
            FROM metadata_cache
            WHERE expires_at IS NULL OR expires_at > ?1
            "#,
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![now], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|error| error.to_string())?;

        let mut map = HashMap::new();
        for row in rows {
            let (rom_key, provider, payload_json, match_kind) =
                row.map_err(|error| error.to_string())?;
            let Some(provider) = provider else {
                continue;
            };
            // Best-effort: skip unparseable rows, matching per-game tolerance.
            let Ok(payload) = serde_json::from_str::<ScrapedGamePayload>(&payload_json) else {
                continue;
            };
            map.insert(
                (provider, rom_key),
                MetadataCacheEntry {
                    payload,
                    match_kind: match_kind.unwrap_or_else(|| "hash".to_string()),
                },
            );
        }
        Ok(map)
    }

    fn load_all_overrides(&self) -> Result<HashMap<String, Option<ScrapedGamePayload>>, String> {
        let mut statement = self
            .conn
            .prepare("SELECT game_id, payload_json FROM scrape_overrides")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .map_err(|error| error.to_string())?;

        let mut map = HashMap::new();
        for row in rows {
            let (game_id, payload_json) = row.map_err(|error| error.to_string())?;
            let payload = payload_json
                .as_deref()
                .filter(|json| !json.trim().is_empty() && json.trim() != "null")
                .and_then(|json| serde_json::from_str::<ScrapedGamePayload>(json).ok());
            map.insert(game_id, payload);
        }
        Ok(map)
    }

    pub fn get_game(&self, game_id: &str) -> Result<Option<CatalogGameView>, String> {
        let mut game = self
            .conn
            .query_row(
                r#"
            SELECT
              g.id, g.source_id, g.repository_id, r.name, g.platform, g.title, g.description,
              g.cover_image_url, g.trailer_url, g.artwork_json, g.metadata_json, g.content_mode,
              g.setup_profile_id, g.downloads_json, g.expected_extensions_json,
              g.required_system_file_ids_json, g.launch_json
            FROM catalog_games g
            JOIN repositories r ON r.id = g.repository_id
            WHERE g.id = ?1
            "#,
                params![game_id],
                map_game_row,
            )
            .optional()
            .map_err(|error| error.to_string())?;

        if let Some(game) = game.as_mut() {
            self.enrich_game_with_scraped(game)?;
        }

        Ok(game)
    }

    fn enrich_game_with_scraped(&self, view: &mut CatalogGameView) -> Result<(), String> {
        let scraped = self.get_scraped_payload_for_game(view)?;
        let steamgriddb =
            self.get_metadata_by_key(&steamgriddb_cache_key(&view.id), "steamgriddb")?;
        let override_ = self.get_override(&view.id)?;
        enrich_with_scraped(
            view,
            scraped.as_ref().map(|entry| &entry.payload),
            steamgriddb.as_ref().map(|entry| &entry.payload),
            override_.as_ref().and_then(|entry| entry.payload.as_ref()),
        );
        Ok(())
    }

    fn get_scraped_payload_for_game(
        &self,
        view: &CatalogGameView,
    ) -> Result<Option<MetadataCacheEntry>, String> {
        if let Some(crc32) = self.get_rom_crc32(&view.id)? {
            if let Some(entry) = self.get_metadata_by_key(&crc32, "screenscraper")? {
                return Ok(Some(entry));
            }
        }

        self.get_metadata_by_key(
            &metadata_name_key(&view.platform, &view.title),
            "screenscraper",
        )
    }

    pub fn get_asset(&self, asset_id: &str) -> Result<Option<AssetView>, String> {
        self.conn
            .query_row(
                r#"
            SELECT
              id, source_id, repository_id, platform, asset_kind, display_name,
              sources_json, install_hint_json, executable
            FROM repository_assets
            WHERE id = ?1
            "#,
                params![asset_id],
                map_asset_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn get_assets(&self, asset_ids: &[String]) -> Result<Vec<AssetView>, String> {
        let mut assets = Vec::new();
        for asset_id in asset_ids {
            if let Some(asset) = self.get_asset(asset_id)? {
                assets.push(asset);
            }
        }
        Ok(assets)
    }
}
