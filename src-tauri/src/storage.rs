use std::path::Path;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::schema::{
    AssetInstallation, AssetView, CatalogGameView, DownloadRecord, EmulatorConfig,
    RepositorySchema, RepositorySummary, TorrentDownloadRecord, TrustedExecutable,
};
use crate::security::global_id;

pub struct RepositoryStore {
    conn: Connection,
}

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
              downloads_json TEXT NOT NULL,
              expected_extensions_json TEXT NOT NULL DEFAULT '[]',
              required_system_file_ids_json TEXT NOT NULL,
              FOREIGN KEY (repository_id) REFERENCES repositories(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS downloads (
              subject_id TEXT PRIMARY KEY,
              subject_type TEXT NOT NULL CHECK (subject_type IN ('asset', 'game')),
              status TEXT NOT NULL CHECK (status IN ('ready', 'error')),
              local_path TEXT,
              sha256 TEXT,
              message TEXT,
              updated_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS torrent_downloads (
              game_id TEXT PRIMARY KEY,
              magnet_uri TEXT NOT NULL,
              save_dir TEXT NOT NULL,
              status TEXT NOT NULL CHECK (
                status IN (
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

            CREATE TABLE IF NOT EXISTS asset_installations (
              asset_id TEXT PRIMARY KEY,
              target_path TEXT,
              status TEXT NOT NULL CHECK (status IN ('ready', 'missing', 'corrupt', 'blocked', 'error')),
              sha256 TEXT,
              verified_at TEXT NOT NULL,
              message TEXT
            );

            CREATE TABLE IF NOT EXISTS app_config (
              key TEXT PRIMARY KEY,
              value TEXT NOT NULL,
              updated_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_assets_repository ON repository_assets(repository_id);
            CREATE INDEX IF NOT EXISTS idx_games_repository ON catalog_games(repository_id);
            CREATE INDEX IF NOT EXISTS idx_torrent_downloads_status ON torrent_downloads(status);
            CREATE INDEX IF NOT EXISTS idx_emulator_configs_status ON emulator_configs(status);
            "#,
        )?;

        self.ensure_column(
            "catalog_games",
            "expected_extensions_json",
            "TEXT NOT NULL DEFAULT '[]'",
        )?;
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

        Ok(())
    }

    fn ensure_column(
        &self,
        table_name: &str,
        column_name: &str,
        column_definition: &str,
    ) -> rusqlite::Result<()> {
        let mut statement = self
            .conn
            .prepare(&format!("PRAGMA table_info({table_name})"))?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        if !columns.iter().any(|column| column == column_name) {
            self.conn.execute(
                &format!("ALTER TABLE {table_name} ADD COLUMN {column_name} {column_definition}"),
                [],
            )?;
        }

        Ok(())
    }

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
        ).map_err(|error| error.to_string())?;

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
            tx.execute(
                r#"
                INSERT INTO catalog_games (
                  id, source_id, repository_id, platform, title, description, cover_image_url,
                  trailer_url, downloads_json, expected_extensions_json, required_system_file_ids_json
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
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
                    serde_json::to_string(&game.downloads).map_err(|error| error.to_string())?,
                    serde_json::to_string(&game.expected_extensions)
                        .map_err(|error| error.to_string())?,
                    serde_json::to_string(&required).map_err(|error| error.to_string())?
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

    pub fn get_catalog(&self) -> Result<Vec<CatalogGameView>, String> {
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT
              g.id, g.source_id, g.repository_id, r.name, g.platform, g.title, g.description,
              g.cover_image_url, g.trailer_url, g.downloads_json, g.expected_extensions_json,
              g.required_system_file_ids_json
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
            .map_err(|error| error.to_string())
    }

    pub fn get_game(&self, game_id: &str) -> Result<Option<CatalogGameView>, String> {
        self.conn
            .query_row(
                r#"
            SELECT
              g.id, g.source_id, g.repository_id, r.name, g.platform, g.title, g.description,
              g.cover_image_url, g.trailer_url, g.downloads_json, g.expected_extensions_json,
              g.required_system_file_ids_json
            FROM catalog_games g
            JOIN repositories r ON r.id = g.repository_id
            WHERE g.id = ?1
            "#,
                params![game_id],
                map_game_row,
            )
            .optional()
            .map_err(|error| error.to_string())
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

    pub fn get_download(&self, subject_id: &str) -> Result<Option<DownloadRecord>, String> {
        self.conn
            .query_row(
                r#"
            SELECT subject_id, subject_type, status, local_path, sha256, message, updated_at
            FROM downloads
            WHERE subject_id = ?1
            "#,
                params![subject_id],
                |row| {
                    Ok(DownloadRecord {
                        subject_id: row.get(0)?,
                        subject_type: row.get(1)?,
                        status: row.get(2)?,
                        local_path: row.get(3)?,
                        sha256: row.get(4)?,
                        message: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn list_download_records(&self) -> Result<Vec<DownloadRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT subject_id, subject_type, status, local_path, sha256, message, updated_at
            FROM downloads
            ORDER BY updated_at DESC
            "#,
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], map_download_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    pub fn record_download(
        &self,
        subject_id: &str,
        subject_type: &str,
        local_path: Option<&str>,
        sha256: Option<&str>,
        message: Option<&str>,
    ) -> Result<DownloadRecord, String> {
        let now = Utc::now().to_rfc3339();
        let status = if message.is_some() { "error" } else { "ready" };
        self.conn.execute(
            r#"
            INSERT INTO downloads (subject_id, subject_type, status, local_path, sha256, message, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(subject_id) DO UPDATE SET
              subject_type = excluded.subject_type,
              status = excluded.status,
              local_path = excluded.local_path,
              sha256 = excluded.sha256,
              message = excluded.message,
              updated_at = excluded.updated_at
            "#,
            params![subject_id, subject_type, status, local_path, sha256, message, now],
        ).map_err(|error| error.to_string())?;

        self.get_download(subject_id)?
            .ok_or_else(|| "Download record was not persisted.".to_string())
    }

    pub fn delete_download(&self, subject_id: &str) -> Result<bool, String> {
        let changed = self
            .conn
            .execute("DELETE FROM downloads WHERE subject_id = ?1", params![subject_id])
            .map_err(|error| error.to_string())?;
        Ok(changed > 0)
    }

    pub fn delete_torrent_download(&self, game_id: &str) -> Result<bool, String> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM torrent_downloads WHERE game_id = ?1",
                params![game_id],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed > 0)
    }

    pub fn upsert_torrent_download_start(
        &self,
        game_id: &str,
        magnet_uri: &str,
        save_dir: &str,
    ) -> Result<TorrentDownloadRecord, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            INSERT INTO torrent_downloads (
              game_id, magnet_uri, save_dir, status, progress_percent,
              downloaded_bytes, total_bytes, download_speed_bytes_per_sec,
              upload_speed_bytes_per_sec, peers_count, torrent_id, error_message,
              created_at, updated_at, completed_at
            ) VALUES (?1, ?2, ?3, 'resolving', 0, 0, 0, 0, 0, 0, NULL, NULL, ?4, ?4, NULL)
            ON CONFLICT(game_id) DO UPDATE SET
              magnet_uri = excluded.magnet_uri,
              save_dir = excluded.save_dir,
              status = excluded.status,
              progress_percent = excluded.progress_percent,
              downloaded_bytes = excluded.downloaded_bytes,
              total_bytes = excluded.total_bytes,
              download_speed_bytes_per_sec = excluded.download_speed_bytes_per_sec,
              upload_speed_bytes_per_sec = excluded.upload_speed_bytes_per_sec,
              peers_count = excluded.peers_count,
              torrent_id = excluded.torrent_id,
              error_message = excluded.error_message,
              updated_at = excluded.updated_at,
              completed_at = excluded.completed_at
            "#,
                params![game_id, magnet_uri, save_dir, now],
            )
            .map_err(|error| error.to_string())?;

        self.get_torrent_download(game_id)?
            .ok_or_else(|| "Torrent download record was not persisted.".to_string())
    }

    pub fn get_torrent_download(
        &self,
        game_id: &str,
    ) -> Result<Option<TorrentDownloadRecord>, String> {
        self.conn
            .query_row(
                r#"
            SELECT
              game_id, magnet_uri, save_dir, status, progress_percent,
              downloaded_bytes, total_bytes, download_speed_bytes_per_sec,
              upload_speed_bytes_per_sec, peers_count, torrent_id, error_message,
              created_at, updated_at, completed_at
            FROM torrent_downloads
            WHERE game_id = ?1
            "#,
                params![game_id],
                map_torrent_download_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn list_torrent_downloads(&self) -> Result<Vec<TorrentDownloadRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT
              game_id, magnet_uri, save_dir, status, progress_percent,
              downloaded_bytes, total_bytes, download_speed_bytes_per_sec,
              upload_speed_bytes_per_sec, peers_count, torrent_id, error_message,
              created_at, updated_at, completed_at
            FROM torrent_downloads
            ORDER BY
              CASE status
                WHEN 'resolving' THEN 0
                WHEN 'downloading' THEN 0
                WHEN 'cancelling' THEN 0
                WHEN 'error' THEN 1
                WHEN 'interrupted' THEN 2
                WHEN 'paused' THEN 2
                WHEN 'completed' THEN 3
                WHEN 'cancelled' THEN 4
                ELSE 5
              END,
              updated_at DESC
            "#,
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], map_torrent_download_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    pub fn list_startup_torrent_downloads(&self) -> Result<Vec<TorrentDownloadRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT
              game_id, magnet_uri, save_dir, status, progress_percent,
              downloaded_bytes, total_bytes, download_speed_bytes_per_sec,
              upload_speed_bytes_per_sec, peers_count, torrent_id, error_message,
              created_at, updated_at, completed_at
            FROM torrent_downloads
            WHERE status IN ('resolving', 'downloading', 'interrupted', 'paused', 'cancelling')
            ORDER BY updated_at ASC
            "#,
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], map_torrent_download_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    pub fn update_torrent_handle(
        &self,
        game_id: &str,
        torrent_id: i64,
        status: &str,
    ) -> Result<TorrentDownloadRecord, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            UPDATE torrent_downloads
            SET torrent_id = ?2,
                status = ?3,
                error_message = NULL,
                updated_at = ?4
            WHERE game_id = ?1
            "#,
                params![game_id, torrent_id, status, now],
            )
            .map_err(|error| error.to_string())?;

        self.get_torrent_download(game_id)?
            .ok_or_else(|| format!("Unknown torrent download: {game_id}"))
    }

    pub fn update_torrent_progress(
        &self,
        game_id: &str,
        status: &str,
        progress_percent: f64,
        downloaded_bytes: u64,
        total_bytes: u64,
        download_speed_bytes_per_sec: u64,
        upload_speed_bytes_per_sec: u64,
        peers_count: usize,
    ) -> Result<TorrentDownloadRecord, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            UPDATE torrent_downloads
            SET status = ?2,
                progress_percent = ?3,
                downloaded_bytes = ?4,
                total_bytes = ?5,
                download_speed_bytes_per_sec = ?6,
                upload_speed_bytes_per_sec = ?7,
                peers_count = ?8,
                error_message = NULL,
                updated_at = ?9,
                completed_at = CASE WHEN ?2 = 'completed' THEN ?9 ELSE completed_at END
            WHERE game_id = ?1
            "#,
                params![
                    game_id,
                    status,
                    progress_percent,
                    u64_to_i64(downloaded_bytes),
                    u64_to_i64(total_bytes),
                    u64_to_i64(download_speed_bytes_per_sec),
                    u64_to_i64(upload_speed_bytes_per_sec),
                    peers_count as i64,
                    now
                ],
            )
            .map_err(|error| error.to_string())?;

        self.get_torrent_download(game_id)?
            .ok_or_else(|| format!("Unknown torrent download: {game_id}"))
    }

    pub fn set_torrent_status(
        &self,
        game_id: &str,
        status: &str,
        error_message: Option<&str>,
    ) -> Result<TorrentDownloadRecord, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            UPDATE torrent_downloads
            SET status = ?2,
                error_message = ?3,
                updated_at = ?4,
                completed_at = CASE WHEN ?2 = 'completed' THEN ?4 ELSE completed_at END
            WHERE game_id = ?1
            "#,
                params![game_id, status, error_message, now],
            )
            .map_err(|error| error.to_string())?;

        self.get_torrent_download(game_id)?
            .ok_or_else(|| format!("Unknown torrent download: {game_id}"))
    }

    pub fn mark_torrent_interrupted(&self, game_id: &str) -> Result<TorrentDownloadRecord, String> {
        self.set_torrent_status(game_id, "interrupted", None)
    }

    pub fn mark_torrent_completed(
        &self,
        game_id: &str,
        save_dir: &str,
        downloaded_bytes: u64,
        total_bytes: u64,
    ) -> Result<TorrentDownloadRecord, String> {
        let record = self.update_torrent_progress(
            game_id,
            "completed",
            100.0,
            downloaded_bytes,
            total_bytes,
            0,
            0,
            0,
        )?;
        self.record_download(game_id, "game", Some(save_dir), None, None)?;
        Ok(record)
    }

    pub fn trust_executable(
        &self,
        asset_id: &str,
        local_path: &str,
        sha256: &str,
    ) -> Result<TrustedExecutable, String> {
        let trusted_at = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            INSERT INTO trusted_executables (asset_id, local_path, sha256, trusted_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(asset_id) DO UPDATE SET
              local_path = excluded.local_path,
              sha256 = excluded.sha256,
              trusted_at = excluded.trusted_at
            "#,
                params![asset_id, local_path, sha256, trusted_at],
            )
            .map_err(|error| error.to_string())?;

        self.get_trusted_executable(asset_id)?
            .ok_or_else(|| "Trusted executable was not persisted.".to_string())
    }

    pub fn get_trusted_executable(
        &self,
        asset_id: &str,
    ) -> Result<Option<TrustedExecutable>, String> {
        self.conn
            .query_row(
                r#"
            SELECT asset_id, local_path, sha256, trusted_at
            FROM trusted_executables
            WHERE asset_id = ?1
            "#,
                params![asset_id],
                |row| {
                    Ok(TrustedExecutable {
                        asset_id: row.get(0)?,
                        local_path: row.get(1)?,
                        sha256: row.get(2)?,
                        trusted_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn list_emulator_configs(&self) -> Result<Vec<EmulatorConfig>, String> {
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT platform, exe_path, status, last_validated_at, version, launch_args_template
            FROM emulator_configs
            ORDER BY platform
            "#,
            )
            .map_err(|error| error.to_string())?;

        let rows = statement
            .query_map([], map_emulator_config_row)
            .map_err(|error| error.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())
    }

    pub fn get_emulator_config(&self, platform: &str) -> Result<Option<EmulatorConfig>, String> {
        self.conn
            .query_row(
                r#"
            SELECT platform, exe_path, status, last_validated_at, version, launch_args_template
            FROM emulator_configs
            WHERE platform = ?1
            "#,
                params![platform],
                map_emulator_config_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn upsert_emulator_config(
        &self,
        platform: &str,
        exe_path: Option<&str>,
        status: &str,
        version: Option<&str>,
        launch_args_template: Option<&str>,
    ) -> Result<EmulatorConfig, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            INSERT INTO emulator_configs (
              platform, exe_path, status, last_validated_at, version, launch_args_template
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(platform) DO UPDATE SET
              exe_path = excluded.exe_path,
              status = excluded.status,
              last_validated_at = excluded.last_validated_at,
              version = excluded.version,
              launch_args_template = excluded.launch_args_template
            "#,
                params![
                    platform,
                    exe_path,
                    status,
                    now,
                    version,
                    launch_args_template
                ],
            )
            .map_err(|error| error.to_string())?;

        self.get_emulator_config(platform)?
            .ok_or_else(|| "Emulator config was not persisted.".to_string())
    }

    pub fn delete_emulator_config(&self, platform: &str) -> Result<bool, String> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM emulator_configs WHERE platform = ?1",
                params![platform],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed > 0)
    }

    pub fn get_asset_installation(
        &self,
        asset_id: &str,
    ) -> Result<Option<AssetInstallation>, String> {
        self.conn
            .query_row(
                r#"
            SELECT asset_id, target_path, status, sha256, verified_at, message
            FROM asset_installations
            WHERE asset_id = ?1
            "#,
                params![asset_id],
                map_asset_installation_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn record_asset_installation(
        &self,
        asset_id: &str,
        target_path: Option<&str>,
        status: &str,
        sha256: Option<&str>,
        message: Option<&str>,
    ) -> Result<AssetInstallation, String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                r#"
            INSERT INTO asset_installations (
              asset_id, target_path, status, sha256, verified_at, message
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(asset_id) DO UPDATE SET
              target_path = excluded.target_path,
              status = excluded.status,
              sha256 = excluded.sha256,
              verified_at = excluded.verified_at,
              message = excluded.message
            "#,
                params![asset_id, target_path, status, sha256, now, message],
            )
            .map_err(|error| error.to_string())?;

        self.get_asset_installation(asset_id)?
            .ok_or_else(|| "Asset installation was not persisted.".to_string())
    }

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

fn map_game_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CatalogGameView> {
    let downloads_json: String = row.get(9)?;
    let expected_extensions_json: String = row.get(10)?;
    let required_json: String = row.get(11)?;
    Ok(CatalogGameView {
        id: row.get(0)?,
        source_id: row.get(1)?,
        repository_id: row.get(2)?,
        repository_name: row.get(3)?,
        platform: row.get(4)?,
        title: row.get(5)?,
        description: row.get(6)?,
        cover_image_url: row.get(7)?,
        trailer_url: row.get(8)?,
        downloads: serde_json::from_str(&downloads_json).unwrap_or_default(),
        expected_extensions: serde_json::from_str(&expected_extensions_json).unwrap_or_default(),
        required_system_file_ids: serde_json::from_str(&required_json).unwrap_or_default(),
    })
}

fn map_repository_summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RepositorySummary> {
    Ok(RepositorySummary {
        id: row.get(0)?,
        name: row.get(1)?,
        version: row.get(2)?,
        url: row.get(3)?,
        connected_at: row.get(4)?,
        maintainer: row.get(5)?,
        homepage_url: row.get(6)?,
        license: row.get(7)?,
        trust_level: row.get(8)?,
        content_hash: row.get(9)?,
        last_refreshed_at: row.get(10)?,
        catalog_count: row.get::<_, i64>(11)? as usize,
        system_file_count: row.get::<_, i64>(12)? as usize,
        has_executable_assets: row.get::<_, i64>(13)? == 1,
    })
}

fn map_download_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DownloadRecord> {
    Ok(DownloadRecord {
        subject_id: row.get(0)?,
        subject_type: row.get(1)?,
        status: row.get(2)?,
        local_path: row.get(3)?,
        sha256: row.get(4)?,
        message: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn map_asset_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetView> {
    let asset_kind_json = format!("\"{}\"", row.get::<_, String>(4)?);
    let sources_json: String = row.get(6)?;
    let install_hint_json: Option<String> = row.get(7)?;
    let install_hint = install_hint_json
        .as_deref()
        .filter(|json| !json.trim().is_empty() && json.trim() != "null")
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    Ok(AssetView {
        id: row.get(0)?,
        source_id: row.get(1)?,
        repository_id: row.get(2)?,
        platform: row.get(3)?,
        asset_kind: serde_json::from_str(&asset_kind_json).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        display_name: row.get(5)?,
        sources: serde_json::from_str(&sources_json).unwrap_or_default(),
        install_hint,
        executable: row.get::<_, i64>(8)? == 1,
    })
}

fn map_emulator_config_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<EmulatorConfig> {
    Ok(EmulatorConfig {
        platform: row.get(0)?,
        exe_path: row.get(1)?,
        status: row.get(2)?,
        last_validated_at: row.get(3)?,
        version: row.get(4)?,
        launch_args_template: row.get(5)?,
    })
}

fn map_asset_installation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AssetInstallation> {
    Ok(AssetInstallation {
        asset_id: row.get(0)?,
        target_path: row.get(1)?,
        status: row.get(2)?,
        sha256: row.get(3)?,
        verified_at: row.get(4)?,
        message: row.get(5)?,
    })
}

fn map_torrent_download_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TorrentDownloadRecord> {
    Ok(TorrentDownloadRecord {
        game_id: row.get(0)?,
        magnet_uri: row.get(1)?,
        save_dir: row.get(2)?,
        status: row.get(3)?,
        progress_percent: row.get(4)?,
        downloaded_bytes: i64_to_u64(row.get(5)?),
        total_bytes: i64_to_u64(row.get(6)?),
        download_speed_bytes_per_sec: i64_to_u64(row.get(7)?),
        upload_speed_bytes_per_sec: i64_to_u64(row.get(8)?),
        peers_count: i64_to_usize(row.get(9)?),
        torrent_id: row.get(10)?,
        error_message: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        completed_at: row.get(14)?,
    })
}

fn u64_to_i64(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn i64_to_u64(value: i64) -> u64 {
    value.max(0) as u64
}

fn i64_to_usize(value: i64) -> usize {
    value.max(0) as usize
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::schema::{
        AssetKind, RepositoryAsset, RepositoryGame, RepositoryMetadata, SourceUri,
    };

    fn test_repo() -> RepositorySchema {
        RepositorySchema {
            metadata: RepositoryMetadata {
                id: "repo".to_string(),
                name: "Repo".to_string(),
                version: "1".to_string(),
                schema_version: 2,
                maintainer: None,
                homepage_url: None,
                license: None,
                trust_level: None,
                content_hash: None,
                updated_at: None,
            },
            system_files: vec![RepositoryAsset {
                id: "emu".to_string(),
                platform: "nes".to_string(),
                asset_kind: AssetKind::Emulator,
                display_name: "Emulator".to_string(),
                sources: vec![SourceUri::Http {
                    url: "https://example.com/emulator.zip".to_string(),
                    sha256: "a".repeat(64),
                    size_bytes: None,
                }],
                install_hint: None,
                executable: true,
            }],
            catalog: vec![RepositoryGame {
                id: "game".to_string(),
                platform: "nes".to_string(),
                title: "Game".to_string(),
                description: None,
                cover_image_url: None,
                trailer_url: None,
                downloads: vec![SourceUri::Magnet {
                    uri: "magnet:?xt=urn:btih:abc".to_string(),
                    info_hash: None,
                    size_bytes: None,
                }],
                expected_extensions: vec![".nes".to_string()],
                required_system_file_ids: vec!["emu".to_string()],
            }],
        }
    }

    #[test]
    fn stores_and_reads_repository_catalog() {
        let dir = tempdir().unwrap();
        let mut store = RepositoryStore::open(&dir.path().join("retrohydra.db")).unwrap();
        let summary = store
            .store_repository("https://example.com/index.json", &test_repo())
            .unwrap();

        assert_eq!(summary.catalog_count, 1);
        assert_eq!(summary.system_file_count, 1);
        assert_eq!(store.list_repositories().unwrap().len(), 1);
        assert_eq!(
            store.get_catalog().unwrap()[0].required_system_file_ids[0],
            "repo::emu"
        );
        assert_eq!(
            store.get_catalog().unwrap()[0].expected_extensions,
            vec![".nes".to_string()]
        );
    }

    #[test]
    fn stores_and_updates_emulator_configs() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("retrohydra.db")).unwrap();

        let saved = store
            .upsert_emulator_config(
                "nes",
                Some("C:/Emulators/nes.exe"),
                "valid",
                Some("1.0.0"),
                Some("--fullscreen {game_path}"),
            )
            .unwrap();

        assert_eq!(saved.platform, "nes");
        assert_eq!(saved.status, "valid");
        assert_eq!(saved.exe_path.as_deref(), Some("C:/Emulators/nes.exe"));
        assert_eq!(store.list_emulator_configs().unwrap().len(), 1);

        let updated = store
            .upsert_emulator_config("nes", Some("C:/Missing/nes.exe"), "missing", None, None)
            .unwrap();
        assert_eq!(updated.status, "missing");
        assert_eq!(updated.version, None);

        assert!(store.delete_emulator_config("nes").unwrap());
        assert!(store.get_emulator_config("nes").unwrap().is_none());
    }

    #[test]
    fn records_asset_installations() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("retrohydra.db")).unwrap();

        let saved = store
            .record_asset_installation(
                "repo::bios",
                Some("F:/System/bios.bin"),
                "ready",
                Some(&"c".repeat(64)),
                None,
            )
            .unwrap();

        assert_eq!(saved.status, "ready");
        assert_eq!(saved.target_path.as_deref(), Some("F:/System/bios.bin"));

        let updated = store
            .record_asset_installation(
                "repo::bios",
                Some("F:/System/bios.bin"),
                "corrupt",
                Some(&"d".repeat(64)),
                Some("SHA-256 mismatch"),
            )
            .unwrap();

        assert_eq!(updated.status, "corrupt");
        assert_eq!(updated.message.as_deref(), Some("SHA-256 mismatch"));
    }

    #[test]
    fn stores_updates_and_completes_torrent_downloads() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("retrohydra.db")).unwrap();

        let started = store
            .upsert_torrent_download_start(
                "repo::game",
                "magnet:?xt=urn:btih:abc",
                "F:/Games/repo-game",
            )
            .unwrap();

        assert_eq!(started.status, "resolving");
        assert_eq!(started.progress_percent, 0.0);
        assert_eq!(started.torrent_id, None);

        let downloading = store
            .update_torrent_handle("repo::game", 7, "downloading")
            .unwrap();
        assert_eq!(downloading.torrent_id, Some(7));
        assert_eq!(downloading.status, "downloading");

        let progressed = store
            .update_torrent_progress("repo::game", "downloading", 42.5, 425, 1000, 12, 3, 4)
            .unwrap();
        assert_eq!(progressed.progress_percent, 42.5);
        assert_eq!(progressed.downloaded_bytes, 425);
        assert_eq!(progressed.total_bytes, 1000);
        assert_eq!(progressed.peers_count, 4);

        let completed = store
            .mark_torrent_completed("repo::game", "F:/Games/repo-game", 1000, 1000)
            .unwrap();
        assert_eq!(completed.status, "completed");
        assert_eq!(completed.progress_percent, 100.0);
        assert!(completed.completed_at.is_some());

        let legacy = store.get_download("repo::game").unwrap().unwrap();
        assert_eq!(legacy.subject_type, "game");
        assert_eq!(legacy.status, "ready");
        assert_eq!(legacy.local_path.as_deref(), Some("F:/Games/repo-game"));
    }

    #[test]
    fn lists_startup_torrent_downloads() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("retrohydra.db")).unwrap();

        store
            .upsert_torrent_download_start("active", "magnet:?xt=urn:btih:abc", "F:/Games/a")
            .unwrap();
        store
            .upsert_torrent_download_start("paused", "magnet:?xt=urn:btih:def", "F:/Games/p")
            .unwrap();
        store.set_torrent_status("paused", "paused", None).unwrap();
        store
            .upsert_torrent_download_start("cancelled", "magnet:?xt=urn:btih:ghi", "F:/Games/c")
            .unwrap();
        store
            .set_torrent_status("cancelled", "cancelled", None)
            .unwrap();

        let game_ids = store
            .list_startup_torrent_downloads()
            .unwrap()
            .into_iter()
            .map(|record| record.game_id)
            .collect::<Vec<_>>();

        assert!(game_ids.contains(&"active".to_string()));
        assert!(game_ids.contains(&"paused".to_string()));
        assert!(!game_ids.contains(&"cancelled".to_string()));
    }

    #[test]
    fn lists_torrent_downloads_with_actionable_records_first() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("retrohydra.db")).unwrap();

        store
            .upsert_torrent_download_start("completed", "magnet:?xt=urn:btih:aaa", "F:/Games/c")
            .unwrap();
        store
            .mark_torrent_completed("completed", "F:/Games/c", 100, 100)
            .unwrap();
        store
            .upsert_torrent_download_start("cancelled", "magnet:?xt=urn:btih:bbb", "F:/Games/x")
            .unwrap();
        store
            .set_torrent_status("cancelled", "cancelled", None)
            .unwrap();
        store
            .upsert_torrent_download_start("paused", "magnet:?xt=urn:btih:ccc", "F:/Games/p")
            .unwrap();
        store.set_torrent_status("paused", "paused", None).unwrap();
        store
            .upsert_torrent_download_start("error", "magnet:?xt=urn:btih:ddd", "F:/Games/e")
            .unwrap();
        store
            .set_torrent_status("error", "error", Some("No peers found"))
            .unwrap();
        store
            .upsert_torrent_download_start("active", "magnet:?xt=urn:btih:eee", "F:/Games/a")
            .unwrap();

        let game_ids = store
            .list_torrent_downloads()
            .unwrap()
            .into_iter()
            .map(|record| record.game_id)
            .collect::<Vec<_>>();

        let position = |game_id: &str| game_ids.iter().position(|id| id == game_id).unwrap();
        assert!(position("active") < position("error"));
        assert!(position("error") < position("paused"));
        assert!(position("paused") < position("completed"));
        assert!(position("completed") < position("cancelled"));
    }
}
