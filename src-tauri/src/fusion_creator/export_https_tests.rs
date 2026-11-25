use super::*;
use crate::fusion_creator::{
    project_service, project_storage, CreatorCheckpointBoundary, CreatorJobKind,
    CreatorJobStartRequest, CreatorJobStatus, CreatorPrivateBindings, CreatorRelease,
    CreatorReportKind, CreatorReportStatus,
};
use crate::fusion_source::{
    adapters::SecureHttpsTransport,
    canonical::QualifiedGameId,
    cas::hash_file,
    dto::{GameOptions, InstallRequest},
    fs_safety,
    offline::CREATOR_SOURCE_ROOT,
    packer::{PackerEngine, PackerRecipe},
    schema::{DigestAlgorithm, SourceIndex},
    service::{FusionService, NoProgress, ServiceEnvironment},
    signatures,
    test_support::https::HttpsFixture,
    ErrorCode,
};
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

struct PortableFixture {
    _dir: tempfile::TempDir,
    runner: JobRunner,
    runtime_dir: std::path::PathBuf,
    key_path: std::path::PathBuf,
    project: crate::fusion_source::program_schema::CreatorProgramProject,
}

impl PortableFixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let runtime_dir = dir.path().join("author-runtime");
        fs::create_dir(&runtime_dir).unwrap();
        fs::write(runtime_dir.join("app.exe"), b"MZ-portable-app-binary").unwrap();
        fs::write(
            runtime_dir.join("data.pak"),
            b"deterministic-pak-bytes-123456",
        )
        .unwrap();
        let key_path = dir.path().join("author-signing.pem");
        signatures::keygen(&key_path).unwrap();

        let mut store = RepositoryStore::open(&dir.path().join("creator.sqlite")).unwrap();
        let project_val = json!({
            "schema_version": "1.1",
            "project_id": "portable-proj",
            "revision": 1,
            "title": "Portable App",
            "source_id": "portable-source",
            "game_id": "portable-game",
            "game_version": "1.2.0",
            "program_descriptor": {
                "document_type": "program_descriptor",
                "schema_version": "1.0",
                "identity": {
                    "id": "portable-game",
                    "title": "Portable App",
                    "version": "1.2.0",
                    "os": "windows",
                    "architecture": "x86_64"
                },
                "executor": {"id": "fusion.declarative", "version": "1.0"},
                "components": [{
                    "id": "program",
                    "layer_id": "runtime",
                    "origin": {"kind": "managed", "artifact_ref": "program", "redistribution_allowed": true},
                    "source_root": "",
                    "mount_path": "app"
                }],
                "required_files": [
                    {"path": "app/app.exe", "role": "main_executable"},
                    {"path": "app/data.pak", "role": "data"}
                ],
                "projection_rules": [],
                "directories": ["userdata"],
                "entrypoint": {
                    "executable": "app/app.exe",
                    "working_directory": "app",
                    "arguments": [],
                    "environment_paths": []
                },
                "configs": [{
                    "id": "main",
                    "path": "userdata/settings.ini",
                    "format": "ini",
                    "seed": {"kind": "empty_ini"},
                    "preserve_unmanaged": true,
                    "keys": [
                        {
                            "id": "fullscreen",
                            "address": {"kind": "ini", "section": "Video", "key": "fullscreen"},
                            "scalar_type": "boolean",
                            "allowed_values": [false, true]
                        },
                        {
                            "id": "preset",
                            "address": {"kind": "ini", "section": "Video", "key": "preset"},
                            "scalar_type": "string",
                            "allowed_values": ["low", "ultra"]
                        }
                    ]
                }],
                "persistence": [],
                "build_profiles": [
                    {
                        "id": "windowed",
                        "title": "Windowed",
                        "layer_ids": ["runtime"],
                        "controls_profile_ids": ["native"],
                        "settings": [
                            {"config_id": "main", "key_id": "fullscreen", "value": false},
                            {"config_id": "main", "key_id": "preset", "value": "low"}
                        ]
                    },
                    {
                        "id": "fullscreen",
                        "title": "Fullscreen",
                        "layer_ids": ["runtime"],
                        "controls_profile_ids": ["native"],
                        "settings": [
                            {"config_id": "main", "key_id": "fullscreen", "value": true},
                            {"config_id": "main", "key_id": "preset", "value": "ultra"}
                        ]
                    }
                ],
                "controls_profiles": [
                    {"id": "native", "title": "Application defaults", "kind": "application_defaults"}
                ]
            }
        });
        let project: crate::fusion_source::program_schema::CreatorProgramProject =
            serde_json::from_value(project_val.clone()).unwrap();
        project_storage::save_project_bytes(
            &mut store,
            &serde_json::to_vec(&project_val).unwrap(),
            None,
        )
        .unwrap();
        project_storage::save_private_bindings(
            &store,
            1,
            None,
            &CreatorPrivateBindings {
                project_id: "portable-proj".into(),
                component_paths: [("program".into(), runtime_dir.to_string_lossy().into())].into(),
                signing_key_ref: Some(key_path.to_string_lossy().into()),
            },
        )
        .unwrap();
        let runner = production_runner(
            Arc::new(Mutex::new(store)),
            &dir.path().join("creator"),
            Arc::new(|_| {}),
        )
        .unwrap();
        Self {
            _dir: dir,
            runner,
            runtime_dir,
            key_path,
            project,
        }
    }

    fn start_retry(
        &self,
        kind: CreatorJobKind,
        request: CreatorJobStartRequest,
    ) -> Result<crate::fusion_creator::CreatorJobReceipt, FusionError> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match self.runner.start(kind.clone(), request.clone()) {
                Ok(receipt) => return Ok(receipt),
                Err(err)
                    if err.code == ErrorCode::OperationInProgress && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn run(
        &self,
        kind: CreatorJobKind,
        request: CreatorJobStartRequest,
    ) -> crate::fusion_creator::CreatorJob {
        let receipt = self.start_retry(kind, request).unwrap();
        let deadline = Instant::now() + Duration::from_secs(45);
        loop {
            let job = self.runner.get(&receipt.job_id).unwrap();
            if matches!(
                job.progress.status,
                CreatorJobStatus::Completed
                    | CreatorJobStatus::Failed
                    | CreatorJobStatus::Cancelled
                    | CreatorJobStatus::Interrupted
            ) {
                return job;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for job {:?}",
                job
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn req(
        &self,
        release_id: Option<String>,
        build_id: &str,
        target: Option<&Path>,
        source_url: Option<&str>,
    ) -> CreatorJobStartRequest {
        CreatorJobStartRequest {
            project_id: "portable-proj".into(),
            project_revision: 1,
            build_profile_ids: vec![build_id.into()],
            controls_profile_ids: vec!["native".into()],
            release_id,
            target_directory: target.map(|p| p.to_string_lossy().into()),
            source_url: source_url.map(str::to_owned),
        }
    }
}

fn copy_dir_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_dir_tree(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap();
        }
    }
}

#[test]
fn portable_release_verify_export_and_multi_origin_https_roundtrip() {
    let f = PortableFixture::new();
    let built = f.run(CreatorJobKind::Build, f.req(None, "windowed", None, None));
    assert_eq!(built.progress.status, CreatorJobStatus::Completed);
    let release_id = built.progress.release_id.clone().unwrap();
    let store = f.runner.store();
    let release = project_service::release_list(&store.lock().unwrap(), "portable-proj")
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(release.release_id, release_id);

    // Delete original author directory and signing key BEFORE Verification & Export.
    fs::remove_dir_all(&f.runtime_dir).unwrap();
    fs::remove_file(&f.key_path).unwrap();

    // Gate 1: Export before verification must be refused.
    let blocked_dir = f._dir.path().join("blocked-export");
    let blocked = f.run(
        CreatorJobKind::Export,
        f.req(
            Some(release_id.clone()),
            "windowed",
            Some(&blocked_dir),
            None,
        ),
    );
    assert_eq!(blocked.progress.status, CreatorJobStatus::Failed);
    let blocked_err = blocked.progress.error.unwrap();
    assert_eq!(blocked_err.code, ErrorCode::ActionUnavailable);
    assert_eq!(blocked_err.details["reason"], "verification_required");
    assert!(!blocked_dir.exists());

    // Gate 2: Verify only pair 1 ("windowed/native"); Export still refuses missing pair 2.
    let verify_1 = f.run(
        CreatorJobKind::Verify,
        f.req(Some(release_id.clone()), "windowed", None, None),
    );
    assert_eq!(verify_1.progress.status, CreatorJobStatus::Completed);
    let blocked_partial = f.run(
        CreatorJobKind::Export,
        f.req(
            Some(release_id.clone()),
            "windowed",
            Some(&blocked_dir),
            None,
        ),
    );
    assert_eq!(blocked_partial.progress.status, CreatorJobStatus::Failed);
    assert_eq!(
        blocked_partial.progress.error.unwrap().details["missingPairs"],
        json!(["fullscreen/native"])
    );
    assert!(!blocked_dir.exists());

    // Verify pair 2 ("fullscreen/native").
    let verify_2 = f.run(
        CreatorJobKind::Verify,
        f.req(Some(release_id.clone()), "fullscreen", None, None),
    );
    assert_eq!(verify_2.progress.status, CreatorJobStatus::Completed);

    // Gate 3: Non-empty export folder refused cleanly without touching existing files.
    let occupied_dir = f._dir.path().join("occupied");
    fs::create_dir(&occupied_dir).unwrap();
    fs::write(occupied_dir.join("keep.txt"), b"must survive").unwrap();
    let refused_non_empty = f.run(
        CreatorJobKind::Export,
        f.req(
            Some(release_id.clone()),
            "windowed",
            Some(&occupied_dir),
            None,
        ),
    );
    assert_eq!(refused_non_empty.progress.status, CreatorJobStatus::Failed);
    assert_eq!(
        refused_non_empty.progress.error.unwrap().code,
        ErrorCode::SnapshotConflict
    );
    assert_eq!(
        fs::read(occupied_dir.join("keep.txt")).unwrap(),
        b"must survive"
    );

    // Gate 4: Overlapping target inside creator workspace/CAS rejected before starting job.
    let overlap_err = f
        .start_retry(
            CreatorJobKind::Export,
            f.req(
                Some(release_id.clone()),
                "windowed",
                Some(&f._dir.path().join("creator/export-inside-cas-root")),
                None,
            ),
        )
        .unwrap_err();
    assert_eq!(overlap_err.code, ErrorCode::InvalidPath);

    // Gate 5: Export into clean empty target directory succeeds atomically and keeps exact allowlist.
    let export_dir = f._dir.path().join("published-export");
    let exported = f.run(
        CreatorJobKind::Export,
        f.req(
            Some(release_id.clone()),
            "windowed",
            Some(&export_dir),
            None,
        ),
    );
    assert_eq!(exported.progress.status, CreatorJobStatus::Completed);
    assert_eq!(exported.checkpoints.len(), 1);
    assert_eq!(
        exported.checkpoints[0].boundary,
        CreatorCheckpointBoundary::ExportCompleted
    );

    let plain_files: BTreeMap<String, String> = fs_safety::list_plain_files(&export_dir)
        .unwrap()
        .into_iter()
        .map(|(rel, path)| (rel, hash_file(&path, DigestAlgorithm::Blake3).unwrap()))
        .collect();
    let (zip_name, zip_hash) = release
        .artifact_hashes
        .iter()
        .find(|(k, _)| k.ends_with(".zip"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .unwrap();
    let expected_names: BTreeSet<String> = BTreeSet::from([
        "source-index.json".into(),
        format!("manifests/portable-game-{}.json", release.manifest_blake3),
        format!(
            "manifests/portable-game-{}.artifacts.json",
            release.manifest_blake3
        ),
        format!("artifacts/{zip_name}"),
    ]);
    assert_eq!(
        plain_files.keys().cloned().collect::<BTreeSet<_>>(),
        expected_names
    );
    assert_eq!(plain_files[&format!("artifacts/{zip_name}")], zip_hash);

    let index_bytes = fs::read(export_dir.join("source-index.json")).unwrap();
    let index: SourceIndex = serde_json::from_slice(&index_bytes).unwrap();
    assert_eq!(
        index.games[0].manifest_url,
        format!("manifests/portable-game-{}.json", release.manifest_blake3)
    );
    let manifest_bytes = fs::read(export_dir.join(format!(
        "manifests/portable-game-{}.json",
        release.manifest_blake3
    )))
    .unwrap();
    let manifest_text = std::str::from_utf8(&manifest_bytes).unwrap();
    assert!(manifest_text.contains(&format!("\"artifacts/{zip_name}\"")));
    assert!(!manifest_text.contains("offline.fusion.invalid"));
    assert!(!manifest_text.contains("author-runtime"));
    assert!(!manifest_text.contains("author-signing"));
    let audit = PackerEngine::new().verify_audit(&export_dir).unwrap();
    assert!(audit.report.signature_verified);
    assert_eq!(audit.report.artifacts_verified, 1);

    // Mount the EXACT exported bytes (unmodified!) under two distinct HTTPS base URLs.
    let host_a = f._dir.path().join("host-a");
    let host_b = f._dir.path().join("host-b");
    copy_dir_tree(&export_dir, &host_a.join("origin-a"));
    copy_dir_tree(&export_dir, &host_b.join("nested/origin-b"));
    let server_a = HttpsFixture::start(&host_a);
    let server_b = HttpsFixture::start(&host_b);
    let url_a = server_a.url("origin-a/source-index.json");
    let url_b = server_b.url("nested/origin-b/source-index.json");

    // VerifyHttps passes on BOTH HTTPS origins from empty caches & writes HttpsAcquisition reports.
    for (url, profile) in [(&url_a, "windowed"), (&url_b, "fullscreen")] {
        let https_job = f.run(
            CreatorJobKind::VerifyHttps,
            f.req(Some(release_id.clone()), profile, None, Some(url)),
        );
        assert_eq!(
            https_job.progress.status,
            CreatorJobStatus::Completed,
            "{https_job:?}"
        );
        assert_eq!(https_job.report_ids.len(), 1);
        let report = project_service::report_get(
            &f.runner.store().lock().unwrap(),
            &https_job.report_ids[0],
        )
        .unwrap();
        assert_eq!(report.kind, CreatorReportKind::HttpsAcquisition);
        assert_eq!(report.status, CreatorReportStatus::Passed);
        assert_eq!(report.release_blake3, release.manifest_blake3);
        assert_eq!(report.environment["source_url"], *url);
    }

    // Player installs the same release from both HTTPS base URLs into fresh empty libraries.
    use crate::fusion_source::{cas::CasStore, service::FusionStorage};
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut downloaded_manifests = Vec::new();
        for url in [&url_a, &url_b] {
            let player_root = tempfile::tempdir().unwrap();
            let lib = player_root.path().join("lib");
            let appdata = player_root.path().join("appdata");
            let local = player_root.path().join("local");
            for dir in [&lib, &appdata, &local] {
                fs::create_dir(dir).unwrap();
            }
            let player_store =
                RepositoryStore::open(&player_root.path().join("player.sqlite")).unwrap();
            let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(player_store))));
            let service = FusionService::open(
                &lib,
                storage.clone(),
                Arc::new(SecureHttpsTransport),
                Arc::new(NoProgress),
                ServiceEnvironment {
                    appdata,
                    local_appdata: local,
                    display_width: 1920,
                    display_height: 1080,
                    refresh_rate: 60,
                },
            )
            .unwrap();
            service.source_add(url).await.unwrap();
            let game = QualifiedGameId {
                source_id: "portable-source".into(),
                game_id: "portable-game".into(),
            };
            service
                .game_set_options(
                    &game,
                    GameOptions {
                        build_profile_id: Some("fullscreen".into()),
                        controls_profile_id: Some("native".into()),
                        ..GameOptions::default()
                    },
                )
                .await
                .unwrap();
            let res = service
                .game_install(&game, InstallRequest::default())
                .await
                .unwrap();
            assert_eq!(res.status, "completed");
            let state = service.game_get_state(&game).await.unwrap();
            assert!(state.installed);
            let snap_id = state.active_snapshot_id.unwrap();
            let applied = state.applied_options.unwrap();
            assert_eq!(applied.build_profile_id.as_deref(), Some("fullscreen"));
            assert_eq!(applied.controls_profile_id.as_deref(), Some("native"));
            let lease = crate::fusion_source::workspace::WorkspaceAssembler::open(&lib, &game)
                .unwrap()
                .active()
                .unwrap()
                .unwrap();
            assert_eq!(lease.snapshot.id, snap_id);
            let ini =
                fs::read_to_string(lease.snapshot.workspace_root.join("userdata/settings.ini"))
                    .unwrap();
            let expected_ini = crate::fusion_creator::declarative::codecs::render_config(
                &f.project.program_descriptor.configs[0],
                None,
                &f.project.program_descriptor.build_profiles[1].settings,
            )
            .unwrap();
            assert_eq!(ini, expected_ini);
            assert!(ini.contains("fullscreen=true"), "{ini}");
            assert!(ini.contains("preset=\"ultra\""), "{ini}");
            let cas = CasStore::open(&lib).unwrap();
            let inv_json: serde_json::Value = serde_json::from_slice(
                &fs::read(export_dir.join(format!(
                    "manifests/portable-game-{}.artifacts.json",
                    release.manifest_blake3
                )))
                .unwrap(),
            )
            .unwrap();
            cas.verify(inv_json["artifacts"][0]["cas_digest"].as_str().unwrap())
                .unwrap();
            let record = storage
                .get_manifest(&game, &release.manifest_blake3)
                .unwrap()
                .unwrap();
            assert_eq!(
                blake3::hash(&record.raw_bytes).to_hex().as_str(),
                release.manifest_blake3
            );
            downloaded_manifests.push(record.raw_bytes);
        }
        assert_eq!(downloaded_manifests[0], downloaded_manifests[1]);
    });

    // Tamper 1 byte of the hosted ZIP on server_a -> VerifyHttps fails closed and records Failed report.
    let hosted_zip = host_a.join("origin-a/artifacts").join(&zip_name);
    let mut tampered = fs::read(&hosted_zip).unwrap();
    tampered[0] ^= 0xff;
    fs::write(&hosted_zip, tampered).unwrap();
    let fail_https = f.run(
        CreatorJobKind::VerifyHttps,
        f.req(Some(release_id), "windowed", None, Some(&url_a)),
    );
    assert_eq!(fail_https.progress.status, CreatorJobStatus::Failed);
    assert_eq!(fail_https.report_ids.len(), 1);
    let fail_report =
        project_service::report_get(&f.runner.store().lock().unwrap(), &fail_https.report_ids[0])
            .unwrap();
    assert_eq!(fail_report.kind, CreatorReportKind::HttpsAcquisition);
    assert_eq!(fail_report.status, CreatorReportStatus::Failed);
}

#[test]
fn legacy_offline_release_verifies_locally_and_refuses_export_and_https() {
    let f = PortableFixture::new();
    let pub_key = base64::engine::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        signatures::read_signing_key(&f.key_path)
            .unwrap()
            .verifying_key()
            .as_bytes(),
    );
    let legacy_recipe: PackerRecipe = serde_json::from_value(json!({
        "source_info": {"id": f.project.source_id, "name": f.project.title, "public_key": pub_key},
        "base_url": CREATOR_SOURCE_ROOT,
        "games": [{
            "id": f.project.game_id,
            "title": f.project.title,
            "version": f.project.game_version,
            "layers": [{"id": "runtime", "kind": "runtime", "local_path": &f.runtime_dir, "archive": "zip"}],
            "program_descriptor": f.project.program_descriptor
        }]
    }))
    .unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let pub_dir = scratch.path().join("pub");
    let built = PackerEngine::new()
        .build_checked(
            &legacy_recipe,
            scratch.path(),
            &pub_dir,
            Some(&f.key_path),
            &|| Ok(()),
        )
        .unwrap();
    let audit = PackerEngine::new().verify_audit(&pub_dir).unwrap();
    let raw_index = fs::read(&built.source_index).unwrap();
    let index: SourceIndex = serde_json::from_slice(&raw_index).unwrap();
    let manifest_blake3 = index.games[0].manifest_blake3.clone();
    let stem = format!("manifests/{}-{}", f.project.game_id, manifest_blake3);
    let raw_manifest = fs::read(pub_dir.join(format!("{stem}.json"))).unwrap();
    let inv_bytes = fs::read(pub_dir.join(format!("{stem}.artifacts.json"))).unwrap();
    let zip_id = format!("{}.zip", built.artifacts[0].transport_digest.value);

    // Stage into a release receipt inside Creator's runner workspace just like an earlier build.
    let ws = f
        ._dir
        .path()
        .join("creator")
        .join("workspaces")
        .join(blake3::hash(b"portable-proj").to_hex().as_str());
    fs::create_dir_all(&ws).unwrap();
    let stage = ws.join("legacy-stage");
    fs::create_dir_all(stage.join("artifacts")).unwrap();
    fs::write(stage.join("manifest.json"), &raw_manifest).unwrap();
    fs::write(stage.join("artifacts/source-index.json"), &raw_index).unwrap();
    fs::write(stage.join("artifacts/artifact-inventory.json"), &inv_bytes).unwrap();
    fs::copy(
        pub_dir.join("artifacts").join(&zip_id),
        stage.join("artifacts").join(&zip_id),
    )
    .unwrap();

    let legacy_release_id = "release-legacy-offline-0001".to_owned();
    let release = CreatorRelease {
        release_id: legacy_release_id.clone(),
        project_id: "portable-proj".into(),
        project_revision: 1,
        manifest_blake3: manifest_blake3.clone(),
        artifact_hashes: BTreeMap::from([
            (
                "source-index.json".into(),
                blake3::hash(&raw_index).to_hex().to_string(),
            ),
            (
                "artifact-inventory.json".into(),
                audit
                    .inventory_blake3(&f.project.game_id, &manifest_blake3)
                    .unwrap()
                    .into(),
            ),
            (
                zip_id.clone(),
                built.artifacts[0].transport_digest.value.clone(),
            ),
        ]),
        report_ids: vec![],
    };
    let receipt_bytes = serde_json::to_vec(&serde_json::json!({
        "release_id": legacy_release_id,
        "release_blake3": blake3::hash(&serde_json::to_vec(&release).unwrap()).to_hex().to_string(),
        "bundle_blake3": crate::fusion_source::cas::CasStore::open(&f._dir.path().join("creator"))
            .unwrap()
            .import_directory(&stage)
            .unwrap()
            .digest
    }))
    .unwrap();
    fs::remove_dir_all(&stage).unwrap();

    let job_id = "job-legacy-build".to_owned();
    let cp_id = format!(
        "checkpoint-release-{}",
        blake3::hash(legacy_release_id.as_bytes()).to_hex()
    );
    let stamp = json!({
        "projectRevision": 1,
        "inputHashes": {},
        "executor": {"id": "fusion.declarative", "version": "1.0"}
    });
    let job_state = crate::fusion_creator::CreatorJob {
        progress: crate::fusion_creator::CreatorJobProgress {
            job_id: job_id.clone(),
            project_id: "portable-proj".into(),
            project_revision: 1,
            sequence: 2,
            kind: CreatorJobKind::Build,
            status: CreatorJobStatus::Completed,
            phase: "completed".into(),
            completed_units: 0,
            total_units: None,
            checkpoint_id: Some(cp_id.clone()),
            release_id: Some(legacy_release_id.clone()),
            error: None,
        },
        checkpoints: vec![crate::fusion_creator::CreatorCheckpoint {
            checkpoint_id: cp_id.clone(),
            job_id: job_id.clone(),
            boundary: CreatorCheckpointBoundary::ReleaseBuilt,
            stamp: serde_json::from_value(stamp.clone()).unwrap(),
            result_blake3: blake3::hash(&receipt_bytes).to_hex().to_string(),
            artifact_id: None,
        }],
        report_ids: vec![],
    };
    {
        let store = f.runner.store();
        let conn = &store.lock().unwrap().conn;
        conn.execute(
            "INSERT INTO creator_releases(release_id,project_id,project_revision,release_json) VALUES(?1,'portable-proj',1,?2)",
            rusqlite::params![legacy_release_id, serde_json::to_vec(&release).unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO creator_jobs(job_id,project_id,project_revision,workspace,status,run_token,state_json,request_json,stamp_json) VALUES(?1,'portable-proj',1,?2,'completed','tok',?3,?4,?5)",
            rusqlite::params![
                job_id,
                ws.to_string_lossy(),
                serde_json::to_vec(&job_state).unwrap(),
                serde_json::to_vec(&f.req(None, "windowed", None, None)).unwrap(),
                serde_json::to_vec(&stamp).unwrap(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO creator_checkpoint_results(checkpoint_id,job_id,evidence) VALUES(?1,?2,?3)",
            rusqlite::params![cp_id, job_id, receipt_bytes],
        )
        .unwrap();
    }

    // Local Verify MUST still succeed on both pairs without mutating legacy URLs.
    for profile in ["windowed", "fullscreen"] {
        let verified = f.run(
            CreatorJobKind::Verify,
            f.req(Some(legacy_release_id.clone()), profile, None, None),
        );
        assert_eq!(verified.progress.status, CreatorJobStatus::Completed);
    }

    // Export refuses cleanly without re-signing or altering the legacy release.
    let export_dir = f._dir.path().join("legacy-export");
    let exported = f.run(
        CreatorJobKind::Export,
        f.req(
            Some(legacy_release_id.clone()),
            "windowed",
            Some(&export_dir),
            None,
        ),
    );
    assert_eq!(exported.progress.status, CreatorJobStatus::Failed);
    let err = exported.progress.error.unwrap();
    assert_eq!(err.code, ErrorCode::InvalidUri);
    assert_eq!(err.details["reason"], "legacy_offline_release_urls");
    assert!(!export_dir.exists());

    // VerifyHttps also refuses cleanly without mutating the release.
    let server = HttpsFixture::start(f._dir.path());
    let https = f.run(
        CreatorJobKind::VerifyHttps,
        f.req(
            Some(legacy_release_id.clone()),
            "windowed",
            None,
            Some(&server.url("source-index.json")),
        ),
    );
    assert_eq!(https.progress.status, CreatorJobStatus::Failed);
    assert_eq!(https.progress.error.unwrap().code, ErrorCode::InvalidUri);

    let after = project_service::release_list(&f.runner.store().lock().unwrap(), "portable-proj")
        .unwrap()
        .into_iter()
        .find(|r| r.release_id == legacy_release_id)
        .unwrap();
    assert_eq!(after.manifest_blake3, manifest_blake3);
}
