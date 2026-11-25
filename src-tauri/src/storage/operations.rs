use super::*;

pub(crate) const SUPERSEDED: &str = "download_superseded";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DownloadAttempt {
    pub subject: String,
    pub attempt: String,
    pub run: u64,
}

pub(crate) struct WorkerProgress {
    pub percent: f64,
    pub downloaded: u64,
    pub total: u64,
    pub speed: u64,
    pub upload: u64,
    pub peers: usize,
}

impl RepositoryStore {
    pub(crate) fn reset_runtime_handles(&self) -> Result<(), String> {
        self.conn
            .execute("UPDATE torrent_downloads SET torrent_id = NULL", [])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(crate) fn assign_download_attempt(&self, identity: &DownloadAttempt) -> Result<(), String> {
        let changed = self.conn.execute(
            "UPDATE torrent_downloads SET attempt_id=?2, run_id=?3, torrent_id=NULL WHERE game_id=?1",
            params![identity.subject, identity.attempt, identity.run as i64],
        ).map_err(|e| e.to_string())?;
        if changed != 1 {
            return Err(SUPERSEDED.into());
        }
        Ok(())
    }

    pub(crate) fn download_attempt(&self, id: &str) -> Result<Option<DownloadAttempt>, String> {
        self.conn.query_row(
            "SELECT attempt_id,run_id FROM torrent_downloads WHERE game_id=?1 AND attempt_id IS NOT NULL",
            params![id],
            |row| Ok(DownloadAttempt { subject: id.into(), attempt: row.get(0)?, run: row.get::<_,i64>(1)? as u64 }),
        ).optional().map_err(|e| e.to_string())
    }

    fn require_attempt(&self, identity: &DownloadAttempt, from: &[&str]) -> Result<(), String> {
        let state = self.conn.query_row(
            "SELECT status FROM torrent_downloads WHERE game_id=?1 AND attempt_id=?2 AND run_id=?3",
            params![identity.subject, identity.attempt, identity.run as i64],
            |row| row.get::<_,String>(0),
        ).optional().map_err(|e| e.to_string())?;
        if !state.as_deref().is_some_and(|state| from.contains(&state)) {
            return Err(SUPERSEDED.into());
        }
        Ok(())
    }

    pub(crate) fn attempt_status(
        &self,
        identity: &DownloadAttempt,
        from: &[&str],
        to: &str,
        error: Option<&str>,
    ) -> Result<TorrentDownloadRecord, String> {
        self.in_transaction(|| {
            self.require_attempt(identity, from)?;
            self.set_torrent_status(&identity.subject, to, error)
        })
    }

    pub(crate) fn attempt_progress(
        &self,
        identity: &DownloadAttempt,
        p: WorkerProgress,
    ) -> Result<TorrentDownloadRecord, String> {
        self.in_transaction(|| {
            self.require_attempt(identity, &["downloading"])?;
            self.update_torrent_progress(
                &identity.subject,
                "downloading",
                p.percent,
                p.downloaded,
                p.total,
                p.speed,
                p.upload,
                p.peers,
            )
        })
    }

    pub(crate) fn attempt_handle(
        &self,
        identity: &DownloadAttempt,
        handle: usize,
        status: &str,
    ) -> Result<TorrentDownloadRecord, String> {
        self.in_transaction(|| {
            self.require_attempt(identity, &["resolving"])?;
            self.update_torrent_handle(&identity.subject, handle as i64, status)
        })
    }

    pub(crate) fn claim_queued_attempt(&self, identity: &DownloadAttempt) -> Result<bool, String> {
        let count = self.conn.execute(
            "UPDATE torrent_downloads SET status='resolving', run_id=?3, torrent_id=NULL WHERE game_id=?1 AND attempt_id=?2 AND status='queued'",
            params![identity.subject, identity.attempt, identity.run as i64],
        ).map_err(|e| e.to_string())?;
        Ok(count == 1)
    }

    pub(crate) fn complete_attempt(
        &self,
        identity: &DownloadAttempt,
        subject_type: &str,
        path: &str,
        sha: Option<&str>,
        total: u64,
    ) -> Result<(DownloadRecord, TorrentDownloadRecord), String> {
        self.in_transaction(|| {
            self.require_attempt(identity, &["downloading", "resolving"])?;
            if subject_type == "asset" {
                self.record_asset_installation(&identity.subject, Some(path), "ready", sha, None)?;
            }
            let download =
                self.record_download(&identity.subject, subject_type, Some(path), sha, None)?;
            self.conn
                .execute(
                    "UPDATE torrent_downloads SET save_dir=?2 WHERE game_id=?1",
                    params![identity.subject, path],
                )
                .map_err(|e| e.to_string())?;
            let torrent = self.update_torrent_progress(
                &identity.subject,
                "completed",
                100.0,
                total,
                total,
                0,
                0,
                0,
            )?;
            Ok((download, torrent))
        })
    }

    pub(crate) fn fail_attempt(
        &self,
        identity: &DownloadAttempt,
        subject_type: &str,
        error: &str,
    ) -> Result<TorrentDownloadRecord, String> {
        self.in_transaction(|| {
            self.require_attempt(identity, &["queued", "resolving", "downloading"])?;
            if subject_type == "asset" {
                let path = self
                    .get_torrent_download(&identity.subject)?
                    .map(|r| r.save_dir);
                self.record_asset_installation(
                    &identity.subject,
                    path.as_deref(),
                    "error",
                    None,
                    Some(error),
                )?;
            }
            self.record_download(&identity.subject, subject_type, None, None, Some(error))?;
            self.set_torrent_status(&identity.subject, "error", Some(error))
        })
    }

    pub(crate) fn delete_attempt_state(&self, id: &str, asset: bool) -> Result<(), String> {
        self.in_transaction(|| {
            if asset {
                self.delete_asset_state(id)?;
            } else {
                self.delete_download(id)?;
                self.delete_scrape_artifacts(id)?;
            }
            self.delete_torrent_download(id)?;
            self.conn
                .execute(
                    "DELETE FROM app_config WHERE key=?1",
                    params![format!("download_ownership:{id}")],
                )
                .map_err(|e| e.to_string())?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn late_attempt_and_runtime_callbacks_cannot_change_new_attempt_or_unpause() {
        let temp = tempfile::tempdir().unwrap();
        let store = RepositoryStore::open(&temp.path().join("db.sqlite")).unwrap();
        let a = DownloadAttempt {
            subject: "game".into(),
            attempt: "A".into(),
            run: 1,
        };
        store
            .record_direct_game_download_started("game", "http", "a", 10)
            .unwrap();
        store.assign_download_attempt(&a).unwrap();
        store.delete_attempt_state("game", false).unwrap();
        store
            .record_direct_game_download_started("game", "http", "b", 10)
            .unwrap();
        let b = DownloadAttempt {
            attempt: "B".into(),
            ..a.clone()
        };
        store.assign_download_attempt(&b).unwrap();
        assert!(store.complete_attempt(&a, "game", "a", None, 10).is_err());
        assert!(store.fail_attempt(&a, "game", "late").is_err());
        assert!(store
            .attempt_status(&a, &["downloading"], "cancelled", None)
            .is_err());
        store
            .attempt_status(&b, &["downloading"], "paused", None)
            .unwrap();
        assert!(store
            .attempt_progress(
                &b,
                WorkerProgress {
                    percent: 50.0,
                    downloaded: 5,
                    total: 10,
                    speed: 1,
                    upload: 0,
                    peers: 0
                }
            )
            .is_err());
        assert_eq!(
            store.get_torrent_download("game").unwrap().unwrap().status,
            "paused"
        );
        let run2 = DownloadAttempt {
            run: 2,
            ..b.clone()
        };
        store.assign_download_attempt(&run2).unwrap();
        assert!(store
            .attempt_status(&b, &["paused"], "downloading", None)
            .is_err());
    }
    #[test]
    fn first_completed_attempt_creates_download_and_cannot_recreate_deleted_row() {
        let temp = tempfile::tempdir().unwrap();
        let store = RepositoryStore::open(&temp.path().join("db.sqlite")).unwrap();
        let a = DownloadAttempt {
            subject: "game".into(),
            attempt: "A".into(),
            run: 1,
        };
        store
            .record_direct_game_download_started("game", "http", "a", 10)
            .unwrap();
        store.assign_download_attempt(&a).unwrap();
        assert!(store.get_download("game").unwrap().is_none());
        store.complete_attempt(&a, "game", "a", None, 10).unwrap();
        store.delete_attempt_state("game", false).unwrap();
        assert!(store.complete_attempt(&a, "game", "a", None, 10).is_err());
        assert!(store.get_download("game").unwrap().is_none());
    }
}
