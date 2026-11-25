//! End-to-end test for a second independent portable program (XML config, persistent file+dir,
//! 2 build profiles, Creator Build -> Verify all pairs -> Export -> 2 HTTPS mirrors -> Player Install -> Update -> Rollback).
use super::*;
use crate::fusion_creator::{
    jobs::JobRunner, project_service, project_storage, CreatorJob, CreatorJobKind,
    CreatorJobStartRequest, CreatorJobStatus, CreatorPrivateBindings, CreatorReportKind,
    CreatorReportStatus,
};
use crate::fusion_source::{
    adapters::SecureHttpsTransport,
    canonical::QualifiedGameId,
    cas::CasStore,
    dto::{GameOptions, InstallRequest},
    fs_safety,
    program_schema::CreatorProgramProject,
    service::{FusionService, FusionStorage, NoProgress, ServiceEnvironment},
    signatures,
    test_support::https::HttpsFixture,
    workspace::WorkspaceAssembler,
};
use crate::storage::{fusion_source::SqliteFusionStorage, RepositoryStore};
use serde_json::json;
use std::{
    fs,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

fn compile_portable_binary(temp_root: &Path, out_exe: &Path, version_label: &str) {
    let stage = temp_root.join(format!("pe-compile-{version_label}"));
    fs::create_dir_all(&stage).unwrap();
    let src = stage.join("src.rs");
    let built = stage.join("notes.exe");
    let code = format!(
        r#"
use std::fs;
use std::path::Path;

fn main() {{
    let xml = fs::read_to_string("../userdata/config.xml").unwrap_or_default();
    let save_dir = Path::new("../userdata/saves");
    fs::create_dir_all(save_dir).unwrap();
    let state_path = save_dir.join("state.txt");
    let prev = fs::read_to_string(&state_path).unwrap_or_default();
    let prev_run: u32 = prev
        .split('|')
        .find_map(|part| part.strip_prefix("run="))
        .and_then(|num| num.parse().ok())
        .unwrap_or(0);
    let next_run = prev_run + 1;
    let payload = format!("run={{next_run}}|ver={version_label}|xml={{xml}}");
    fs::write(&state_path, payload.as_bytes()).unwrap();
    fs::write("../userdata/profile.dat", payload.as_bytes()).unwrap();
}}
"#
    );
    fs::write(&src, code).unwrap();
    let remap = format!("{}=app", stage.display());
    let output = Command::new("rustc")
        .arg("--edition")
        .arg("2021")
        .arg("-C")
        .arg("debuginfo=0")
        .arg("-C")
        .arg("strip=symbols")
        .arg("--remap-path-prefix")
        .arg(&remap)
        .arg(&src)
        .arg("-o")
        .arg(&built)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "rustc failed compiling portable fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::copy(&built, out_exe).unwrap();
    fs::remove_dir_all(&stage).unwrap();
}

fn portable_project_json(revision: u64, game_version: &str) -> serde_json::Value {
    json!({
        "schema_version": "1.1",
        "project_id": "notes-proj",
        "revision": revision,
        "title": "Notes Portable",
        "source_id": "notes-source",
        "game_id": "notes-portable",
        "game_version": game_version,
        "program_descriptor": {
            "document_type": "program_descriptor",
            "schema_version": "1.0",
            "identity": {
                "id": "notes-portable",
                "title": "Notes Portable",
                "version": game_version,
                "os": "windows",
                "architecture": "x86_64"
            },
            "executor": {"id": "fusion.declarative", "version": "1.0"},
            "components": [{
                "id": "app-bin",
                "layer_id": "runtime",
                "origin": {"kind": "managed", "artifact_ref": "app-bin", "redistribution_allowed": true},
                "source_root": "",
                "mount_path": "app"
            }],
            "required_files": [{"path": "app/notes.exe", "role": "main_executable"}],
            "projection_rules": [],
            "directories": ["userdata", "userdata/saves"],
            "entrypoint": {
                "executable": "app/notes.exe",
                "working_directory": "app",
                "arguments": [],
                "environment_paths": []
            },
            "configs": [{
                "id": "xml-main",
                "path": "userdata/config.xml",
                "format": "xml_attributes",
                "seed": {
                    "kind": "xml_root",
                    "root": "notes",
                    "attributes": [{"name": "schema", "value": 1}]
                },
                "preserve_unmanaged": true,
                "keys": [
                    {
                        "id": "mode",
                        "address": {"kind": "xml_attribute", "elements": ["ui"], "attribute": "mode"},
                        "scalar_type": "string",
                        "allowed_values": ["compact", "expanded"]
                    },
                    {
                        "id": "autosave",
                        "address": {"kind": "xml_attribute", "elements": ["storage"], "attribute": "autosave"},
                        "scalar_type": "boolean",
                        "allowed_values": [false, true]
                    }
                ]
            }],
            "persistence": [
                {
                    "path": "userdata/profile.dat",
                    "kind": "file",
                    "policy": "preserve_latest",
                    "export": "never"
                },
                {
                    "path": "userdata/saves",
                    "kind": "directory",
                    "policy": "preserve_latest",
                    "export": "never"
                }
            ],
            "build_profiles": [
                {
                    "id": "compact",
                    "title": "Compact UI",
                    "layer_ids": ["runtime"],
                    "controls_profile_ids": ["defaults"],
                    "settings": [
                        {"config_id": "xml-main", "key_id": "mode", "value": "compact"},
                        {"config_id": "xml-main", "key_id": "autosave", "value": false}
                    ]
                },
                {
                    "id": "expanded",
                    "title": "Expanded UI",
                    "layer_ids": ["runtime"],
                    "controls_profile_ids": ["defaults"],
                    "settings": [
                        {"config_id": "xml-main", "key_id": "mode", "value": "expanded"},
                        {"config_id": "xml-main", "key_id": "autosave", "value": true}
                    ]
                }
            ],
            "controls_profiles": [
                {"id": "defaults", "title": "Application defaults", "kind": "application_defaults"}
            ]
        }
    })
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

fn run_job(
    runner: &JobRunner,
    kind: CreatorJobKind,
    request: CreatorJobStartRequest,
) -> CreatorJob {
    let deadline = Instant::now() + Duration::from_secs(60);
    let receipt = loop {
        match runner.start(kind.clone(), request.clone()) {
            Ok(receipt) => break receipt,
            Err(err)
                if err.code == crate::fusion_source::ErrorCode::OperationInProgress
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(err) => panic!("failed starting creator job: {err:?}"),
        }
    };
    loop {
        let job = runner.get(&receipt.job_id).unwrap();
        if matches!(
            job.progress.status,
            CreatorJobStatus::Completed
                | CreatorJobStatus::Failed
                | CreatorJobStatus::Cancelled
                | CreatorJobStatus::Interrupted
        ) {
            return job;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {job:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct AuthorReleaseSpec<'a> {
    temp_root: &'a Path,
    revision: u64,
    version: &'a str,
    prev_revision: Option<u64>,
    prev_bindings_rev: Option<u64>,
    key_path: &'a Path,
    export_dir: &'a Path,
}

fn author_verify_and_export(
    runner: &JobRunner,
    store: &Arc<Mutex<RepositoryStore>>,
    spec: AuthorReleaseSpec<'_>,
) -> (String, String) {
    let runtime_dir = spec.temp_root.join(format!("author-{}", spec.version));
    fs::create_dir_all(&runtime_dir).unwrap();
    compile_portable_binary(spec.temp_root, &runtime_dir.join("notes.exe"), spec.version);

    let project_val = portable_project_json(spec.revision, spec.version);
    let _parsed: CreatorProgramProject = serde_json::from_value(project_val.clone()).unwrap();
    {
        let mut guard = store.lock().unwrap();
        project_storage::save_project_bytes(
            &mut guard,
            &serde_json::to_vec(&project_val).unwrap(),
            spec.prev_revision,
        )
        .unwrap();
        project_storage::save_private_bindings(
            &guard,
            spec.revision,
            spec.prev_bindings_rev,
            &CreatorPrivateBindings {
                project_id: "notes-proj".into(),
                component_paths: [("app-bin".into(), runtime_dir.to_string_lossy().into())].into(),
                signing_key_ref: Some(spec.key_path.to_string_lossy().into()),
            },
        )
        .unwrap();
    }

    let req = |release_id: Option<String>, build_id: &str, target: Option<&Path>| {
        CreatorJobStartRequest {
            project_id: "notes-proj".into(),
            project_revision: spec.revision,
            build_profile_ids: vec![build_id.into()],
            controls_profile_ids: vec!["defaults".into()],
            release_id,
            target_directory: target.map(|p| p.to_string_lossy().into()),
            source_url: None,
        }
    };

    let build = run_job(runner, CreatorJobKind::Build, req(None, "compact", None));
    assert_eq!(
        build.progress.status,
        CreatorJobStatus::Completed,
        "{build:?}"
    );
    let releases = project_service::release_list(&store.lock().unwrap(), "notes-proj").unwrap();
    let release = releases
        .into_iter()
        .find(|item| item.project_revision == spec.revision)
        .expect("saved release for revision");

    // Delete original author directory and signing key before clean Verify!
    fs::remove_dir_all(&runtime_dir).unwrap();
    fs::remove_file(spec.key_path).unwrap();

    for profile in ["compact", "expanded"] {
        let verify = run_job(
            runner,
            CreatorJobKind::Verify,
            req(Some(release.release_id.clone()), profile, None),
        );
        assert_eq!(
            verify.progress.status,
            CreatorJobStatus::Completed,
            "{verify:?}"
        );
        let reports: Vec<_> = {
            let guard = store.lock().unwrap();
            verify
                .report_ids
                .iter()
                .map(|id| project_service::report_get(&guard, id).unwrap())
                .collect()
        };
        assert!(reports.iter().any(|r| {
            r.kind == CreatorReportKind::StructureIntegrity
                && r.status == CreatorReportStatus::Passed
        }));
        assert!(reports.iter().any(|r| {
            r.kind == CreatorReportKind::CleanInstall && r.status == CreatorReportStatus::Passed
        }));
    }

    let export = run_job(
        runner,
        CreatorJobKind::Export,
        req(
            Some(release.release_id.clone()),
            "compact",
            Some(spec.export_dir),
        ),
    );
    assert_eq!(
        export.progress.status,
        CreatorJobStatus::Completed,
        "{export:?}"
    );
    let plain = fs_safety::list_plain_files_checked(spec.export_dir, &|| Ok(())).unwrap();
    for (rel, path) in plain {
        let bytes = fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("author-") && !text.contains("-----BEGIN"),
            "export file {rel} leaked private author data"
        );
    }
    (release.release_id, release.manifest_blake3)
}

#[test]
fn second_portable_program_xml_and_persistence_completes_full_lifecycle() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let key_path = root.join("notes-signing.pem");
    signatures::keygen(&key_path).unwrap();
    let pub_key_path = root.join("notes-signing-reuse.pem");
    fs::copy(&key_path, &pub_key_path).unwrap();

    let store = Arc::new(Mutex::new(
        RepositoryStore::open(&root.join("creator.sqlite")).unwrap(),
    ));
    let runner =
        production_runner(Arc::clone(&store), &root.join("creator"), Arc::new(|_| {})).unwrap();

    let export_v1 = root.join("export-v1");
    let (release_v1_id, manifest_v1_blake3) = author_verify_and_export(
        &runner,
        &store,
        AuthorReleaseSpec {
            temp_root: root,
            revision: 0,
            version: "1.0.0",
            prev_revision: None,
            prev_bindings_rev: None,
            key_path: &key_path,
            export_dir: &export_v1,
        },
    );

    // Verify HTTPS acquisition from two distinct HTTPS base URLs with empty caches.
    let host_a = root.join("host-a");
    let host_b = root.join("host-b");
    copy_dir_tree(&export_v1, &host_a.join("mirror-a"));
    copy_dir_tree(&export_v1, &host_b.join("nested/mirror-b"));
    let server_a = HttpsFixture::start(&host_a);
    let server_b = HttpsFixture::start(&host_b);
    let url_a = server_a.url("mirror-a/source-index.json");
    let url_b = server_b.url("nested/mirror-b/source-index.json");

    for (url, profile) in [(&url_a, "compact"), (&url_b, "expanded")] {
        let job = run_job(
            &runner,
            CreatorJobKind::VerifyHttps,
            CreatorJobStartRequest {
                project_id: "notes-proj".into(),
                project_revision: 0,
                build_profile_ids: vec![profile.into()],
                controls_profile_ids: vec!["defaults".into()],
                release_id: Some(release_v1_id.clone()),
                target_directory: None,
                source_url: Some(url.clone()),
            },
        );
        assert_eq!(job.progress.status, CreatorJobStatus::Completed, "{job:?}");
        let report =
            project_service::report_get(&store.lock().unwrap(), &job.report_ids[0]).unwrap();
        assert_eq!(report.kind, CreatorReportKind::HttpsAcquisition);
        assert_eq!(report.status, CreatorReportStatus::Passed);
        assert_eq!(report.release_blake3, manifest_v1_blake3);
    }

    // Check downloaded HTTPS manifest + CAS bytes match across both mirrors in fresh player libraries.
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut downloaded_manifests = Vec::new();
        for url in [&url_a, &url_b] {
            let player_root = tempfile::tempdir().unwrap();
            let lib = player_root.path().join("lib");
            let appdata = player_root.path().join("appdata");
            let local = player_root.path().join("local");
            for d in [&lib, &appdata, &local] {
                fs::create_dir(d).unwrap();
            }
            let p_store = RepositoryStore::open(&player_root.path().join("player.sqlite")).unwrap();
            let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(p_store))));
            let service = FusionService::open(
                &lib,
                storage.clone(),
                Arc::new(SecureHttpsTransport),
                Arc::new(NoProgress),
                ServiceEnvironment {
                    appdata,
                    local_appdata: local,
                    display_width: 1280,
                    display_height: 720,
                    refresh_rate: 60,
                },
            )
            .unwrap();
            service.source_add(url).await.unwrap();
            let game = QualifiedGameId {
                source_id: "notes-source".into(),
                game_id: "notes-portable".into(),
            };
            service
                .game_set_options(
                    &game,
                    GameOptions {
                        build_profile_id: Some("expanded".into()),
                        controls_profile_id: Some("defaults".into()),
                        ..GameOptions::default()
                    },
                )
                .await
                .unwrap();
            service
                .game_install(&game, InstallRequest::default())
                .await
                .unwrap();
            let record = storage
                .get_manifest(&game, &manifest_v1_blake3)
                .unwrap()
                .unwrap();
            assert_eq!(
                blake3::hash(&record.raw_bytes).to_hex().as_str(),
                manifest_v1_blake3
            );
            downloaded_manifests.push(record.raw_bytes);
            CasStore::open(&lib)
                .unwrap()
                .verify(
                    serde_json::from_slice::<serde_json::Value>(
                        &fs::read(export_v1.join(format!(
                            "manifests/notes-portable-{manifest_v1_blake3}.artifacts.json"
                        )))
                        .unwrap(),
                    )
                    .unwrap()["artifacts"][0]["cas_digest"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
        }
        assert_eq!(downloaded_manifests[0], downloaded_manifests[1]);
    });

    // Author v2 with same signing key, verify, export, and run full player install -> launch -> update -> launch -> rollback -> launch!
    let export_v2 = root.join("export-v2");
    let (_release_v2_id, _manifest_v2_blake3) = author_verify_and_export(
        &runner,
        &store,
        AuthorReleaseSpec {
            temp_root: root,
            revision: 1,
            version: "1.1.0",
            prev_revision: Some(0),
            prev_bindings_rev: Some(0),
            key_path: &pub_key_path,
            export_dir: &export_v2,
        },
    );

    let live_host = root.join("live-host");
    copy_dir_tree(&export_v1, &live_host.join("notes"));
    let live_server = HttpsFixture::start(&live_host);
    let live_url = live_server.url("notes/source-index.json");

    rt.block_on(async {
        let player_root = tempfile::tempdir().unwrap();
        let lib = player_root.path().join("lib");
        let appdata = player_root.path().join("appdata");
        let local = player_root.path().join("local");
        for d in [&lib, &appdata, &local] {
            fs::create_dir(d).unwrap();
        }
        let p_store = RepositoryStore::open(&player_root.path().join("player.sqlite")).unwrap();
        let storage = Arc::new(SqliteFusionStorage::new(Arc::new(Mutex::new(p_store))));
        let service = FusionService::open(
            &lib,
            storage,
            Arc::new(SecureHttpsTransport),
            Arc::new(NoProgress),
            ServiceEnvironment {
                appdata,
                local_appdata: local,
                display_width: 1280,
                display_height: 720,
                refresh_rate: 60,
            },
        )
        .unwrap();
        service.source_add(&live_url).await.unwrap();
        let game = QualifiedGameId {
            source_id: "notes-source".into(),
            game_id: "notes-portable".into(),
        };
        service
            .game_set_options(
                &game,
                GameOptions {
                    build_profile_id: Some("expanded".into()),
                    controls_profile_id: Some("defaults".into()),
                    ..GameOptions::default()
                },
            )
            .await
            .unwrap();
        service
            .game_install(&game, InstallRequest::default())
            .await
            .unwrap();

        let state1 = service.game_get_state(&game).await.unwrap();
        assert_eq!(state1.active_version.as_deref(), Some("1.0.0"));
        let ws1 = WorkspaceAssembler::open(&lib, &game)
            .unwrap()
            .active()
            .unwrap()
            .unwrap()
            .snapshot
            .workspace_root;
        let xml1 = fs::read_to_string(ws1.join("userdata/config.xml")).unwrap();
        assert!(xml1.contains("mode=\"expanded\""), "{xml1}");
        assert!(xml1.contains("autosave=\"true\""), "{xml1}");

        service
            .game_trust_executables(
                &game,
                state1
                    .executables
                    .iter()
                    .map(|e| (e.path.clone(), e.blake3.clone()))
                    .collect(),
            )
            .await
            .unwrap();
        service.game_launch(&game).await.unwrap();

        let save1 = fs::read_to_string(ws1.join("userdata/saves/state.txt")).unwrap();
        let profile1 = fs::read_to_string(ws1.join("userdata/profile.dat")).unwrap();
        assert!(save1.starts_with("run=1|ver=1.0.0|xml="), "{save1}");
        assert_eq!(save1, profile1);

        // Publish v2 onto the HTTPS server, refresh, update, trust, and launch v2!
        fs::remove_dir_all(live_host.join("notes")).unwrap();
        copy_dir_tree(&export_v2, &live_host.join("notes"));
        service.source_refresh("notes-source").await.unwrap();
        let pre_update = service.game_get_state(&game).await.unwrap();
        assert!(pre_update.update_available);
        service
            .game_update(&game, InstallRequest::default())
            .await
            .unwrap();

        let state2 = service.game_get_state(&game).await.unwrap();
        assert_eq!(state2.active_version.as_deref(), Some("1.1.0"));
        service
            .game_trust_executables(
                &game,
                state2
                    .executables
                    .iter()
                    .map(|e| (e.path.clone(), e.blake3.clone()))
                    .collect(),
            )
            .await
            .unwrap();
        service.game_launch(&game).await.unwrap();

        let ws2 = WorkspaceAssembler::open(&lib, &game)
            .unwrap()
            .active()
            .unwrap()
            .unwrap()
            .snapshot
            .workspace_root;
        let save2 = fs::read_to_string(ws2.join("userdata/saves/state.txt")).unwrap();
        let profile2 = fs::read_to_string(ws2.join("userdata/profile.dat")).unwrap();
        assert!(save2.starts_with("run=2|ver=1.1.0|xml="), "{save2}");
        assert_eq!(save2, profile2);

        // Rollback to v1 while preserving latest v2 user data!
        let _rolled_back = service.game_rollback(&game).await.unwrap();
        let state_rb = service.game_get_state(&game).await.unwrap();
        assert_eq!(state_rb.active_version.as_deref(), Some("1.0.0"));
        let ws_rb = WorkspaceAssembler::open(&lib, &game)
            .unwrap()
            .active()
            .unwrap()
            .unwrap()
            .snapshot
            .workspace_root;
        let rb_save = fs::read_to_string(ws_rb.join("userdata/saves/state.txt")).unwrap();
        let rb_profile = fs::read_to_string(ws_rb.join("userdata/profile.dat")).unwrap();
        assert_eq!(
            rb_save, save2,
            "rollback must preserve latest post-update dir save"
        );
        assert_eq!(
            rb_profile, profile2,
            "rollback must preserve latest post-update file save"
        );

        service.game_launch(&game).await.unwrap();
        let rb_after_launch = fs::read_to_string(ws_rb.join("userdata/saves/state.txt")).unwrap();
        assert!(
            rb_after_launch.starts_with("run=3|ver=1.0.0|xml="),
            "{rb_after_launch}"
        );
    });
}
