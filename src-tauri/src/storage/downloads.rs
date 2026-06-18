use super::rows::*;
use super::*;

impl RepositoryStore {
    pub fn get_download(&self, subject_id: &str) -> Result<Option<DownloadRecord>, String> {
        self.conn
            .query_row(
                r#"
            SELECT subject_id, subject_type, status, local_path, sha256, message, source, magnet_uri, updated_at
            FROM downloads
            WHERE subject_id = ?1
            "#,
                params![subject_id],
                map_download_row,
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    pub fn list_download_records(&self) -> Result<Vec<DownloadRecord>, String> {
        let mut statement = self
            .conn
            .prepare(
                r#"
            SELECT subject_id, subject_type, status, local_path, sha256, message, source, magnet_uri, updated_at
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
        let status = if message.is_some() { "error" } else { "ready" };
        self.upsert_download(
            subject_id,
            subject_type,
            status,
            local_path,
            sha256,
            message,
            "legacy",
            "",
        )
    }

    pub fn record_imported_asset_download(
        &self,
        asset_id: &str,
        local_path: &str,
        sha256: Option<&str>,
    ) -> Result<DownloadRecord, String> {
        self.upsert_download(
            asset_id,
            "asset",
            "completed",
            Some(local_path),
            sha256,
            None,
            "user_import",
            "",
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn upsert_download(
        &self,
        subject_id: &str,
        subject_type: &str,
        status: &str,
        local_path: Option<&str>,
        sha256: Option<&str>,
        message: Option<&str>,
        source: &str,
        magnet_uri: &str,
    ) -> Result<DownloadRecord, String> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            r#"
            INSERT INTO downloads (
              subject_id, subject_type, status, local_path, sha256, message, source, magnet_uri, updated_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(subject_id) DO UPDATE SET
              subject_type = excluded.subject_type,
              status = excluded.status,
              local_path = excluded.local_path,
              sha256 = excluded.sha256,
              message = excluded.message,
              source = excluded.source,
              magnet_uri = excluded.magnet_uri,
              updated_at = excluded.updated_at
            "#,
            params![
                subject_id,
                subject_type,
                status,
                local_path,
                sha256,
                message,
                source,
                magnet_uri,
                now
            ],
        ).map_err(|error| error.to_string())?;

        self.get_download(subject_id)?
            .ok_or_else(|| "Download record was not persisted.".to_string())
    }

    pub fn delete_download(&self, subject_id: &str) -> Result<bool, String> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM downloads WHERE subject_id = ?1",
                params![subject_id],
            )
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
        let mut record = self
            .conn
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
            .map_err(|error| error.to_string())?;
        if let Some(record) = record.as_mut() {
            self.enrich_torrent_download_record(record)?;
        }
        Ok(record)
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
        let mut records = rows
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        for record in &mut records {
            self.enrich_torrent_download_record(record)?;
        }
        Ok(records)
    }

    fn enrich_torrent_download_record(
        &self,
        record: &mut TorrentDownloadRecord,
    ) -> Result<(), String> {
        if let Some(game) = self.get_game(&record.game_id)? {
            record.subject_type = Some("game".to_string());
            record.display_name = Some(game.title);
        } else if let Some(asset) = self.get_asset(&record.game_id)? {
            record.subject_type = Some("asset".to_string());
            record.display_name = Some(asset.display_name);
        }
        Ok(())
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

    #[allow(clippy::too_many_arguments)]
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

    /// Runs `body` inside a single SQLite transaction so multi-statement writes
    /// either fully apply or fully roll back, preventing partial/ghost records
    /// if the process dies mid-write.
    fn in_transaction<T>(&self, body: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|error| error.to_string())?;
        match body() {
            Ok(value) => {
                self.conn
                    .execute_batch("COMMIT")
                    .map_err(|error| error.to_string())?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    pub fn record_direct_game_download_started(
        &self,
        game_id: &str,
        source_kind: &str,
        target_path: &str,
        total_bytes: u64,
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
            ) VALUES (?1, ?2, ?3, 'downloading', 0, 0, ?4, 0, 0, 0, NULL, NULL, ?5, ?5, NULL)
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
                params![
                    game_id,
                    format!("direct:{source_kind}"),
                    target_path,
                    u64_to_i64(total_bytes),
                    now
                ],
            )
            .map_err(|error| error.to_string())?;

        self.get_torrent_download(game_id)?
            .ok_or_else(|| "Direct download record was not persisted.".to_string())
    }

    pub fn record_direct_asset_download_started(
        &self,
        asset_id: &str,
        source_kind: &str,
        target_path: &str,
        total_bytes: u64,
    ) -> Result<TorrentDownloadRecord, String> {
        self.record_direct_game_download_started(
            asset_id,
            &format!("asset:{source_kind}"),
            target_path,
            total_bytes,
        )
    }

    pub fn record_direct_game_download_failed(
        &self,
        game_id: &str,
        source_kind: &str,
        target_path: &str,
        total_bytes: u64,
        error: &str,
    ) -> Result<TorrentDownloadRecord, String> {
        self.in_transaction(|| {
            self.record_download(game_id, "game", None, None, Some(error))?;
            self.record_direct_game_download_started(
                game_id,
                source_kind,
                target_path,
                total_bytes,
            )?;
            self.set_torrent_status(game_id, "error", Some(error))
        })
    }

    pub fn record_direct_asset_download_failed(
        &self,
        asset_id: &str,
        source_kind: &str,
        target_path: &str,
        total_bytes: u64,
        error: &str,
    ) -> Result<TorrentDownloadRecord, String> {
        self.in_transaction(|| {
            self.record_download(asset_id, "asset", None, None, Some(error))?;
            self.record_direct_asset_download_started(
                asset_id,
                source_kind,
                target_path,
                total_bytes,
            )?;
            self.set_torrent_status(asset_id, "error", Some(error))
        })
    }

    pub fn record_direct_game_download_completed(
        &self,
        game_id: &str,
        source_kind: &str,
        local_path: &str,
        sha256: &str,
        total_bytes: u64,
    ) -> Result<(DownloadRecord, TorrentDownloadRecord), String> {
        self.in_transaction(|| {
            let download =
                self.record_download(game_id, "game", Some(local_path), Some(sha256), None)?;
            self.record_direct_game_download_started(
                game_id,
                source_kind,
                local_path,
                total_bytes,
            )?;
            let torrent = self.update_torrent_progress(
                game_id,
                "completed",
                100.0,
                total_bytes,
                total_bytes,
                0,
                0,
                0,
            )?;
            Ok((download, torrent))
        })
    }

    pub fn record_direct_asset_download_completed(
        &self,
        asset_id: &str,
        source_kind: &str,
        local_path: &str,
        sha256: &str,
        total_bytes: u64,
    ) -> Result<(DownloadRecord, TorrentDownloadRecord), String> {
        self.in_transaction(|| {
            let download =
                self.record_download(asset_id, "asset", Some(local_path), Some(sha256), None)?;
            self.record_direct_asset_download_started(
                asset_id,
                source_kind,
                local_path,
                total_bytes,
            )?;
            let torrent = self.update_torrent_progress(
                asset_id,
                "completed",
                100.0,
                total_bytes,
                total_bytes,
                0,
                0,
                0,
            )?;
            Ok((download, torrent))
        })
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
}
