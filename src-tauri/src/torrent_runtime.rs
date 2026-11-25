use crate::download_operations::{OperationCoordinator, OperationLease};
use crate::storage::{DownloadAttempt, WorkerProgress};

trait DownloadEventSink: Send + Sync {
    fn emit(&self, event: DownloadProgressEvent);
}
struct AppEvents(AppHandle);
impl DownloadEventSink for AppEvents {
    fn emit(&self, event: DownloadProgressEvent) {
        let _ = self.0.emit("download:progress", event);
    }
}

const STOP_LIMIT: Duration = Duration::from_secs(15);

impl TorrentManager {
    pub async fn new(
        data_dir: PathBuf,
        store: Arc<StoreMutex<RepositoryStore>>,
        app: AppHandle,
        operations: OperationCoordinator,
    ) -> Result<Self, String> {
        let engine = Arc::new(LibtorrentEngine::new(app.clone()));
        Self::build(
            engine,
            data_dir,
            store,
            Arc::new(AppEvents(app)),
            operations,
        )
        .await
    }

    async fn build(
        engine: Arc<dyn TorrentEngine>,
        data_dir: PathBuf,
        store: Arc<StoreMutex<RepositoryStore>>,
        events: Arc<dyn DownloadEventSink>,
        operations: OperationCoordinator,
    ) -> Result<Self, String> {
        let _ = data_dir;
        let manager = Self {
            engine,
            records: Arc::new(Mutex::new(HashMap::new())),
            store,
            events,
            operations,
            owners: Arc::new(StoreMutex::new(HashMap::new())),
            writer_guards: Arc::new(StoreMutex::new(HashMap::new())),
            tasks: Arc::new(StoreMutex::new(HashMap::new())),
            control: Arc::new(Mutex::new(())),
        };
        manager.restore_startup_downloads().await?;
        Ok(manager)
    }

    fn owner(&self, id: &str) -> Result<OperationLease, String> {
        self.owners
            .lock()
            .map_err(|_| "Torrent owners poisoned.")?
            .get(id)
            .cloned()
            .ok_or_else(|| format!("Torrent owner missing: {id}"))
    }

    fn release_writer_guard(&self, id: &str) {
        if let Ok(mut guards) = self.writer_guards.lock() {
            guards.remove(id);
        }
    }
    fn retain_torrent_guard(&self, id: &str, owner: &OperationLease) -> Result<(), String> {
        let guard = owner.protect_torrent_writes()?;
        self.writer_guards
            .lock()
            .map_err(|_| "Torrent path guards poisoned.")?
            .insert(id.into(), guard);
        Ok(())
    }

    fn emit_attempt(&self, record: &TorrentDownloadRecord, identity: &DownloadAttempt) {
        let mut event = event_from_status(&status_from_persisted_record(record));
        event.subject_type = record.subject_type.clone();
        event.display_name = record.display_name.clone();
        event.parent_game_id = record.parent_game_id.clone();
        event.attempt_id = Some(identity.attempt.clone());
        event.run_id = Some(identity.run);
        self.events.emit(event);
    }

    pub async fn start_magnet_download(
        &self,
        game_id: String,
        magnet_uri: String,
        save_dir: String,
    ) -> Result<TorrentStartReport, String> {
        validate_start_request(&game_id, &magnet_uri, &save_dir)?;
        let game_id = game_id.trim().to_string();
        let owner = self.operations.start(&game_id)?;
        let control = self.control.lock().await;
        self.operations.ensure_active(&owner)?;
        let installation = {
            let store = lock_store(&self.store)?;
            crate::download_installation::allocate(&store, &game_id, &owner, Path::new(&save_dir))?
        };
        let save_dir = installation.root().to_string_lossy().to_string();
        let identity = DownloadAttempt {
            subject: game_id.clone(),
            attempt: owner.attempt().into(),
            run: 0,
        };
        let record = {
            let store = lock_store(&self.store)?;
            let r = store.upsert_torrent_download_queued(&game_id, &magnet_uri, &save_dir)?;
            store.assign_download_attempt(&identity)?;
            r
        };
        self.owners
            .lock()
            .map_err(|_| "Torrent owners poisoned.")?
            .insert(game_id.clone(), owner);
        self.records.lock().await.insert(
            game_id.clone(),
            TorrentRecord {
                game_id: game_id.clone(),
                save_dir: save_dir.clone(),
                torrent_id: None,
                state: TorrentRecordState::Queued,
                error: None,
                identity: identity.clone(),
            },
        );
        self.emit_attempt(&record, &identity);
        drop(control);
        self.process_next_queued_download().await;
        let state = lock_store(&self.store)?
            .get_torrent_download(&game_id)?
            .map(|r| r.status)
            .unwrap_or_else(|| "queued".into());
        Ok(TorrentStartReport {
            game_id,
            state,
            save_dir,
        })
    }

    pub fn get_game_download(&self, id: &str) -> Result<Option<TorrentDownloadRecord>, String> {
        lock_store(&self.store)?.get_torrent_download(id)
    }
    pub fn list_downloads(&self) -> Result<Vec<TorrentDownloadRecord>, String> {
        lock_store(&self.store)?.list_torrent_downloads()
    }
    pub async fn get_torrent_status(&self, id: String) -> Result<TorrentStatus, String> {
        if let Some(status) = status_from_records(&self.engine, &self.records, id.clone()).await? {
            return Ok(status);
        }
        self.get_game_download(&id)?
            .map(|r| status_from_persisted_record(&r))
            .ok_or_else(|| format!("Unknown torrent download: {id}"))
    }

    async fn join_worker(&self, id: &str) -> Result<(), String> {
        let task = self
            .tasks
            .lock()
            .map_err(|_| "Torrent tasks poisoned.")?
            .remove(id);
        if let Some(mut task) = task {
            match tokio::time::timeout(STOP_LIMIT, &mut task).await {
                Ok(Ok(())) => {}
                // JoinError confirms that the future is gone, including on a
                // panic. The caller must still confirm the sidecar tree below;
                // returning early here would retain guards after a proven exit.
                Ok(Err(e)) => eprintln!("Torrent worker exited abnormally for {id}: {e}"),
                Err(_) => {
                    self.tasks
                        .lock()
                        .map_err(|_| "Torrent tasks poisoned.")?
                        .insert(id.into(), task);
                    return Err("Torrent worker stop timed out; files preserved.".into());
                }
            }
        }
        Ok(())
    }

    /// Called AFTER coordinator cancellation and BEFORE waiting for final leases.
    /// Never uses persisted torrent_id: those ids belong to an older process.
    pub async fn stop_download(&self, id: &str) -> Result<(), String> {
        let control = self.control.lock().await;
        self.operations.cancel(id)?;
        let runtime = {
            let mut records = self.records.lock().await;
            records
                .get_mut(id)
                .filter(|r| r.state != TorrentRecordState::Completed)
                .map(|r| {
                    let capture = matches!(
                        r.state,
                        TorrentRecordState::Resolving
                            | TorrentRecordState::Downloading
                            | TorrentRecordState::Cancelling
                    );
                    r.state = TorrentRecordState::Cancelling;
                    (r.clone(), capture)
                })
        };
        if let Some((runtime, capture)) = runtime {
            if let Ok(record) = lock_store(&self.store)?.attempt_status(
                &runtime.identity,
                &[
                    "queued",
                    "resolving",
                    "downloading",
                    "paused",
                    "error",
                    "cancelling",
                ],
                "cancelling",
                None,
            ) {
                self.emit_attempt(&record, &runtime.identity);
            }
            if runtime.identity.run > 0 {
                self.engine.stop_for_path(&runtime.save_dir).await?;
            }
            self.join_worker(id).await?;
            // add_magnet may have published its id while stop_for_path waited.
            if runtime.identity.run > 0 {
                self.engine.stop_for_path(&runtime.save_dir).await?;
            }
            self.release_writer_guard(id);
            if capture {
                if let Ok(owner) = self.owner(id) {
                    let owned = lock_store(&self.store)
                        .and_then(|s| crate::download_installation::read(&s, id))?;
                    if owned
                        .as_ref()
                        .is_some_and(|r| r.attempt() == owner.attempt())
                    {
                        // In-flight writers are now gone; capture their exact partials.
                        let _ = crate::download_installation::snapshot(
                            Arc::clone(&self.store),
                            id.into(),
                            owner,
                        )
                        .await;
                    }
                }
            }
        }
        self.owners
            .lock()
            .map_err(|_| "Torrent owners poisoned.")?
            .remove(id);
        drop(control);
        Ok(())
    }

    pub async fn cancel_download(&self, id: String) -> Result<TorrentDownloadRecord, String> {
        let existing = self
            .get_game_download(&id)?
            .ok_or_else(|| format!("Unknown download: {id}"))?;
        if existing.status == "completed" {
            return Err("Completed downloads cannot be cancelled.".into());
        }
        let ticket = self.operations.begin_delete(&id)?;
        if self
            .get_game_download(&id)?
            .is_some_and(|r| r.status == "completed")
        {
            return Err("Completed downloads cannot be cancelled.".into());
        }
        self.stop_download(&id).await?;
        ticket.wait_owner(STOP_LIMIT).await?;
        let identity = lock_store(&self.store)?
            .download_attempt(&id)?
            .ok_or("Download attempt missing.")?;
        let record = lock_store(&self.store)?.attempt_status(
            &identity,
            &[
                "queued",
                "resolving",
                "downloading",
                "paused",
                "error",
                "cancelling",
            ],
            "cancelled",
            None,
        )?;
        self.emit_attempt(&record, &identity);
        self.records.lock().await.remove(&id);
        drop(ticket);
        self.process_next_queued_download().await;
        Ok(record)
    }

    pub async fn pause_download(&self, id: String) -> Result<TorrentDownloadRecord, String> {
        let control = self.control.lock().await;
        let owner = self.owner(&id)?;
        self.operations.ensure_active(&owner)?;
        let runtime = {
            let mut records = self.records.lock().await;
            let r = records.get_mut(&id).ok_or("Torrent runtime missing.")?;
            if !matches!(
                r.state,
                TorrentRecordState::Queued
                    | TorrentRecordState::Resolving
                    | TorrentRecordState::Downloading
            ) {
                return Err("Download cannot be paused from this state.".into());
            }
            lock_store(&self.store)?.attempt_status(
                &r.identity,
                &["queued", "resolving", "downloading"],
                "cancelling",
                None,
            )?;
            r.state = TorrentRecordState::Cancelling;
            r.clone()
        };
        if runtime.identity.run > 0 {
            self.engine.stop_for_path(&runtime.save_dir).await?;
        }
        self.join_worker(&id).await?;
        if runtime.identity.run > 0 {
            self.engine.stop_for_path(&runtime.save_dir).await?;
        }
        self.release_writer_guard(&id);
        let owned =
            lock_store(&self.store).and_then(|s| crate::download_installation::read(&s, &id))?;
        if owned
            .as_ref()
            .is_some_and(|r| r.attempt() == owner.attempt())
        {
            crate::download_installation::snapshot(
                Arc::clone(&self.store),
                id.clone(),
                owner.clone(),
            )
            .await?;
        }
        let record = lock_store(&self.store)?.attempt_status(
            &runtime.identity,
            &["cancelling"],
            "paused",
            None,
        )?;
        if let Some(runtime) = self.records.lock().await.get_mut(&id) {
            runtime.state = TorrentRecordState::Paused;
        }
        self.emit_attempt(&record, &runtime.identity);
        drop(control);
        self.process_next_queued_download().await;
        Ok(record)
    }

    pub async fn resume_download(&self, id: String) -> Result<TorrentDownloadRecord, String> {
        let control = self.control.lock().await;
        let owner = self.owner(&id)?;
        self.operations.ensure_active(&owner)?;
        let identity = {
            let mut records = self.records.lock().await;
            let r = records.get_mut(&id).ok_or("Torrent runtime missing.")?;
            if !matches!(
                r.state,
                TorrentRecordState::Paused | TorrentRecordState::Error
            ) {
                return Err("Download cannot be resumed from this state.".into());
            }
            let record = lock_store(&self.store)?.attempt_status(
                &r.identity,
                &["paused", "error", "interrupted"],
                "queued",
                None,
            )?;
            r.state = TorrentRecordState::Queued;
            r.error = None;
            (r.identity.clone(), record)
        };
        let (identity, record) = identity;
        self.emit_attempt(&record, &identity);
        drop(control);
        self.process_next_queued_download().await;
        self.get_game_download(&id)?
            .ok_or("Download disappeared.".into())
    }

    async fn restore_startup_downloads(&self) -> Result<(), String> {
        let records = {
            let store = lock_store(&self.store)?;
            store.reset_runtime_handles()?;
            store.list_startup_torrent_downloads()?
        };
        for persisted in records {
            if is_direct_download_record(&persisted) {
                lock_store(&self.store)?.set_torrent_status(
                    &persisted.game_id,
                    "error",
                    Some("Direct download interrupted; retry is available."),
                )?;
                continue;
            }
            if persisted.status == "cancelling" {
                lock_store(&self.store)?.set_torrent_status(
                    &persisted.game_id,
                    "cancelled",
                    None,
                )?;
                continue;
            }
            let owner = self.operations.start(&persisted.game_id)?;
            let identity = DownloadAttempt {
                subject: persisted.game_id.clone(),
                attempt: owner.attempt().into(),
                run: 0,
            };
            let state = if persisted.status == "paused" {
                TorrentRecordState::Paused
            } else {
                TorrentRecordState::Queued
            };
            {
                let store = lock_store(&self.store)?;
                store.assign_download_attempt(&identity)?;
                crate::download_installation::renew_restored(&store, &persisted.game_id, &owner)?;
                store.set_torrent_status(&persisted.game_id, state.as_str(), None)?;
            }
            self.owners
                .lock()
                .map_err(|_| "Torrent owners poisoned.")?
                .insert(persisted.game_id.clone(), owner);
            self.records.lock().await.insert(
                persisted.game_id.clone(),
                TorrentRecord {
                    game_id: persisted.game_id,
                    save_dir: persisted.save_dir,
                    torrent_id: None,
                    state,
                    error: None,
                    identity,
                },
            );
        }
        self.process_next_queued_download().await;
        Ok(())
    }

    /// Selection, DB claim and runtime publication share the same short lock.
    pub async fn process_next_queued_download(&self) {
        let _control = self.control.lock().await;
        let claimed = {
            let mut records = self.records.lock().await;
            if active_download_count(&records) >= MAX_ACTIVE_DOWNLOADS {
                return;
            }
            let candidate = records
                .values()
                .find(|r| r.state == TorrentRecordState::Queued)
                .cloned();
            let Some(mut candidate) = candidate else {
                return;
            };
            let Ok(owner) = self.owner(&candidate.game_id) else {
                return;
            };
            if self.operations.ensure_active(&owner).is_err() {
                return;
            }
            candidate.identity.run += 1;
            let result = (|| {
                let store = lock_store(&self.store)?;
                if !store.claim_queued_attempt(&candidate.identity)? {
                    return Err("download_superseded".to_string());
                }
                let p = store
                    .get_torrent_download(&candidate.game_id)?
                    .ok_or("Download missing.")?;
                Ok(p.magnet_uri)
            })();
            let Ok(magnet) = result else {
                return;
            };
            candidate.state = TorrentRecordState::Resolving;
            records.insert(candidate.game_id.clone(), candidate.clone());
            Some((candidate, owner, magnet))
        };
        if let Some((runtime, owner, magnet)) = claimed {
            let id = runtime.game_id.clone();
            let manager = self.clone();
            let task = tauri::async_runtime::spawn(async move {
                manager.run_torrent(runtime, owner, magnet).await;
            });
            if let Ok(mut tasks) = self.tasks.lock() {
                tasks.insert(id, task);
            }
        }
    }

    fn schedule_queue(&self) {
        let manager = self.clone();
        tauri::async_runtime::spawn(async move {
            manager.process_next_queued_download().await;
        });
    }

    pub async fn forget_download(&self, id: &str) {
        self.records.lock().await.remove(id);
    }

    async fn finish_worker_error(
        &self,
        runtime: &TorrentRecord,
        owner: &OperationLease,
        error: &str,
    ) {
        let stopped = self.engine.stop_for_path(&runtime.save_dir).await.is_ok();
        let active = {
            let records = self.records.lock().await;
            records.get(&runtime.game_id).is_some_and(|r| {
                r.identity == runtime.identity
                    && matches!(
                        r.state,
                        TorrentRecordState::Resolving
                            | TorrentRecordState::Downloading
                            | TorrentRecordState::Error
                    )
            })
        };
        if !active {
            return;
        }
        if stopped {
            let owned = lock_store(&self.store)
                .and_then(|s| crate::download_installation::read(&s, &runtime.game_id))
                .ok()
                .flatten();
            if owned
                .as_ref()
                .is_some_and(|r| r.attempt() == owner.attempt())
            {
                let _ = crate::download_installation::snapshot(
                    Arc::clone(&self.store),
                    runtime.game_id.clone(),
                    owner.clone(),
                )
                .await;
            }
        }
        if let Ok(store) = lock_store(&self.store) {
            let record = if stopped {
                store.fail_attempt(&runtime.identity, "game", error)
            } else {
                store.attempt_status(
                    &runtime.identity,
                    &["resolving", "downloading", "error"],
                    "cancelling",
                    Some(error),
                )
            };
            if let Ok(record) = record {
                self.emit_attempt(&record, &runtime.identity);
            }
        }
        if let Some(record) = self.records.lock().await.get_mut(&runtime.game_id) {
            if record.identity == runtime.identity {
                record.state = if stopped {
                    TorrentRecordState::Error
                } else {
                    TorrentRecordState::Cancelling
                };
                record.error = Some(error.into());
            }
        }
        if stopped {
            self.release_writer_guard(&runtime.game_id);
            self.schedule_queue();
        }
    }

    async fn run_torrent(&self, runtime: TorrentRecord, owner: OperationLease, magnet: String) {
        let identity = runtime.identity.clone();
        let id = &runtime.game_id;
        if owner.is_cancelled() {
            return;
        }
        // Seal before metadata discovery; only exclusively reserved producer
        // paths may subsequently extend the inventory. Never adopt a legacy root.
        if let Err(error) = crate::download_installation::approve_torrent_files(
            Arc::clone(&self.store),
            id.clone(),
            owner.clone(),
            Vec::new(),
        )
        .await
        {
            self.finish_worker_error(&runtime, &owner, &error).await;
            return;
        }
        // A resumed sidecar checks existing payload before publishing metadata.
        // Protect every previously authorized name during that phase as well.
        if let Err(error) = self.retain_torrent_guard(id, &owner) {
            self.finish_worker_error(&runtime, &owner, &error).await;
            return;
        }
        let handle = match self
            .engine
            .add_magnet(magnet, runtime.save_dir.clone())
            .await
        {
            Ok(handle) => handle,
            Err(error) => {
                self.finish_worker_error(&runtime, &owner, &error).await;
                return;
            }
        };
        let state = {
            let mut records = self.records.lock().await;
            records
                .get_mut(id)
                .filter(|r| r.identity == identity)
                .map(|r| {
                    r.torrent_id = Some(handle);
                    r.state
                })
        };
        if owner.is_cancelled() || state != Some(TorrentRecordState::Resolving) {
            let _ = self.engine.stop(handle).await;
            return;
        }
        let started = {
            let result = lock_store(&self.store)
                .and_then(|s| s.attempt_handle(&identity, handle, "downloading"));
            match result {
                Ok(r) => r,
                Err(error) => {
                    self.finish_worker_error(&runtime, &owner, &error).await;
                    return;
                }
            }
        };
        if let Some(record) = self.records.lock().await.get_mut(id) {
            if record.identity == identity {
                record.state = TorrentRecordState::Downloading;
            }
        }
        self.emit_attempt(&started, &identity);
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        let mut inventory_approved = false;
        loop {
            tokio::select! {_ = owner.cancelled()=>{let _=self.engine.stop(handle).await;return;},_ = interval.tick()=>{}}
            let active = self.records.lock().await.get(id).is_some_and(|r| {
                r.identity == identity && r.state == TorrentRecordState::Downloading
            });
            if !active {
                return;
            }
            let Some(stats) = self.engine.stats(handle) else {
                self.finish_worker_error(&runtime, &owner, "Torrent statistics disappeared.")
                    .await;
                return;
            };
            if let Some(error) = stats.error.as_deref() {
                self.finish_worker_error(&runtime, &owner, error).await;
                return;
            }
            if !inventory_approved {
                let Some(paths) = stats.inventory.clone() else {
                    continue;
                };
                let approved = crate::download_installation::approve_torrent_files(
                    Arc::clone(&self.store),
                    id.clone(),
                    owner.clone(),
                    paths,
                )
                .await;
                if let Err(error) = approved {
                    self.finish_worker_error(&runtime, &owner, &error).await;
                    return;
                }
                if let Err(error) = self.retain_torrent_guard(id, &owner) {
                    self.finish_worker_error(&runtime, &owner, &error).await;
                    return;
                }
                // Cancellation between reservation and acknowledgement cannot
                // permit a stopped or superseded worker to start payload writes.
                if owner.is_cancelled()
                    || !self.records.lock().await.get(id).is_some_and(|r| {
                        r.identity == identity && r.state == TorrentRecordState::Downloading
                    })
                {
                    let _ = self.engine.stop(handle).await;
                    return;
                }
                if let Err(error) = self.engine.approve_inventory(handle).await {
                    self.finish_worker_error(&runtime, &owner, &error).await;
                    return;
                }
                inventory_approved = true;
            }
            if stats.finished || stats.progress >= 1.0 {
                if let Err(error) = self.engine.stop(handle).await {
                    self.finish_worker_error(&runtime, &owner, &error).await;
                    return;
                }
                if owner.is_cancelled() {
                    return;
                }
                // Existing legacy installs have no authority manifest; never adopt them.
                let owned = lock_store(&self.store)
                    .and_then(|s| crate::download_installation::read(&s, id))
                    .ok()
                    .flatten();
                if owned
                    .as_ref()
                    .is_some_and(|r| r.attempt() == owner.attempt())
                {
                    if let Err(error) = crate::download_installation::snapshot(
                        Arc::clone(&self.store),
                        id.clone(),
                        owner.clone(),
                    )
                    .await
                    {
                        self.finish_worker_error(&runtime, &owner, &error).await;
                        return;
                    }
                }
                let completed = lock_store(&self.store).and_then(|store| {
                    store.complete_attempt(
                        &identity,
                        "game",
                        &runtime.save_dir,
                        None,
                        stats.total_bytes,
                    )
                });
                if let Ok((_, record)) = completed {
                    self.emit_attempt(&record, &identity);
                    if let Some(r) = self.records.lock().await.get_mut(id) {
                        if r.identity == identity {
                            r.state = TorrentRecordState::Completed;
                        }
                    }
                    if let Ok(mut owners) = self.owners.lock() {
                        owners.remove(id);
                    }
                    let _ = self.engine.delete(handle, false).await;
                    self.release_writer_guard(id);
                    self.schedule_queue();
                } else if let Err(error) = completed {
                    self.finish_worker_error(&runtime, &owner, &error).await;
                }
                return;
            }
            let progress = WorkerProgress {
                percent: stats.progress * 100.0,
                downloaded: stats.downloaded_bytes,
                total: stats.total_bytes,
                speed: stats.download_speed_bytes_per_sec,
                upload: stats.upload_speed_bytes_per_sec,
                peers: stats.peers_count,
            };
            let persisted = lock_store(&self.store)
                .and_then(|store| store.attempt_progress(&identity, progress));
            match persisted {
                Ok(record) => self.emit_attempt(&record, &identity),
                Err(error) => {
                    self.finish_worker_error(&runtime, &owner, &error).await;
                    return;
                }
            }
        }
    }
}
