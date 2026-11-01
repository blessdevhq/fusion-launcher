//! Foundation H: real desktop transport + existing Packer/service/SQLite wiring.
//! No transport trait is implemented here. Local acquisition is not HTTPS QA.
use super::cas::hash_file;
use super::errors::ErrorCode;
use super::schema::DigestAlgorithm;
use super::service::FusionStorage;
use super::test_support::{CreatorHarness, BASE, RUNTIME, SOURCE_ID, USER_DATA};
use std::fs;

#[tokio::test]
async fn legacy_projection_shared_resolver_preserves_desktop_install() {
    let fixture = CreatorHarness::synthetic().await;
    let result = fixture
        .service
        .game_install(&fixture.game, fixture.request.clone())
        .await
        .unwrap();
    assert_eq!(result.status, "completed");
    let snapshot = fixture.snapshot();
    assert_eq!(
        fs::read(snapshot.workspace_root.join("runtime/main.exe")).unwrap(),
        RUNTIME
    );
    assert_eq!(
        fs::read(snapshot.workspace_root.join("base.bin")).unwrap(),
        BASE
    );
}

#[tokio::test]
async fn fixture_source_rebuilds_with_identical_publication_hashes() {
    let fixture = CreatorHarness::synthetic().await;
    let proof = &fixture.reproducibility;
    assert_ne!(fixture.published, fixture.repeated_publication);
    assert!(!proof.first.files.is_empty());
    assert!(proof.first.files.contains_key("source-index.json"));
    assert!(proof
        .first
        .files
        .keys()
        .any(|path| path.starts_with("manifests/")));
    assert!(proof.first.artifacts.contains_key("runtime"));
    assert!(proof.first.artifacts.contains_key("content-pack"));
    for artifact in proof.first.artifacts.values() {
        assert_eq!(artifact.transport_digest.value.len(), 64);
        assert_eq!(artifact.cas_blake3.len(), 64);
        assert!(artifact.size_bytes > 0);
    }
    assert_eq!(
        proof.first, proof.second,
        "same recipe, inputs and TEST key must reproduce all bytes and transport/CAS digests"
    );
    println!("reproducible publication: {proof:#?}");
}

#[test]
fn publication_fingerprint_detects_changed_owned_file_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let probe = directory.path().join("owned-probe.txt");
    fs::write(&probe, b"first owned bytes").unwrap();
    let before = super::test_support::publication_hashes(directory.path());
    fs::write(&probe, b"second owned bytes").unwrap();
    let after = super::test_support::publication_hashes(directory.path());
    assert_ne!(before["owned-probe.txt"], after["owned-probe.txt"]);
}
#[tokio::test]
async fn fixture_supplies_the_complete_install_request() {
    let fixture = CreatorHarness::synthetic().await;
    assert!(fixture.request.user_files.contains_key("user-data"));
    assert_eq!(fixture.request.raw_paths["runtime"], "main.exe");
    assert_eq!(fixture.request.raw_paths["user-data"], "player.bin");
    assert_eq!(
        fixture.request.external_workspace_root.as_deref(),
        Some(fixture.external_workspace.to_str().unwrap())
    );
    assert!(!fixture.author.exists(), "author inputs are unavailable");
    assert!(fixture
        .storage
        .get_active_snapshot(&fixture.game)
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn desktop_transport_clean_install_consumes_published_and_player_inputs() {
    let fixture = CreatorHarness::synthetic().await;
    let result = fixture
        .service
        .game_install(&fixture.game, fixture.request.clone())
        .await
        .unwrap();
    assert_eq!(result.status, "completed");
    let snapshot = fixture.snapshot();
    assert!(snapshot
        .workspace_root
        .starts_with(&fixture.external_workspace));
    assert!(!snapshot.workspace_root.starts_with(&fixture.published));
    assert_eq!(
        fs::read(snapshot.workspace_root.join("runtime/main.exe")).unwrap(),
        RUNTIME
    );
    assert_eq!(
        fs::read(snapshot.workspace_root.join("base.bin")).unwrap(),
        BASE
    );
    assert_eq!(
        fs::read(snapshot.workspace_root.join("user/player.bin")).unwrap(),
        USER_DATA
    );
    let state = fixture.service.game_get_state(&fixture.game).await.unwrap();
    assert!(
        !state.ready,
        "a source signature never grants executable approval"
    );
    assert!(!state.executables.is_empty());
    assert!(fixture
        .storage
        .pinned_source_identity(SOURCE_ID, &fixture.index_url())
        .unwrap()
        .is_some());
    assert!(fixture.database.is_file());
    assert!(fixture.library.is_dir());
    assert_eq!(
        fs::read(&fixture.request.user_files["user-data"]).unwrap(),
        USER_DATA
    );
}

#[cfg(windows)]
#[tokio::test]
async fn native_desktop_install_launch_update_rollback_preserves_latest_save() {
    let fixture = CreatorHarness::native().await;
    assert!(!fixture.author.exists());
    fixture
        .service
        .game_install(&fixture.game, fixture.request.clone())
        .await
        .unwrap();
    let initial = fixture.snapshot();
    assert_eq!(
        fixture
            .service
            .game_launch(&fixture.game)
            .await
            .unwrap_err()
            .code,
        ErrorCode::UntrustedExecutable
    );
    fixture.approve_and_launch().await;
    let save = initial.workspace_root.join("saves/progress.dat");
    assert_eq!(fs::read(&save).unwrap(), b"new atomic save");
    fs::write(&save, b"player progress after initial launch").unwrap();

    fixture.connect(&fixture.published_update).await;
    let update = fixture
        .service
        .game_update(&fixture.game, fixture.request.clone())
        .await
        .unwrap();
    assert_eq!(update.status, "completed");
    let updated = fixture.snapshot();
    assert_ne!(updated.id, initial.id);
    assert_eq!(updated.manifest.version, "2");
    assert!(updated.delta.reused.contains(&"base".into()));
    assert_eq!(
        fs::read(updated.workspace_root.join("saves/progress.dat")).unwrap(),
        b"player progress after initial launch"
    );
    assert!(
        !fixture
            .service
            .game_get_state(&fixture.game)
            .await
            .unwrap()
            .ready,
        "changed runtime needs fresh approval"
    );
    assert_ne!(
        hash_file(
            &initial.workspace_root.join("runtime/main.exe"),
            DigestAlgorithm::Blake3
        )
        .unwrap(),
        hash_file(
            &updated.workspace_root.join("runtime/main.exe"),
            DigestAlgorithm::Blake3
        )
        .unwrap()
    );
    fixture.approve_and_launch().await;
    fs::write(
        updated.workspace_root.join("saves/progress.dat"),
        b"latest player progress after update",
    )
    .unwrap();

    let rollback = fixture.service.game_rollback(&fixture.game).await.unwrap();
    assert_eq!(rollback.status, "completed");
    let rolled = fixture.snapshot();
    assert_eq!(rolled.id, initial.id);
    assert_eq!(
        fs::read(rolled.workspace_root.join("saves/progress.dat")).unwrap(),
        b"latest player progress after update"
    );
    assert!(
        fixture
            .service
            .game_get_state(&fixture.game)
            .await
            .unwrap()
            .ready
    );
    fixture.approve_and_launch().await;
    assert_eq!(
        fs::read(rolled.workspace_root.join("saves/progress.dat")).unwrap(),
        b"new atomic save"
    );
    assert!(
        !fixture.author.exists(),
        "no launch can fall back to the author input"
    );
}
