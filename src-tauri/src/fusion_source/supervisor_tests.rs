use super::*;

#[test]
fn ring_bounds_bytes_lines_and_utf8_when_newlines_are_absent() {
    let limits = LogLimits {
        max_bytes: 64,
        max_lines: 3,
        max_line_bytes: 24,
    };
    let logs = Mutex::new(BoundedLogRingBuffer::new(limits).unwrap());
    let mut accumulator = LineAccumulator {
        pending: Vec::new(),
        truncated: false,
        limit: limits.max_line_bytes,
    };
    for _ in 0..1024 {
        accumulator.consume(&[b'x'; 4096], "main", LogStream::Stdout, &logs);
    }
    assert_eq!(accumulator.pending.len(), 24);
    accumulator.consume(b"\nfinal\n", "main", LogStream::Stdout, &logs);
    assert!(lock_logs(&logs).lines()[0].truncated);
    for _ in 0..50 {
        lock_logs(&logs).push(LogLine {
            process_id: "main".into(),
            stream: LogStream::Stderr,
            text: "日本語日本語日本語日本語".into(),
            truncated: false,
        });
    }
    let logs = lock_logs(&logs);
    assert!(logs.byte_len() <= 64);
    assert!(logs.lines().len() <= 3);
    assert!(logs.dropped_lines() > 0);
    assert!(logs
        .lines()
        .iter()
        .all(|line| line.truncated && line.text.len() <= 24));
}

#[test]
fn partial_log_lines_have_an_independent_memory_ceiling() {
    assert_eq!(
        BoundedLogRingBuffer::new(LogLimits {
            max_bytes: 64 * 1024 * 1024,
            max_lines: 1000,
            max_line_bytes: 64 * 1024 + 1
        })
        .unwrap_err()
        .code,
        ErrorCode::InvalidManifest
    );
    assert!(BoundedLogRingBuffer::new(LogLimits {
        max_bytes: 64 * 1024 * 1024,
        max_lines: 1000,
        max_line_bytes: 64 * 1024
    })
    .is_ok());
}

#[test]
fn parameters_are_literal_vectors_and_save_environment_cannot_escape() {
    let spec = ProcessSpec {
        id: "game".into(),
        executable: "bin/game.exe".into(),
        args: vec![
            "{{WORKSPACE_DIR}}/name with spaces".into(),
            "& echo secret | other".into(),
        ],
        env: BTreeMap::from([("APPDATA".into(), "{{APPDATA}}".into())]),
        depends_on: Vec::new(),
        role: ProcessRole::Primary,
    };
    let environment = BTreeMap::from([
        ("APPDATA".into(), "C:/owned/appdata".into()),
        ("LOCALAPPDATA".into(), "C:/owned/localappdata".into()),
    ]);
    let command = prepare_command(&spec, Path::new("C:/owned/workspace"), &environment).unwrap();
    let args: Vec<_> = command
        .get_args()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args,
        [
            "C:/owned/workspace/name with spaces",
            "& echo secret | other"
        ]
    );
    let mut invalid = spec.clone();
    invalid
        .env
        .insert("appdata".into(), "C:/Users/private".into());
    assert_eq!(
        prepare_command(&invalid, Path::new("C:/owned/workspace"), &environment)
            .unwrap_err()
            .code,
        ErrorCode::InvalidEnvironment
    );
    invalid = spec.clone();
    invalid.executable = "bin/cmd.exe".into();
    assert_eq!(
        prepare_command(&invalid, Path::new("C:/owned/workspace"), &environment)
            .unwrap_err()
            .code,
        ErrorCode::ProcessLaunchFailed
    );
    invalid = spec;
    invalid.args = vec!["{{UNKNOWN}}".into()];
    assert_eq!(
        prepare_command(&invalid, Path::new("C:/owned/workspace"), &environment)
            .unwrap_err()
            .code,
        ErrorCode::InvalidEnvironment
    );
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::ffi::c_void;
    use std::fs;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::PathBuf;
    use std::time::Instant;

    use crate::fusion_source::canonical::QualifiedGameId;
    use crate::fusion_source::capabilities::CapabilityRegistry;
    use crate::fusion_source::cas::hash_file;
    use crate::fusion_source::config_templates::TemplateContext;
    use crate::fusion_source::layers::LayerInput;
    use crate::fusion_source::persistence::PersistenceRoots;
    use crate::fusion_source::plan::{PlanBuilder, PlanOptions};
    use crate::fusion_source::schema::DigestAlgorithm;
    use crate::fusion_source::workspace::{
        VerifiedManifestBinding, WorkspaceAssembler, WorkspaceBuildOptions,
    };
    use async_trait::async_trait;
    use serde_json::json;

    const HELPER: &str = "fusion_source::supervisor::tests::native::native_process_fixture";
    #[test]
    fn native_process_fixture() {
        let Ok(mode) = std::env::var("FUSION_SUPERVISOR_FIXTURE") else {
            return;
        };
        if mode == "descendant" {
            std::thread::sleep(Duration::from_secs(120));
            return;
        }
        if mode == "support" {
            println!("support stdout");
            eprintln!("support stderr");
            std::thread::sleep(Duration::from_secs(120));
            return;
        }
        if mode == "tree" {
            let child = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", HELPER, "--nocapture"])
                .env("FUSION_SUPERVISOR_FIXTURE", "descendant")
                .spawn()
                .unwrap();
            fs::write(
                std::env::var("FUSION_MARKER").unwrap(),
                child.id().to_string(),
            )
            .unwrap();
            // The private job, not this fixture, owns and stops the descendant.
            std::mem::forget(child);
            std::thread::sleep(Duration::from_secs(120));
            return;
        }
        if mode == "delayed" {
            std::thread::sleep(Duration::from_millis(100));
        }
        if let Ok(save) = std::env::var("FUSION_SAVE") {
            let save = PathBuf::from(save);
            let temporary = save.with_extension("tmp");
            fs::write(&temporary, b"new atomic save").unwrap();
            fs::rename(temporary, save).unwrap();
        }
        if matches!(mode.as_str(), "quiet-crash" | "key-only-crash") {
            std::process::exit(37);
        }
        if mode == "privacy-crash" {
            for key in ["APPDATA", "LOCALAPPDATA", "TEMP", "TMP"] {
                eprintln!("{}", std::env::var(key).unwrap());
            }
            std::process::exit(37);
        }
        for index in 0..400 {
            println!("stdout {index} {}", "x".repeat(128));
            eprintln!("stderr {index}");
        }
        println!("{}", "z".repeat(1024 * 1024));
        println!("final stdout marker");
        eprintln!("final stderr marker");
        if mode == "crash" {
            std::process::exit(37);
        }
    }

    struct Trust(bool);
    impl ExecutableTrust for Trust {
        fn is_trusted(&self, _: &QualifiedGameId, _: &str, _: &str) -> Result<bool, FusionError> {
            Ok(self.0)
        }
    }
    struct SyncProof {
        expected: String,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }
    #[async_trait]
    impl ManifestSyncVerifier for SyncProof {
        async fn verify_current(
            &self,
            _: &QualifiedGameId,
            digest: &str,
        ) -> Result<(), FusionError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if digest == self.expected {
                Ok(())
            } else {
                Err(FusionError::new(
                    ErrorCode::StrictSyncRequired,
                    "Index changed.",
                ))
            }
        }
    }
    struct Fixture {
        _temp: tempfile::TempDir,
        assembler: WorkspaceAssembler,
        lease: Option<AssembledWorkspace>,
        base: PathBuf,
        persistent_save: PathBuf,
        marker: PathBuf,
    }
    fn fixture(mode: &str, graph: bool, strict: bool, raw_digest: Option<String>) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("owned-local-base");
        let library = temp.path().join("library");
        let appdata = temp.path().join("original-appdata");
        let local = temp.path().join("original-local-appdata");
        for root in [&base.join("saves"), &library, &appdata, &local] {
            fs::create_dir_all(root).unwrap();
        }
        fs::copy(std::env::current_exe().unwrap(), base.join("main.exe")).unwrap();
        // The Rust/Tauri test binary may use an external Common Controls v6
        // activation manifest. Preserve it when renaming the fixture binary;
        // otherwise Windows loads comctl32 v5 and reports ENTRYPOINT_NOT_FOUND.
        let executable = std::env::current_exe().unwrap();
        let sidecar_manifest = PathBuf::from(format!("{}.manifest", executable.display()));
        let activation_manifest_digest = if sidecar_manifest.is_file() {
            fs::copy(sidecar_manifest, base.join("main.exe.manifest")).unwrap();
            Some(hash_file(&base.join("main.exe.manifest"), DigestAlgorithm::Blake3).unwrap())
        } else {
            None
        };
        fs::write(base.join("saves/progress.dat"), b"original save").unwrap();
        fs::write(
            base.join("lease-lock-probe.bin"),
            b"immutable local layer probe",
        )
        .unwrap();
        if mode == "invalid" {
            fs::write(base.join("bad.exe"), b"not a PE executable").unwrap();
        }
        let digest = hash_file(&base.join("main.exe"), DigestAlgorithm::Blake3).unwrap();
        let env = json!({"FUSION_SUPERVISOR_FIXTURE":mode,"FUSION_SAVE":"{{WORKSPACE_DIR}}/saves/progress.dat","FUSION_MARKER":"{{APPDATA}}/descendant.pid"});
        let primary = json!({"id":"game","executable":if mode == "invalid" {"bad.exe"} else {"main.exe"},"args":["--exact",HELPER,"--nocapture"],"env":env,"role":"primary","depends_on":if graph {vec!["support"]} else {vec![]}});
        let mut document = json!({"schema_version":"1.0","id":"process-fixture","title":"Owned native process fixture","version":"1",
            "layers":[{"id":"base","kind":"base","source":{"kind":"local_locator","locator":{"store":"manual","path_hints":[base.to_string_lossy()],"key_files":[{"path":"main.exe","blake3":digest,"role":"main_executable"}]}},"critical_files":[{"path":"main.exe","blake3":digest,"role":"main_executable"}]}],
            "persistence":{"saves_paths":["workspace/saves/progress.dat"]}});
        if let Some(digest) = activation_manifest_digest {
            document["layers"][0]["critical_files"]
                .as_array_mut()
                .unwrap()
                .push(json!({"path":"main.exe.manifest","blake3":digest,"role":"core_library"}));
        }
        if mode == "key-only-crash" {
            // A local locator key can attest the primary executable without a
            // duplicate declaration in the layer's critical_files array.
            document["layers"][0]["critical_files"]
                .as_array_mut()
                .unwrap()
                .retain(|file| file["path"] != "main.exe");
        }
        if mode == "invalid" {
            document["layers"][0]["critical_files"].as_array_mut().unwrap().push(json!({"path":"bad.exe","blake3":blake3::hash(b"not a PE executable").to_hex().to_string(),"role":"main_executable"}));
        }
        if graph || strict {
            let mut processes = vec![primary];
            if graph {
                processes.push(json!({"id":"support","executable":"main.exe","args":["--exact",HELPER,"--nocapture"],"env":{"FUSION_SUPERVISOR_FIXTURE":"support"},"role":"supporting"}));
            }
            document["orchestrator"] = json!({"processes":processes,"strict_sync":strict});
        } else {
            document["launch"] = primary;
        }
        let manifest =
            SchemaUpcaster::upcast_game_manifest("tests", &document.to_string(), &BTreeMap::new())
                .unwrap();
        let registry = CapabilityRegistry::new([
            "core.schema.v1",
            "core.plan.v1",
            "core.layers.v1",
            "vfs.hardlink.v1",
            "local_locator.v1",
            "proc.supervisor.v1",
            "persistence.saves.v1",
            "orchestrator.multi_process.v1",
            "orchestrator.strict_sync.v1",
        ]);
        let plan = PlanBuilder::build(&manifest, &PlanOptions::default(), &registry).unwrap();
        let game = QualifiedGameId {
            source_id: "tests".into(),
            game_id: "process-fixture".into(),
        };
        let assembler = WorkspaceAssembler::open(&library, &game).unwrap();
        let context = TemplateContext {
            workspace_dir: library.clone(),
            appdata_dir: appdata.clone(),
            local_appdata_dir: local.clone(),
            display_width: 1920,
            display_height: 1080,
            refresh_rate: 60,
            enabled_addons: BTreeSet::new(),
        };
        let lease = assembler
            .assemble(
                &manifest,
                &plan,
                &BTreeMap::from([(
                    "base".into(),
                    LayerInput::LocalDirectory { root: base.clone() },
                )]),
                &context,
                &PersistenceRoots {
                    appdata,
                    local_appdata: local,
                },
                &WorkspaceBuildOptions {
                    verified_manifest: raw_digest.as_deref().map(|digest| {
                        VerifiedManifestBinding::from_verified(&manifest, digest).unwrap()
                    }),
                    ..Default::default()
                },
            )
            .unwrap();
        let persistent_save = lease.snapshot.persistence.bindings[0]
            .persistent_path
            .clone();
        let marker =
            PathBuf::from(&lease.isolated_environment().unwrap()["APPDATA"]).join("descendant.pid");
        Fixture {
            _temp: temp,
            assembler,
            lease: Some(lease),
            base,
            persistent_save,
            marker,
        }
    }
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }
    fn bounded_options() -> SupervisorOptions {
        SupervisorOptions {
            logs: LogLimits {
                max_bytes: 4096,
                max_lines: 20,
                max_line_bytes: 256,
            },
            telemetry: None,
            ..Default::default()
        }
    }
    fn assert_lease_released(fixture: &Fixture) {
        // Our operation lock must be free immediately. Opening a new lease and
        // dropping it also proves all handles that lease owns can be released.
        let reopened = fixture.assembler.active().unwrap().unwrap();
        drop(reopened);
        // This immutable source file was protected by the same local-base
        // lease, but has no executable image mapping or loader dependency.
        // Its deny-write handle must be gone without any retry.
        let probe = fs::OpenOptions::new()
            .write(true)
            .open(fixture.base.join("lease-lock-probe.bin"));
        assert!(
            probe.is_ok(),
            "local layer deny-write handle remained after completion: {probe:?}"
        );
        drop(probe);
        // Windows image teardown/antivirus scanning can briefly deny WRITE on
        // a freshly exited executable under parallel test load. Retry only the
        // exact sharing violation; a retained lease would fail this deadline.
        // This probe neither creates, truncates nor changes the executable.
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .open(fixture.base.join("main.exe"))
            {
                Ok(file) => {
                    drop(file);
                    break;
                }
                Err(error) if error.raw_os_error() == Some(32) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("local executable lock remained after completion: {error:?}"),
            }
        }
    }
    #[test]
    fn native_exit_captures_bounded_output_reconciles_atomic_save_and_releases_lease() {
        let mut fixture = fixture("exit", false, false, None);
        let result = runtime().block_on(async {
            ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(true)),
                None,
                bounded_options(),
            )
            .await
            .unwrap()
            .wait()
            .await
            .unwrap()
        });
        assert_eq!(result.primary_exit_code, 0);
        assert!(!result.user_cancelled);
        assert!(result.sanitized_crash_report.is_none());
        assert!(result.log_tail.len() <= 20);
        assert!(result.dropped_log_lines > 0);
        assert!(result.log_tail.iter().any(|line| line.truncated));
        assert!(result
            .log_tail
            .iter()
            .any(|line| line.stream == LogStream::Stderr));
        assert_eq!(
            fs::read(&fixture.persistent_save).unwrap(),
            b"new atomic save"
        );
        assert_eq!(
            fs::read(fixture.base.join("saves/progress.dat")).unwrap(),
            b"original save"
        );
        assert_lease_released(&fixture);
    }
    #[test]
    fn graph_primary_crash_stops_support_process_and_returns_sanitized_report() {
        let mut fixture = fixture("crash", true, false, None);
        let result = runtime().block_on(async {
            ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(true)),
                None,
                bounded_options(),
            )
            .await
            .unwrap()
            .wait()
            .await
            .unwrap()
        });
        assert_eq!(result.primary_exit_code, 37);
        assert_eq!(result.process_exit_codes.len(), 2);
        assert!(!result.user_cancelled);
        assert!(result.sanitized_crash_report.is_some());
        assert_eq!(
            fs::read(&fixture.persistent_save).unwrap(),
            b"new atomic save"
        );
        assert_lease_released(&fixture);
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn WaitForSingleObject(handle: *mut c_void, milliseconds: u32) -> u32;
    }
    async fn descendant_handle(marker: &Path) -> OwnedHandle {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(pid) = fs::read_to_string(marker) {
                if let Ok(pid) = pid.parse() {
                    let raw = unsafe { OpenProcess(0x00100000, 0, pid) };
                    assert!(
                        !raw.is_null(),
                        "OpenProcess fixture descendant: {}",
                        std::io::Error::last_os_error()
                    );
                    return unsafe { OwnedHandle::from_raw_handle(raw) };
                }
            }
            assert!(
                Instant::now() < deadline,
                "descendant failed to publish pid"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    #[test]
    fn cancelling_job_waits_for_actual_descendants_and_drops_all_base_locks() {
        let mut fixture = fixture("tree", false, false, None);
        runtime().block_on(async {
            let running = ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(true)),
                None,
                bounded_options(),
            )
            .await
            .unwrap();
            let descendant = descendant_handle(&fixture.marker).await;
            assert_eq!(
                fixture.assembler.active().unwrap_err().code,
                ErrorCode::OperationInProgress
            );
            assert!(fs::OpenOptions::new()
                .write(true)
                .open(fixture.base.join("main.exe"))
                .is_err());
            assert!(fs::OpenOptions::new()
                .write(true)
                .open(fixture.base.join("lease-lock-probe.bin"))
                .is_err());
            running.cancel();
            let result = running.wait().await.unwrap();
            assert!(result.user_cancelled);
            assert!(result.sanitized_crash_report.is_none());
            assert_eq!(
                unsafe { WaitForSingleObject(descendant.as_raw_handle(), 0) },
                0
            );
        });
        assert_lease_released(&fixture);
    }
    #[test]
    fn dropping_caller_future_cancels_tree_without_dropping_lease_early() {
        let mut fixture = fixture("tree", false, false, None);
        runtime().block_on(async {
            let running = ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(true)),
                None,
                bounded_options(),
            )
            .await
            .unwrap();
            let descendant = descendant_handle(&fixture.marker).await;
            let mut waiting = Box::pin(running.wait());
            // Poll the wait future into Pending, then abort the caller while
            // the actual child and descendant are still alive.
            tokio::select! {
                result = &mut waiting => panic!("fixture unexpectedly exited before caller abort: {result:?}"),
                _ = tokio::time::sleep(Duration::from_millis(20)) => {},
            }
            drop(waiting);
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                match fixture.assembler.active() {
                    Ok(Some(lease)) => {
                        drop(lease);
                        break;
                    }
                    Err(error) if error.code == ErrorCode::OperationInProgress => {}
                    result => panic!("unexpected active result: {result:?}"),
                }
                assert!(Instant::now() < deadline, "ownership worker did not finish");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            assert_eq!(
                unsafe { WaitForSingleObject(descendant.as_raw_handle(), 0) },
                0
            );
        });
        assert_lease_released(&fixture);
    }
    #[test]
    fn strict_sync_requires_raw_hash_and_fresh_verifier_before_any_spawn() {
        let mut fixture = fixture("exit", false, true, None);
        let error = runtime()
            .block_on(ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(true)),
                None,
                bounded_options(),
            ))
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::StrictSyncRequired);
        assert!(!fixture.marker.exists());
        assert_lease_released(&fixture);
        let digest = blake3::hash(b"verified raw manifest").to_hex().to_string();
        let mut fixture = self::fixture("exit", false, true, Some(digest.clone()));
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let verifier = Arc::new(SyncProof {
            expected: digest,
            calls: Arc::clone(&calls),
        });
        let result = runtime().block_on(async {
            ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(true)),
                Some(verifier),
                bounded_options(),
            )
            .await
            .unwrap()
            .wait()
            .await
            .unwrap()
        });
        assert_eq!(result.primary_exit_code, 0);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_lease_released(&fixture);
    }
    #[test]
    fn untrusted_process_never_starts_and_worker_releases_lease() {
        let mut fixture = fixture("exit", false, false, None);
        let error = runtime().block_on(async {
            ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(false)),
                None,
                bounded_options(),
            )
            .await
            .unwrap()
            .wait()
            .await
            .err()
            .unwrap()
        });
        assert_eq!(error.code, ErrorCode::UntrustedExecutable);
        assert_eq!(
            fs::read(&fixture.persistent_save).unwrap(),
            b"original save"
        );
        assert_lease_released(&fixture);
    }

    #[test]
    fn partial_graph_startup_failure_stops_started_support_and_releases_lease() {
        let mut fixture = fixture("invalid", true, false, None);
        let error = runtime().block_on(async {
            ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(true)),
                None,
                bounded_options(),
            )
            .await
            .unwrap()
            .wait()
            .await
            .err()
            .unwrap()
        });
        assert_eq!(error.code, ErrorCode::ProcessLaunchFailed);
        assert_eq!(
            fs::read(&fixture.persistent_save).unwrap(),
            b"original save"
        );
        assert_lease_released(&fixture);
    }

    #[test]
    fn stale_strict_sync_proof_cannot_start_any_process() {
        let digest = blake3::hash(b"installed source manifest")
            .to_hex()
            .to_string();
        let mut fixture = fixture("exit", false, true, Some(digest));
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let verifier = Arc::new(SyncProof {
            expected: blake3::hash(b"updated source manifest")
                .to_hex()
                .to_string(),
            calls: Arc::clone(&calls),
        });
        let error = runtime()
            .block_on(ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(true)),
                Some(verifier),
                bounded_options(),
            ))
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::StrictSyncRequired);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            fs::read(&fixture.persistent_save).unwrap(),
            b"original save"
        );
        assert_lease_released(&fixture);
    }

    #[test]
    fn repeated_native_crash_is_suppressed_by_persisted_signature_limiter() {
        let mut fixture = fixture("quiet-crash", false, false, None);
        let first = runtime().block_on(async {
            ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(true)),
                None,
                bounded_options(),
            )
            .await
            .unwrap()
            .wait()
            .await
            .unwrap()
        });
        assert!(first.sanitized_crash_report.is_some());
        assert!(matches!(
            first.crash_deduplication,
            Some(DeduplicationDecision::Accepted { .. })
        ));
        let lease = fixture.assembler.active().unwrap().unwrap();
        let repeated = runtime().block_on(async {
            ProcessSupervisor::launch(lease, Arc::new(Trust(true)), None, bounded_options())
                .await
                .unwrap()
                .wait()
                .await
                .unwrap()
        });
        assert_eq!(repeated.primary_exit_code, 37);
        assert!(repeated.sanitized_crash_report.is_none());
        assert!(matches!(
            repeated.crash_deduplication,
            Some(DeduplicationDecision::SuppressedCooldown { .. })
        ));
        assert_lease_released(&fixture);
    }

    #[test]
    fn empty_custom_sanitizer_cannot_disable_redaction_of_actual_isolated_environment() {
        use crate::fusion_source::telemetry::{LogSanitizationLimits, TelemetryContext};
        let mut fixture = fixture("privacy-crash", false, false, None);
        let environment = fixture
            .lease
            .as_ref()
            .unwrap()
            .isolated_environment()
            .unwrap();
        let options = SupervisorOptions {
            telemetry: Some(
                TelemetrySanitizer::new(
                    TelemetryContext::default(),
                    LogSanitizationLimits::default(),
                )
                .unwrap(),
            ),
            ..Default::default()
        };
        let outcome = runtime().block_on(async {
            ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(true)),
                None,
                options,
            )
            .await
            .unwrap()
            .wait()
            .await
            .unwrap()
        });
        let report = outcome.sanitized_crash_report.unwrap();
        for path in environment.values() {
            assert!(
                !report.clean_log_tail().contains(path),
                "isolated environment path leaked: {path}"
            );
        }
        assert!(report.clean_log_tail().contains("<APPDATA>"));
        assert!(report.clean_log_tail().contains("<LOCALAPPDATA>"));
        assert_lease_released(&fixture);
    }

    #[test]
    fn native_primary_declared_only_as_locator_key_still_produces_crash_report() {
        let mut fixture = fixture("key-only-crash", false, false, None);
        let outcome = runtime().block_on(async {
            ProcessSupervisor::launch(
                fixture.lease.take().unwrap(),
                Arc::new(Trust(true)),
                None,
                bounded_options(),
            )
            .await
            .unwrap()
            .wait()
            .await
            .unwrap()
        });
        assert_eq!(outcome.primary_exit_code, 37);
        assert!(outcome.sanitized_crash_report.is_some());
        assert_lease_released(&fixture);
    }
}
