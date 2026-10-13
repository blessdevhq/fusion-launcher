use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::watch;

const MAGNET: &str = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567";

#[derive(Clone)]
struct FakeTorrent {
    path: String,
    stopped: bool,
    stats: EngineTorrentStats,
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
        }
    }

    fn add_count(&self) -> usize {
        self.adds.lock().unwrap().len()
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
            },
        );
    }
}

#[async_trait::async_trait]
impl TorrentEngine for FakeEngine {
    async fn add_magnet(&self, _: String, path: String) -> Result<usize, String> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        std::fs::write(Path::new(&path).join("payload.bin"), b"payload sentinel").unwrap();
        std::fs::write(
            Path::new(&path).join("resume.fastresume"),
            b"resume sentinel",
        )
        .unwrap();
        self.entries.lock().unwrap().insert(
            id,
            FakeTorrent {
                path: path.clone(),
                stopped: false,
                stats: EngineTorrentStats {
                    total_bytes: 16,
                    ..EngineTorrentStats::default()
                },
            },
        );
        self.adds.lock().unwrap().push((id, path));
        let mut gate = self.add_gate.subscribe();
        gate.wait_for(|blocked| !*blocked)
            .await
            .map_err(|error| error.to_string())?;
        if self.add_fails.load(Ordering::SeqCst) {
            return Err("injected add failure after writer creation".into());
        }
        Ok(id)
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
    fixture.manager.pause_download("one".into()).await.unwrap();
    let sentinel = Path::new(&started.save_dir).join("personal-save.txt");
    std::fs::write(&sentinel, b"preserve after resume").unwrap();
    fixture.manager.resume_download("one".into()).await.unwrap();
    eventually(|| fixture.engine.add_count() == 2).await;
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
}
