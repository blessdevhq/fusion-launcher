use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::watch;

const MAGNET: &str = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";

#[derive(Clone)]
struct FakeTorrent {
    path: String,
    stopped: bool,
    stats: EngineTorrentStats,
    approved: bool,
}

struct FakeEngine {
    next_id: AtomicUsize,
    entries: StoreMutex<HashMap<usize, FakeTorrent>>,
    adds: StoreMutex<Vec<(usize, String)>>,
    stops: StoreMutex<Vec<usize>>,
    path_stops: StoreMutex<Vec<String>>,
    deletes: StoreMutex<Vec<(usize, bool)>>,
    add_gate: watch::Sender<bool>,
    stop_fails: AtomicBool,
    add_fails: AtomicBool,
    add_panics: AtomicBool,
    late_inventory: AtomicBool,
    hide_metadata: AtomicBool,
}

impl FakeEngine {
    fn new() -> Self {
        let (add_gate, _) = watch::channel(false);
        Self {
            next_id: AtomicUsize::new(1),
            entries: StoreMutex::new(HashMap::new()),
            adds: StoreMutex::new(Vec::new()),
            stops: StoreMutex::new(Vec::new()),
            path_stops: StoreMutex::new(Vec::new()),
            deletes: StoreMutex::new(Vec::new()),
            add_gate,
            stop_fails: AtomicBool::new(false),
            add_fails: AtomicBool::new(false),
            add_panics: AtomicBool::new(false),
            late_inventory: AtomicBool::new(false),
            hide_metadata: AtomicBool::new(false),
        }
    }

    fn add_count(&self) -> usize {
        self.adds.lock().unwrap().len()
    }

    fn approved(&self, id: usize) -> bool {
        self.entries
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|entry| entry.approved)
    }

    fn set_finished(&self, id: usize) {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get_mut(&id).unwrap();
        entry.stats.finished = true;
        entry.stats.progress = 1.0;
        entry.stats.downloaded_bytes = entry.stats.total_bytes;
    }

    fn set_add_blocked(&self, blocked: bool) {
        self.add_gate.send_replace(blocked);
    }

    fn live(&self, id: usize) -> bool {
        self.entries
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|entry| !entry.stopped)
    }

    fn insert_foreign(&self, id: usize, path: &Path) {
        self.entries.lock().unwrap().insert(
            id,
            FakeTorrent {
                path: path.to_string_lossy().into_owned(),
                stopped: false,
                stats: EngineTorrentStats::default(),
                approved: false,
            },
        );
    }
}

#[async_trait::async_trait]
impl TorrentEngine for FakeEngine {
    async fn add_magnet(&self, _: String, path: String) -> Result<usize, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        // Fault injection models an old/misbehaving writer that creates files
        // before returning its handle. Those files must remain unowned.
        if *self.add_gate.borrow() || self.add_fails.load(Ordering::SeqCst) {
            std::fs::write(Path::new(&path).join("payload.bin"), b"payload sentinel").unwrap();
            std::fs::write(
                Path::new(&path).join("resume.fastresume"),
                b"resume sentinel",
            )
            .unwrap();
        }
        let mut inventory = vec!["payload.bin".into(), "resume.fastresume".into()];
        if self.late_inventory.load(Ordering::SeqCst) {
            inventory.push("new-after-resume.bin".into());
        }
        self.entries.lock().unwrap().insert(
            id,
            FakeTorrent {
                path: path.clone(),
                stopped: false,
                stats: EngineTorrentStats {
                    total_bytes: 16,
                    inventory: (!self.hide_metadata.load(Ordering::SeqCst)).then_some(inventory),
                    ..EngineTorrentStats::default()
                },
                approved: false,
            },
        );
        self.adds.lock().unwrap().push((id, path));
        let mut gate = self.add_gate.subscribe();
        gate.wait_for(|blocked| !*blocked)
            .await
            .map_err(|error| error.to_string())?;
        assert!(
            !self.add_panics.load(Ordering::SeqCst),
            "injected worker panic"
        );
        if self.add_fails.load(Ordering::SeqCst) {
            return Err("injected add failure after writer creation".into());
        }
        Ok(id)
    }

    async fn approve_inventory(&self, id: usize) -> Result<(), String> {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get_mut(&id).ok_or("missing fake runtime")?;
        if entry.stopped {
            return Err("fake runtime stopped".into());
        }
        let root = Path::new(&entry.path);
        for relative in entry.stats.inventory.as_ref().unwrap() {
            assert!(
                root.join(relative).is_file(),
                "Parent must reserve before acknowledgement"
            );
            let bytes: &[u8] = match relative.as_str() {
                "payload.bin" => b"payload sentinel",
                "resume.fastresume" => b"resume sentinel",
                _ => b"late payload",
            };
            std::fs::write(root.join(relative), bytes).unwrap();
        }
        entry.approved = true;
        Ok(())
    }

    async fn stop(&self, id: usize) -> Result<(), String> {
        self.stops.lock().unwrap().push(id);
        if self.stop_fails.load(Ordering::SeqCst) {
            return Err("injected unconfirmed process exit".into());
        }
        if let Some(entry) = self.entries.lock().unwrap().get_mut(&id) {
            entry.stopped = true;
        }
        Ok(())
    }

    async fn stop_for_path(&self, path: &str) -> Result<(), String> {
        self.path_stops.lock().unwrap().push(path.to_string());
        let ids: Vec<_> = self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, entry)| entry.path == path)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            self.stop(id).await?;
        }
        Ok(())
    }

    async fn pause(&self, id: usize) -> Result<(), String> {
        self.stop(id).await
    }
    async fn resume(&self, _: usize) -> Result<(), String> {
        Err("Runtime must allocate a new generation.".into())
    }
    async fn delete(&self, id: usize, delete_files: bool) -> Result<(), String> {
        self.stop(id).await?;
        self.deletes.lock().unwrap().push((id, delete_files));
        self.entries.lock().unwrap().remove(&id);
        Ok(())
    }
    fn exists(&self, id: usize) -> bool {
        self.entries.lock().unwrap().contains_key(&id)
    }
    fn stats(&self, id: usize) -> Option<EngineTorrentStats> {
        self.entries
            .lock()
            .unwrap()
            .get(&id)
            .map(|entry| entry.stats.clone())
    }
}

#[derive(Default)]
struct TestEvents(StoreMutex<Vec<DownloadProgressEvent>>);
impl DownloadEventSink for TestEvents {
    fn emit(&self, event: DownloadProgressEvent) {
        self.0.lock().unwrap().push(event);
    }
}

struct Fixture {
    _temp: tempfile::TempDir,
    parent: PathBuf,
    manager: TorrentManager,
    engine: Arc<FakeEngine>,
    events: Arc<TestEvents>,
    operations: OperationCoordinator,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_seed(|_, _, _| {}).await
    }

    async fn with_seed(seed: impl FnOnce(&RepositoryStore, &FakeEngine, &Path)) -> Self {
        let temp = tempfile::Builder::new()
            .prefix("torrent-runtime-")
            .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
            .unwrap();
        let parent = temp.path().join("downloads");
        std::fs::create_dir(&parent).unwrap();
        let store = RepositoryStore::open(&temp.path().join("test.sqlite")).unwrap();
        let engine = Arc::new(FakeEngine::new());
        seed(&store, &engine, &parent);
        let events = Arc::new(TestEvents::default());
        let operations = OperationCoordinator::new();
        let manager = TorrentManager::build(
            engine.clone(),
            temp.path().into(),
            Arc::new(StoreMutex::new(store)),
            events.clone(),
            operations.clone(),
        )
        .await
        .unwrap();
        Self {
            _temp: temp,
            parent,
            manager,
            engine,
            events,
            operations,
        }
    }

    async fn start(&self, id: &str) -> TorrentStartReport {
        self.manager
            .start_magnet_download(
                id.into(),
                MAGNET.into(),
                self.parent.to_string_lossy().into(),
            )
            .await
            .unwrap()
    }

    async fn seed_queued(&self, id: &str) {
        let owner = self.operations.start(id).unwrap();
        let identity = DownloadAttempt {
            subject: id.into(),
            attempt: owner.attempt().into(),
            run: 0,
        };
        let path = {
            let store = self.manager.store.lock().unwrap();
            let owned =
                crate::download_installation::allocate(&store, id, &owner, &self.parent).unwrap();
            let path = owned.root().to_string_lossy().into_owned();
            store
                .upsert_torrent_download_queued(id, MAGNET, &path)
                .unwrap();
            store.assign_download_attempt(&identity).unwrap();
            path
        };
        self.manager.owners.lock().unwrap().insert(id.into(), owner);
        self.manager.records.lock().await.insert(
            id.into(),
            TorrentRecord {
                game_id: id.into(),
                save_dir: path,
                torrent_id: None,
                state: TorrentRecordState::Queued,
                error: None,
                identity,
            },
        );
    }
}

async fn eventually(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("Runtime condition did not become true");
}

fn assert_payload_preserved(path: &str) {
    assert_eq!(
        std::fs::read(Path::new(path).join("payload.bin")).unwrap(),
        b"payload sentinel"
    );
    assert_eq!(
        std::fs::read(Path::new(path).join("resume.fastresume")).unwrap(),
        b"resume sentinel"
    );
}

#[tokio::test]
async fn simultaneous_queue_claims_launch_exactly_one_runtime() {
    let fixture = Fixture::new().await;
    fixture.engine.set_add_blocked(true);
    fixture.seed_queued("one").await;
    tokio::join!(
        fixture.manager.process_next_queued_download(),
        fixture.manager.process_next_queued_download()
    );
    eventually(|| fixture.engine.add_count() == 1).await;
    let record = fixture.manager.get_game_download("one").unwrap().unwrap();
    assert_eq!(record.run_id, Some(1));
    assert_eq!(fixture.manager.records.lock().await["one"].identity.run, 1);
    fixture.engine.set_add_blocked(false);
    fixture.manager.cancel_download("one".into()).await.unwrap();
    assert_eq!(fixture.engine.add_count(), 1);
}

#[tokio::test]
async fn restored_paused_id_does_not_stop_an_unrelated_current_process() {
    let fixture = Fixture::with_seed(|store, engine, parent| {
        let stale_path = parent.join("old-install");
        store
            .upsert_torrent_download_start("old", MAGNET, &stale_path.to_string_lossy())
            .unwrap();
        store.update_torrent_handle("old", 1, "paused").unwrap();
        engine.insert_foreign(1, &parent.join("other-current-install"));
    })
    .await;
    assert_eq!(
        fixture
            .manager
            .get_game_download("old")
            .unwrap()
            .unwrap()
            .torrent_id,
        None
    );
    fixture.manager.cancel_download("old".into()).await.unwrap();
    assert!(fixture.engine.live(1));
    assert!(fixture.engine.stops.lock().unwrap().is_empty());
    assert!(fixture.engine.path_stops.lock().unwrap().is_empty());
}

#[tokio::test]
async fn cancellation_during_add_waits_for_writer_and_preserves_payload() {
    let fixture = Fixture::new().await;
    fixture.engine.set_add_blocked(true);
    let started = fixture.start("one").await;
    eventually(|| fixture.engine.add_count() == 1).await;
    let manager = fixture.manager.clone();
    let cancellation = tokio::spawn(async move { manager.cancel_download("one".into()).await });
    eventually(|| {
        fixture.operations.start("one").is_err()
            && !fixture.engine.path_stops.lock().unwrap().is_empty()
    })
    .await;
    assert!(!cancellation.is_finished());
    assert_payload_preserved(&started.save_dir);
    fixture.engine.set_finished(1); // Late successful add must not publish completion.
    fixture.engine.set_add_blocked(false);
    let record = tokio::time::timeout(Duration::from_secs(5), cancellation)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(record.status, "cancelled");
    assert_payload_preserved(&started.save_dir);
    assert!(!fixture.engine.live(1));
    assert!(!fixture
        .events
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|event| event.game_id == "one" && event.status == "completed"));
}

#[tokio::test]
async fn failed_pause_blocks_resume_and_queue_until_confirmed_stop() {
    let fixture = Fixture::new().await;
    fixture.start("one").await;
    eventually(|| {
        fixture
            .manager
            .get_game_download("one")
            .unwrap()
            .is_some_and(|record| record.status == "downloading")
    })
    .await;
    fixture.start("two").await;
    fixture.engine.stop_fails.store(true, Ordering::SeqCst);
    assert!(fixture.manager.pause_download("one".into()).await.is_err());
    assert!(matches!(
        fixture.manager.records.lock().await["one"].state,
        TorrentRecordState::Cancelling
    ));
    assert!(fixture.manager.resume_download("one".into()).await.is_err());
    tokio::join!(
        fixture.manager.process_next_queued_download(),
        fixture.manager.process_next_queued_download()
    );
    assert_eq!(fixture.engine.add_count(), 1);
    assert_eq!(
        fixture
            .manager
            .get_game_download("two")
            .unwrap()
            .unwrap()
            .status,
        "queued"
    );
    fixture.engine.stop_fails.store(false, Ordering::SeqCst);
    fixture.manager.cancel_download("one".into()).await.unwrap();
    eventually(|| fixture.engine.add_count() == 2).await;
    assert!(!fixture.engine.live(1));
    fixture.manager.cancel_download("two".into()).await.unwrap();
}

#[tokio::test]
async fn confirmed_completion_preserves_files_and_advances_queue() {
    let fixture = Fixture::new().await;
    let first = fixture.start("one").await;
    eventually(|| {
        fixture
            .manager
            .get_game_download("one")
            .unwrap()
            .is_some_and(|record| record.status == "downloading")
    })
    .await;
    fixture.start("two").await;
    fixture.engine.set_finished(1);
    eventually(|| fixture.engine.add_count() == 2).await;
    assert_eq!(
        fixture
            .manager
            .get_game_download("one")
            .unwrap()
            .unwrap()
            .status,
        "completed"
    );
    assert_payload_preserved(&first.save_dir);
    assert!(fixture.engine.deletes.lock().unwrap().contains(&(1, false)));
    {
        let events = fixture.events.0.lock().unwrap();
        let completed = events
            .iter()
            .find(|event| event.game_id == "one" && event.status == "completed")
            .unwrap();
        assert!(completed.attempt_id.is_some());
        assert_eq!(completed.run_id, Some(1));
    }
    fixture.manager.cancel_download("two".into()).await.unwrap();
}

#[tokio::test]
async fn failed_add_with_unconfirmed_writer_keeps_slot_until_retry_stop() {
    let fixture = Fixture::new().await;
    fixture.engine.add_fails.store(true, Ordering::SeqCst);
    fixture.engine.stop_fails.store(true, Ordering::SeqCst);
    let first = fixture.start("one").await;
    eventually(|| {
        fixture
            .manager
            .get_game_download("one")
            .unwrap()
            .is_some_and(|record| record.status == "error" || record.status == "cancelling")
    })
    .await;
    fixture.start("two").await;
    fixture.manager.process_next_queued_download().await;
    assert_eq!(fixture.engine.add_count(), 1);
    assert!(fixture.manager.cancel_download("one".into()).await.is_err());
    assert!(fixture.operations.start("one").is_err());
    assert_payload_preserved(&first.save_dir);
    fixture.engine.add_fails.store(false, Ordering::SeqCst);
    fixture.engine.stop_fails.store(false, Ordering::SeqCst);
    fixture.manager.cancel_download("one".into()).await.unwrap();
    eventually(|| fixture.engine.add_count() == 2).await;
    fixture.manager.cancel_download("two".into()).await.unwrap();
}

#[tokio::test]
async fn removing_paused_torrent_does_not_adopt_files_added_after_pause() {
    let fixture = Fixture::new().await;
    let started = fixture.start("one").await;
    eventually(|| {
        fixture
            .manager
            .get_game_download("one")
            .unwrap()
            .is_some_and(|r| r.status == "downloading")
    })
    .await;
    eventually(|| fixture.engine.approved(1)).await;
    fixture.manager.pause_download("one".into()).await.unwrap();
    let sentinel = Path::new(&started.save_dir).join("unrelated.txt");
    std::fs::write(&sentinel, b"keep me").unwrap();
    assert!(fixture.manager.pause_download("one".into()).await.is_err());
    let ticket = fixture.operations.begin_delete("one").unwrap();
    fixture.manager.stop_download("one").await.unwrap();
    ticket.wait_owner(Duration::from_secs(2)).await.unwrap();
    let owned = {
        let store = fixture.manager.store.lock().unwrap();
        crate::download_installation::read(&store, "one")
            .unwrap()
            .unwrap()
    };
    crate::download_installation::cleanup(owned).await.unwrap();
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"keep me");
    assert!(!Path::new(&started.save_dir).join("payload.bin").exists());
}

#[tokio::test]
async fn resumed_completion_keeps_unknown_files_out_of_cleanup_authority() {
    let fixture = Fixture::new().await;
    let started = fixture.start("one").await;
    eventually(|| {
        fixture
            .manager
            .get_game_download("one")
            .unwrap()
            .is_some_and(|r| r.status == "downloading")
    })
    .await;
    eventually(|| fixture.engine.approved(1)).await;
    fixture.manager.pause_download("one".into()).await.unwrap();
    let sentinel = Path::new(&started.save_dir).join("personal-save.txt");
    std::fs::write(&sentinel, b"preserve after resume").unwrap();
    fixture.engine.late_inventory.store(true, Ordering::SeqCst);
    fixture.manager.resume_download("one".into()).await.unwrap();
    eventually(|| fixture.engine.add_count() == 2).await;
    eventually(|| fixture.engine.approved(2)).await;
    assert_eq!(
        std::fs::read(Path::new(&started.save_dir).join("new-after-resume.bin")).unwrap(),
        b"late payload"
    );
    fixture.engine.set_finished(2);
    eventually(|| {
        fixture
            .manager
            .get_game_download("one")
            .unwrap()
            .is_some_and(|r| r.status == "completed")
    })
    .await;
    assert!(fixture.manager.cancel_download("one".into()).await.is_err());
    let ticket = fixture.operations.begin_delete("one").unwrap();
    fixture.manager.stop_download("one").await.unwrap();
    ticket.wait_owner(Duration::from_secs(2)).await.unwrap();
    let owned = {
        let store = fixture.manager.store.lock().unwrap();
        crate::download_installation::read(&store, "one")
            .unwrap()
            .unwrap()
    };
    let preserved = crate::download_installation::cleanup(owned).await.unwrap();
    assert!(!preserved.is_empty());
    assert_eq!(std::fs::read(sentinel).unwrap(), b"preserve after resume");
    assert!(!Path::new(&started.save_dir).join("payload.bin").exists());
    assert!(!Path::new(&started.save_dir)
        .join("new-after-resume.bin")
        .exists());
}

#[tokio::test]
async fn conventional_torrent_removal_leaves_no_owned_directory() {
    let fixture = Fixture::new().await;
    let started = fixture.start("one").await;
    eventually(|| fixture.engine.approved(1)).await;
    let ticket = fixture.operations.begin_delete("one").unwrap();
    fixture.manager.stop_download("one").await.unwrap();
    ticket.wait_owner(Duration::from_secs(2)).await.unwrap();
    let owned = {
        let store = fixture.manager.store.lock().unwrap();
        crate::download_installation::read(&store, "one")
            .unwrap()
            .unwrap()
    };
    assert!(crate::download_installation::cleanup(owned)
        .await
        .unwrap()
        .is_empty());
    assert!(!Path::new(&started.save_dir).exists());
}

#[tokio::test]
async fn metadata_received_only_after_resume_extends_previously_sealed_empty_inventory() {
    let fixture = Fixture::new().await;
    fixture.engine.hide_metadata.store(true, Ordering::SeqCst);
    let started = fixture.start("one").await;
    eventually(|| {
        fixture
            .manager
            .get_game_download("one")
            .unwrap()
            .is_some_and(|r| r.status == "downloading")
    })
    .await;
    assert!(!fixture.engine.approved(1));
    fixture.manager.pause_download("one".into()).await.unwrap();
    assert!(!Path::new(&started.save_dir).join("payload.bin").exists());
    fixture.engine.hide_metadata.store(false, Ordering::SeqCst);
    fixture.manager.resume_download("one".into()).await.unwrap();
    eventually(|| fixture.engine.approved(2)).await;
    let ticket = fixture.operations.begin_delete("one").unwrap();
    fixture.manager.stop_download("one").await.unwrap();
    ticket.wait_owner(Duration::from_secs(2)).await.unwrap();
    let owned = {
        let store = fixture.manager.store.lock().unwrap();
        crate::download_installation::read(&store, "one")
            .unwrap()
            .unwrap()
    };
    assert!(crate::download_installation::cleanup(owned)
        .await
        .unwrap()
        .is_empty());
    assert!(!Path::new(&started.save_dir).exists());
}

#[tokio::test]
async fn resumed_metadata_cannot_adopt_an_existing_unowned_payload_path() {
    let fixture = Fixture::new().await;
    let started = fixture.start("one").await;
    eventually(|| fixture.engine.approved(1)).await;
    fixture.manager.pause_download("one".into()).await.unwrap();
    let collision = Path::new(&started.save_dir).join("new-after-resume.bin");
    std::fs::write(&collision, b"personal file matches later metadata path").unwrap();
    fixture.engine.late_inventory.store(true, Ordering::SeqCst);
    fixture.manager.resume_download("one".into()).await.unwrap();
    eventually(|| {
        fixture
            .manager
            .get_game_download("one")
            .unwrap()
            .is_some_and(|record| record.status == "error")
    })
    .await;
    assert!(!fixture.engine.approved(2));
    assert!(!fixture.engine.live(2));
    let ticket = fixture.operations.begin_delete("one").unwrap();
    fixture.manager.stop_download("one").await.unwrap();
    ticket.wait_owner(Duration::from_secs(2)).await.unwrap();
    let owned = {
        let store = fixture.manager.store.lock().unwrap();
        crate::download_installation::read(&store, "one")
            .unwrap()
            .unwrap()
    };
    assert!(!crate::download_installation::cleanup(owned)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        std::fs::read(collision).unwrap(),
        b"personal file matches later metadata path"
    );
    assert!(!Path::new(&started.save_dir).join("payload.bin").exists());
}

#[cfg(windows)]
fn assert_library_handles_released(fixture: &Fixture, root: &str) {
    let renamed = fixture._temp.path().join("renamed-library");
    std::fs::rename(&fixture.parent, &renamed)
        .expect("Confirmed terminal operation must release ancestor rename guards");
    std::fs::rename(&renamed, &fixture.parent).unwrap();
    let detached = fixture._temp.path().join("detached-installation");
    std::fs::rename(root, &detached).expect("Installation guards must also be released");
    std::fs::remove_dir(&fixture.parent)
        .expect("Empty library must be removable after confirmed shutdown");
    std::fs::create_dir(&fixture.parent).unwrap();
    std::fs::rename(&detached, root).unwrap();
}

#[cfg(windows)]
#[tokio::test]
async fn terminal_torrent_paths_release_library_handles() {
    for terminal in ["pause", "cancel", "complete", "error"] {
        let fixture = Fixture::new().await;
        fixture
            .engine
            .add_fails
            .store(terminal == "error", Ordering::SeqCst);
        let started = fixture.start("one").await;
        if terminal == "error" {
            eventually(|| {
                fixture
                    .manager
                    .tasks
                    .lock()
                    .unwrap()
                    .get("one")
                    .is_some_and(|task| task.inner().is_finished())
            })
            .await;
        } else {
            eventually(|| fixture.engine.approved(1)).await;
            match terminal {
                "pause" => {
                    fixture.manager.pause_download("one".into()).await.unwrap();
                }
                "cancel" => {
                    fixture.manager.cancel_download("one".into()).await.unwrap();
                }
                "complete" => {
                    fixture.engine.set_finished(1);
                    eventually(|| {
                        fixture
                            .manager
                            .tasks
                            .lock()
                            .unwrap()
                            .get("one")
                            .is_some_and(|task| task.inner().is_finished())
                    })
                    .await;
                }
                _ => unreachable!(),
            }
        }
        assert!(!fixture.engine.live(1), "{terminal} must confirm shutdown");
        assert_library_handles_released(&fixture, &started.save_dir);
    }
}

#[cfg(windows)]
#[tokio::test]
async fn confirmed_stop_after_worker_panic_releases_library_handles() {
    let fixture = Fixture::new().await;
    fixture.engine.add_panics.store(true, Ordering::SeqCst);
    let started = fixture.start("one").await;
    eventually(|| {
        fixture
            .manager
            .tasks
            .lock()
            .unwrap()
            .get("one")
            .is_some_and(|task| task.inner().is_finished())
    })
    .await;
    // A panic may be returned as an error, but cannot keep OS handles alive
    // once the engine has independently confirmed all writers are stopped.
    let _ = fixture.manager.stop_download("one").await;
    assert!(!fixture.engine.live(1));
    assert_library_handles_released(&fixture, &started.save_dir);
}

/// Run through sidecar/test_native_inventory.py with its generated loopback
/// seeder and FUSION_NATIVE_MANAGER_TEST_BINARY pointing at this test binary.
#[cfg(windows)]
#[tokio::test]
#[ignore = "requires explicit native packaged sidecar and generated loopback seeder"]
async fn native_packaged_sidecar_manager_pause_resume_and_remove() {
    let executable = PathBuf::from(std::env::var("FUSION_NATIVE_SIDECAR_EXE").unwrap());
    let magnet = std::env::var("FUSION_NATIVE_MAGNET").unwrap();
    let temporary = tempfile::Builder::new()
        .prefix("native-manager-")
        .tempdir_in(Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
        .unwrap();
    let parent = temporary.path().join("library");
    std::fs::create_dir(&parent).unwrap();
    let store = Arc::new(StoreMutex::new(
        RepositoryStore::open(&temporary.path().join("test.sqlite")).unwrap(),
    ));
    let engine =
        Arc::new(crate::libtorrent_engine::LibtorrentEngine::with_test_executable(executable));
    let operations = OperationCoordinator::new();
    let manager = TorrentManager::build(
        engine.clone(),
        temporary.path().into(),
        store.clone(),
        Arc::new(TestEvents::default()),
        operations.clone(),
    )
    .await
    .unwrap();
    let started = manager
        .start_magnet_download(
            "native".into(),
            magnet,
            parent.to_string_lossy().into_owned(),
        )
        .await
        .unwrap();
    async fn wait_bytes(
        manager: &TorrentManager,
        engine: &crate::libtorrent_engine::LibtorrentEngine,
        threshold: u64,
    ) -> u64 {
        tokio::time::timeout(Duration::from_secs(90), async {
            let mut last_log = std::time::Instant::now() - Duration::from_secs(10);
            loop {
                let record = manager.get_game_download("native").unwrap().unwrap();
                if last_log.elapsed() >= Duration::from_secs(5) {
                    println!("Native threshold={threshold} status={} run={:?} bytes={} engine1={:?} engine2={:?}",
                        record.status, record.run_id, record.downloaded_bytes,
                        engine.stats(1), engine.stats(2));
                    println!("Native stderr1={:?} stderr2={:?}", engine.test_stderr(1), engine.test_stderr(2));
                    last_log = std::time::Instant::now();
                }
                assert!(record.error_message.is_none(), "{:?}", record.error_message);
                if record.downloaded_bytes >= threshold {
                    break record.downloaded_bytes;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("Native sidecar failed to make expected progress")
    }
    let paused_bytes = wait_bytes(&manager, engine.as_ref(), 100 * 1024 * 1024).await;
    manager.pause_download("native".into()).await.unwrap();
    let persisted_bytes = engine.stats(1).unwrap().downloaded_bytes;
    assert_eq!(
        manager.get_game_download("native").unwrap().unwrap().status,
        "paused"
    );
    let renamed = temporary.path().join("library-after-pause");
    std::fs::rename(&parent, &renamed).expect("Paused real sidecar must release library handles");
    std::fs::rename(&renamed, &parent).unwrap();
    let root = Path::new(&started.save_dir);
    let resume = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "fastresume")
        })
        .unwrap();
    assert!(!std::fs::read(&resume).unwrap().is_empty());
    let resumed_at = std::time::Instant::now();
    manager.resume_download("native".into()).await.unwrap();
    let resumed_bytes =
        wait_bytes(&manager, engine.as_ref(), persisted_bytes + 8 * 1024 * 1024).await;
    println!("Native manager paused_bytes={paused_bytes} persisted_bytes={persisted_bytes} resumed_bytes={resumed_bytes} resume_seconds={:.3}",
        resumed_at.elapsed().as_secs_f64());
    let ticket = operations.begin_delete("native").unwrap();
    manager.stop_download("native").await.unwrap();
    ticket.wait_owner(Duration::from_secs(10)).await.unwrap();
    let owned = crate::download_installation::read(&store.lock().unwrap(), "native")
        .unwrap()
        .unwrap();
    let preserved = crate::download_installation::cleanup(owned).await.unwrap();
    assert!(
        preserved.is_empty(),
        "Native sidecar left unknown payload: {preserved:?}"
    );
    assert!(
        !root.exists(),
        "Confirmed native removal must remove the installation completely"
    );
    std::fs::remove_dir(&parent).expect("Stopped real sidecar must release all library handles");
    println!("Native manager confirmed tree shutdown; owned installation and library removed");
}
