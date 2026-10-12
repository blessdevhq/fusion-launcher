use super::rows::*;
use super::*;

impl RepositoryStore {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let conn = Connection::open(db_path).map_err(|error| error.to_string())?;
        let store = Self { conn };
        store.initialize().map_err(|error| error.to_string())?;
        Ok(store)
    }

    fn initialize(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            r#"
            PRAGMA foreign_keys = ON;
            PRAGMA busy_timeout = 5000;
            PRAGMA synchronous = NORMAL;
            "#,
        )?;
        let _: String = self
            .conn
            .query_row("PRAGMA journal_mode = WAL;", [], |row| row.get(0))?;
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS repositories (
              id TEXT PRIMARY KEY,
              url TEXT NOT NULL,
              name TEXT NOT NULL,
              version TEXT NOT NULL,
              schema_version INTEGER NOT NULL,
              maintainer TEXT,
              homepage_url TEXT,
              license TEXT,
              trust_level TEXT NOT NULL DEFAULT 'unknown',
              content_hash TEXT,
              last_refreshed_at TEXT,
              updated_at TEXT,
              connected_at TEXT NOT NULL,
              raw_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS repository_assets (
              id TEXT PRIMARY KEY,
              source_id TEXT NOT NULL,
              repository_id TEXT NOT NULL,
              platform TEXT NOT NULL,
              asset_kind TEXT NOT NULL,
              display_name TEXT NOT NULL,
              sources_json TEXT NOT NULL,
              install_hint_json TEXT,
              executable INTEGER NOT NULL DEFAULT 0,
              FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS catalog_games (
              id TEXT PRIMARY KEY,
              source_id TEXT NOT NULL,
              repository_id TEXT NOT NULL,
              platform TEXT NOT NULL,
              title TEXT NOT NULL,
              description TEXT,
              cover_image_url TEXT,
              trailer_url TEXT,
              artwork_json TEXT,
              metadata_json TEXT,
              content_mode TEXT,
              setup_profile_id TEXT,
              downloads_json TEXT NOT NULL,
              expected_extensions_json TEXT NOT NULL DEFAULT '[]',
              required_system_file_ids_json TEXT NOT NULL,
              launch_json TEXT,
              FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS downloads (
              subject_id TEXT PRIMARY KEY,
              subject_type TEXT NOT NULL CHECK (subject_type IN ('asset', 'game')),
              status TEXT NOT NULL CHECK (status IN ('ready', 'completed', 'error')),
              local_path TEXT,
              sha256 TEXT,
              message TEXT,
              source TEXT NOT NULL DEFAULT 'legacy',
              magnet_uri TEXT NOT NULL DEFAULT '',
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS torrent_downloads (
              game_id TEXT PRIMARY KEY,
              magnet_uri TEXT NOT NULL,
              save_dir TEXT NOT NULL,
              status TEXT NOT NULL CHECK (
                status IN (
                  'queued',
                  'resolving',
                  'downloading',
                  'paused',
                  'interrupted',
                  'completed',
                  'cancelling',
                  'cancelled',
                  'error'
                )
              ),
              progress_percent REAL NOT NULL DEFAULT 0,
              downloaded_bytes INTEGER NOT NULL DEFAULT 0,
              total_bytes INTEGER NOT NULL DEFAULT 0,
              download_speed_bytes_per_sec INTEGER NOT NULL DEFAULT 0,
              upload_speed_bytes_per_sec INTEGER NOT NULL DEFAULT 0,
              peers_count INTEGER NOT NULL DEFAULT 0,
              torrent_id INTEGER,
              error_message TEXT,
              created_at TEXT NOT NULL,
              updated_at TEXT NOT NULL,
              completed_at TEXT
            );

            CREATE TABLE IF NOT EXISTS trusted_executables (
              asset_id TEXT PRIMARY KEY,
              local_path TEXT NOT NULL,
              sha256 TEXT NOT NULL,
              trusted_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS emulator_configs (
              platform TEXT PRIMARY KEY,
              exe_path TEXT,
              status TEXT NOT NULL CHECK (status IN ('valid', 'missing', 'invalid')),
              last_validated_at TEXT,
              version TEXT,
              launch_args_template TEXT
            );

            CREATE TABLE IF NOT EXISTS profile_emulator_configs (
              profile_id TEXT PRIMARY KEY,
              platform TEXT NOT NULL,
              exe_path TEXT,
              status TEXT NOT NULL CHECK (status IN ('valid', 'missing', 'invalid')),
              last_validated_at TEXT,
              version TEXT,
              launch_args_template TEXT
            );

            CREATE TABLE IF NOT EXISTS profile_system_file_imports (
              profile_id TEXT NOT NULL,
              requirement_id TEXT NOT NULL,
              target_path TEXT,
              status TEXT NOT NULL CHECK (status IN ('ready', 'missing', 'corrupt', 'error')),
              sha256 TEXT,
              verified_at TEXT NOT NULL,
              message TEXT,
              PRIMARY KEY (profile_id, requirement_id)
            );

            CREATE TABLE IF NOT EXISTS asset_installations (
              asset_id TEXT PRIMARY KEY,
              target_path TEXT,
              status TEXT NOT NULL CHECK (status IN ('ready', 'missing', 'corrupt', 'blocked', 'error')),
              sha256 TEXT,
              verified_at TEXT NOT NULL,
              message TEXT
            );

            CREATE TABLE IF NOT EXISTS rom_hashes (
              game_id TEXT PRIMARY KEY,
              crc32 TEXT,
              md5 TEXT,
              sha1 TEXT,
              sha256 TEXT,
              file_size INTEGER,
              hashed_at TEXT
            );

            CREATE TABLE IF NOT EXISTS metadata_cache (
              rom_key TEXT PRIMARY KEY,
              provider TEXT,
              payload_json TEXT NOT NULL,
              match_kind TEXT,
              fetched_at TEXT,
              expires_at TEXT
            );

            CREATE TABLE IF NOT EXISTS scrape_overrides (
              game_id TEXT PRIMARY KEY,
              provider_game_id TEXT,
              payload_json TEXT,
              locked INTEGER NOT NULL DEFAULT 1,
              updated_at TEXT
            );

            CREATE TABLE IF NOT EXISTS scrape_state (
              game_id TEXT PRIMARY KEY,
              status TEXT CHECK(status IN ('pending','hashing','fetching','ready','ambiguous','failed','skipped')),
              match_kind TEXT,
              candidates_json TEXT,
              message TEXT,
              updated_at TEXT
            );

            CREATE TABLE IF NOT EXISTS app_config (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            -- Links a child download (e.g. an emulator/core bundle pulled in while
            -- installing a game) to the parent game install, so the Downloads view
            -- can group them instead of showing two unrelated rows.
            CREATE TABLE IF NOT EXISTS download_parents (
              child_game_id TEXT PRIMARY KEY,
              parent_game_id TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_assets_repository ON repository_assets(repository_id);
            CREATE INDEX IF NOT EXISTS idx_games_repository ON catalog_games(repository_id);
            CREATE INDEX IF NOT EXISTS idx_torrent_downloads_status ON torrent_downloads(status);
            CREATE INDEX IF NOT EXISTS idx_emulator_configs_status ON emulator_configs(status);
            CREATE INDEX IF NOT EXISTS idx_profile_emulator_configs_status ON profile_emulator_configs(status);
            CREATE INDEX IF NOT EXISTS idx_profile_system_file_imports_status ON profile_system_file_imports(status);
            CREATE INDEX IF NOT EXISTS idx_metadata_cache_provider ON metadata_cache(provider);
            CREATE INDEX IF NOT EXISTS idx_scrape_state_status ON scrape_state(status);
            "#,
        )?;

        self.migrate_downloads_schema()?;
        self.migrate_torrent_downloads_schema()?;
        self.ensure_column(
            "catalog_games",
            "expected_extensions_json",
            "TEXT NOT NULL DEFAULT '[]'",
        )?;
        self.ensure_column("catalog_games", "launch_json", "TEXT")?;
        self.ensure_column("catalog_games", "artwork_json", "TEXT")?;
        self.ensure_column("catalog_games", "metadata_json", "TEXT")?;
        self.ensure_column("catalog_games", "content_mode", "TEXT")?;
        self.ensure_column("catalog_games", "setup_profile_id", "TEXT")?;
        self.ensure_column("repositories", "maintainer", "TEXT")?;
        self.ensure_column("repositories", "homepage_url", "TEXT")?;
        self.ensure_column("repositories", "license", "TEXT")?;
        self.ensure_column(
            "repositories",
            "trust_level",
            "TEXT NOT NULL DEFAULT 'unknown'",
        )?;
        self.ensure_column("repositories", "content_hash", "TEXT")?;
        self.ensure_column("repositories", "last_refreshed_at", "TEXT")?;

        let user_version: i32 = self
            .conn
            .query_row("PRAGMA user_version;", [], |row| row.get(0))?;
        if user_version < 1 {
            self.migrate_v0_to_v1_inner()?;
        }
        if user_version < 2 {
            self.migrate_v1_to_v2_inner()?;
        }

        Ok(())
    }

    fn migrate_v0_to_v1_inner(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> rusqlite::Result<()> {
            self.conn.execute(
                "DELETE FROM profile_emulator_configs WHERE version = 'manifest'",
                [],
            )?;
            self.conn.execute(
                "DELETE FROM emulator_configs WHERE version = 'manifest'",
                [],
            )?;
            self.conn.execute(
                "DELETE FROM app_config WHERE key LIKE 'manifest_emulator:%'",
                [],
            )?;

            let legacy_configs = {
                let mut statement = self.conn.prepare(
                    r#"
                    SELECT platform, exe_path, status, last_validated_at, version, launch_args_template
                    FROM emulator_configs
                    ORDER BY platform
                    "#,
                )?;
                let rows = statement.query_map([], map_emulator_config_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };

            for config in legacy_configs {
                if config.version.as_deref() == Some("manifest") {
                    continue;
                }
                let Some(profile) =
                    setup_profiles::get_default_platform_setup_profile(&config.platform)
                else {
                    continue;
                };
                let exists = self
                    .conn
                    .query_row(
                        "SELECT 1 FROM profile_emulator_configs WHERE profile_id = ?1",
                        params![profile.id],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?;
                if exists.is_some() {
                    continue;
                }
                self.conn.execute(
                    r#"
                    INSERT INTO profile_emulator_configs (
                      profile_id, platform, exe_path, status, last_validated_at, version, launch_args_template
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    "#,
                    params![
                        profile.id,
                        config.platform,
                        config.exe_path,
                        config.status,
                        config.last_validated_at,
                        config.version,
                        config.launch_args_template
                    ],
                )?;
            }

            self.conn.execute_batch("PRAGMA user_version = 1;")?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    fn migrate_v1_to_v2_inner(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> rusqlite::Result<()> {
            let builtin_urls = [
                crate::builtin_demo::BUILTIN_DEMO_REPOSITORY_URL,
                crate::builtin_demo::LEGACY_BUILTIN_DEMO_REPOSITORY_URL,
            ];
            self.conn.execute(
                r#"
                UPDATE repositories
                SET trust_level = 'community'
                WHERE trust_level = 'official'
                  AND url NOT IN (?1, ?2)
                "#,
                params![builtin_urls[0], builtin_urls[1]],
            )?;

            self.conn.execute_batch("PRAGMA user_version = 2;")?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(())
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    pub fn sanitize_manifest_emulator_records(&self) -> Result<usize, String> {
        self.sanitize_manifest_emulator_records_inner()
            .map_err(|error| error.to_string())
    }

    fn sanitize_manifest_emulator_records_inner(&self) -> rusqlite::Result<usize> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let res = (|| -> rusqlite::Result<usize> {
            let count1 = self.conn.execute(
                "DELETE FROM profile_emulator_configs WHERE version = 'manifest'",
                [],
            )?;
            let count2 = self.conn.execute(
                "DELETE FROM emulator_configs WHERE version = 'manifest'",
                [],
            )?;
            let count3 = self.conn.execute(
                "DELETE FROM app_config WHERE key LIKE 'manifest_emulator:%'",
                [],
            )?;
            Ok(count1 + count2 + count3)
        })();
        match res {
            Ok(count) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(count)
            }
            Err(err) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(err)
            }
        }
    }

    pub fn migrate_legacy_emulator_configs(&self) -> Result<usize, String> {
        self.migrate_legacy_emulator_configs_inner()
            .map_err(|error| error.to_string())
    }

    fn migrate_legacy_emulator_configs_inner(&self) -> rusqlite::Result<usize> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> rusqlite::Result<usize> {
            let legacy_configs = {
                let mut statement = self.conn.prepare(
                    r#"
                    SELECT platform, exe_path, status, last_validated_at, version, launch_args_template
                    FROM emulator_configs
                    ORDER BY platform
                    "#,
                )?;
                let rows = statement.query_map([], map_emulator_config_row)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };

            let mut migrated = 0;
            for config in legacy_configs {
                if config.version.as_deref() == Some("manifest") {
                    continue;
                }
                let Some(profile) =
                    setup_profiles::get_default_platform_setup_profile(&config.platform)
                else {
                    continue;
                };
                let exists = self
                    .conn
                    .query_row(
                        "SELECT 1 FROM profile_emulator_configs WHERE profile_id = ?1",
                        params![profile.id],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?;
                if exists.is_some() {
                    continue;
                }
                self.conn.execute(
                    r#"
                    INSERT INTO profile_emulator_configs (
                      profile_id, platform, exe_path, status, last_validated_at, version, launch_args_template
                    ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                    "#,
                    params![
                        profile.id,
                        config.platform,
                        config.exe_path,
                        config.status,
                        config.last_validated_at,
                        config.version,
                        config.launch_args_template
                    ],
                )?;
                migrated += 1;
            }

            Ok(migrated)
        })();

        match result {
            Ok(migrated) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(migrated)
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    fn migrate_downloads_schema(&self) -> rusqlite::Result<()> {
        let create_sql = self
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'downloads'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_default();
        let columns = self.table_columns("downloads")?;
        let has_completed_status = create_sql.contains("'completed'");
        let has_source = columns.iter().any(|column| column == "source");
        let has_magnet_uri = columns.iter().any(|column| column == "magnet_uri");

        if has_completed_status && has_source && has_magnet_uri {
            return Ok(());
        }

        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> rusqlite::Result<()> {
            self.conn.execute_batch(
                r#"
                DROP TABLE IF EXISTS downloads_next;
                CREATE TABLE downloads_next (
                  subject_id TEXT PRIMARY KEY,
                  subject_type TEXT NOT NULL CHECK (subject_type IN ('asset', 'game')),
                  status TEXT NOT NULL CHECK (status IN ('ready', 'completed', 'error')),
                  local_path TEXT,
                  sha256 TEXT,
                  message TEXT,
                  source TEXT NOT NULL DEFAULT 'legacy',
                  magnet_uri TEXT NOT NULL DEFAULT '',
                  updated_at TEXT NOT NULL
                );
                "#,
            )?;

            let source_expr = if has_source { "source" } else { "'legacy'" };
            let magnet_uri_expr = if has_magnet_uri { "magnet_uri" } else { "''" };
            self.conn.execute(
                &format!(
                    r#"
                    INSERT INTO downloads_next (
                      subject_id, subject_type, status, local_path, sha256, message,
                      source, magnet_uri, updated_at
                    )
                    SELECT
                      subject_id, subject_type, status, local_path, sha256, message,
                      {source_expr}, {magnet_uri_expr}, updated_at
                    FROM downloads
                    "#
                ),
                [],
            )?;
            self.conn.execute_batch(
                r#"
                DROP TABLE downloads;
                ALTER TABLE downloads_next RENAME TO downloads;
                "#,
            )?;
            Ok(())
        })();

        if let Err(error) = result {
            let _ = self.conn.execute_batch("ROLLBACK");
            return Err(error);
        }

        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    fn migrate_torrent_downloads_schema(&self) -> rusqlite::Result<()> {
        let create_sql = self
            .conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'torrent_downloads'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_default();

        if create_sql.contains("'queued'") {
            return Ok(());
        }

        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let result = (|| -> rusqlite::Result<()> {
            self.conn.execute_batch(
                r#"
                DROP TABLE IF EXISTS torrent_downloads_next;
                CREATE TABLE torrent_downloads_next (
                  game_id TEXT PRIMARY KEY,
                  magnet_uri TEXT NOT NULL,
                  save_dir TEXT NOT NULL,
                  status TEXT NOT NULL CHECK (
                    status IN (
                      'queued',
                      'resolving',
                      'downloading',
                      'paused',
                      'interrupted',
                      'completed',
                      'cancelling',
                      'cancelled',
                      'error'
                    )
                  ),
                  progress_percent REAL NOT NULL DEFAULT 0,
                  downloaded_bytes INTEGER NOT NULL DEFAULT 0,
                  total_bytes INTEGER NOT NULL DEFAULT 0,
                  download_speed_bytes_per_sec INTEGER NOT NULL DEFAULT 0,
                  upload_speed_bytes_per_sec INTEGER NOT NULL DEFAULT 0,
                  peers_count INTEGER NOT NULL DEFAULT 0,
                  torrent_id INTEGER,
                  error_message TEXT,
                  created_at TEXT NOT NULL,
                  updated_at TEXT NOT NULL,
                  completed_at TEXT
                );
                INSERT INTO torrent_downloads_next (
                  game_id, magnet_uri, save_dir, status, progress_percent,
                  downloaded_bytes, total_bytes, download_speed_bytes_per_sec,
                  upload_speed_bytes_per_sec, peers_count, torrent_id, error_message,
                  created_at, updated_at, completed_at
                )
                SELECT
                  game_id, magnet_uri, save_dir, status, progress_percent,
                  downloaded_bytes, total_bytes, download_speed_bytes_per_sec,
                  upload_speed_bytes_per_sec, peers_count, torrent_id, error_message,
                  created_at, updated_at, completed_at
                FROM torrent_downloads;

                DROP TABLE torrent_downloads;
                ALTER TABLE torrent_downloads_next RENAME TO torrent_downloads;
                CREATE INDEX IF NOT EXISTS idx_torrent_downloads_status ON torrent_downloads(status);
                "#,
            )?;
            Ok(())
        })();

        if let Err(error) = result {
            let _ = self.conn.execute_batch("ROLLBACK");
            return Err(error);
        }

        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    fn ensure_column(
        &self,
        table_name: &str,
        column_name: &str,
        column_definition: &str,
    ) -> rusqlite::Result<()> {
        let columns = self.table_columns(table_name)?;

        if !columns.iter().any(|column| column == column_name) {
            self.conn.execute(
                &format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {column_definition}"),
                [],
            )?;
        }

        Ok(())
    }

    fn table_columns(&self, table_name: &str) -> rusqlite::Result<Vec<String>> {
        let mut statement = self
            .conn
            .prepare(&format!("PRAGMA table_info({table_name})"))?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(columns)
    }
}
