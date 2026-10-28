use super::*;
use std::time::Duration;

/// A source release can change configuration/mods without a gameplay fixture.
/// Exercise the public offline host with actual packer bytes, CAS and snapshots.
#[tokio::test]
async fn offline_config_releases_and_addon_selection_preserve_saves_and_rollback() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    let addon = root.path().join("addon");
    std::fs::create_dir_all(input.join("saves")).unwrap();
    std::fs::create_dir(&addon).unwrap();
    std::fs::write(input.join("main.exe"), b"fixture runtime one").unwrap();
    std::fs::write(input.join("saves/progress.dat"), b"initial save").unwrap();
    std::fs::write(addon.join("graphics.txt"), b"optional graphics fixture").unwrap();
    let recipe = root.path().join("recipe.json");
    let mut document = serde_json::json!({
        "source_info":{"id":"owned","name":"Owned"},
        "base_url":"https://offline.fusion.invalid/owned/",
        "games":[{"id":"demo","title":"Demo","version":"1",
            "layers":[
                {"id":"runtime","kind":"runtime","local_path":"input","mount_path":"runtime",
                 "critical_files":[{"path":"main.exe","role":"main_executable"}]},
                {"id":"graphics","kind":"mod","local_path":"addon","mount_path":"mods",
                 "optional":true,"default_enabled":true}
            ],
            "runtime":{"layer_id":"runtime","executable":"runtime/main.exe"},
            "persistence":{"saves_paths":["workspace/runtime/saves"]},
            "config_templates":[{"id":"quality","path":"settings/quality.ini","format":"ini","template":"[graphics]\nquality=balanced\n"}]
        }]
    });
    std::fs::write(&recipe, serde_json::to_vec(&document).unwrap()).unwrap();
    let first_source = root.path().join("source-one");
    super::super::packer::PackerEngine::new()
        .build_file(&recipe, &first_source)
        .unwrap();
    let library = root.path().join("library");
    let host = OfflineLibrary::open(&first_source, &library, "demo")
        .await
        .unwrap();
    host.install().await.unwrap();
    host.trust(
        "runtime/main.exe",
        blake3::hash(b"fixture runtime one").to_hex().as_ref(),
    )
    .await
    .unwrap();
    let initial = host.state().await.unwrap();
    assert!(initial.ready);
    assert!(initial
        .applied_options
        .unwrap()
        .enabled_addons
        .unwrap()
        .contains("graphics"));
    let selection = super::super::dto::GameOptions {
        enabled_addons: Some(Default::default()),
        runtime_channel: None,
    };
    let staged = host.set_options(selection.clone()).await.unwrap();
    assert_eq!(staged.active_snapshot_id, initial.active_snapshot_id);
    assert!(staged
        .applied_options
        .unwrap()
        .enabled_addons
        .unwrap()
        .contains("graphics"));
    host.update().await.unwrap();
    let without_addon = host.state().await.unwrap();
    assert_ne!(without_addon.active_snapshot_id, initial.active_snapshot_id);
    assert_eq!(
        without_addon.applied_options.unwrap().enabled_addons,
        selection.enabled_addons
    );
    let save = library
        .join(".fusion-source/persistence/owned/demo/data/workspace/runtime/saves/progress.dat");
    std::fs::write(&save, b"latest user save").unwrap();
    drop(host);

    document["games"][0]["version"] = serde_json::json!("2");
    document["games"][0]["config_templates"][0]["template"] =
        serde_json::json!("[graphics]\nquality=quality\n");
    std::fs::write(input.join("main.exe"), b"fixture runtime two").unwrap();
    std::fs::write(&recipe, serde_json::to_vec(&document).unwrap()).unwrap();
    let second_source = root.path().join("source-two");
    super::super::packer::PackerEngine::new()
        .build_file(&recipe, &second_source)
        .unwrap();
    let host = OfflineLibrary::open(&second_source, &library, "demo")
        .await
        .unwrap();
    assert!(host.state().await.unwrap().update_available);
    host.update().await.unwrap();
    let updated = host.state().await.unwrap();
    assert_eq!(updated.active_version.as_deref(), Some("2"));
    assert!(
        !updated.ready,
        "a new executable digest needs separate trust"
    );
    assert_eq!(std::fs::read(&save).unwrap(), b"latest user save");
    let assembler =
        super::super::workspace::WorkspaceAssembler::open(&library, &host.game).unwrap();
    {
        let active = assembler.active().unwrap().unwrap();
        assert_eq!(
            std::fs::read_to_string(active.snapshot.workspace_root.join("settings/quality.ini"))
                .unwrap(),
            "[graphics]\nquality=quality\n"
        );
        assert!(!active
            .snapshot
            .workspace_root
            .join("mods/graphics.txt")
            .exists());
    }
    host.rollback().await.unwrap();
    let rolled = host.state().await.unwrap();
    assert_eq!(rolled.active_snapshot_id, without_addon.active_snapshot_id);
    assert_eq!(rolled.active_version.as_deref(), Some("1"));
    assert!(rolled.ready);
    assert_eq!(std::fs::read(&save).unwrap(), b"latest user save");
    let active = assembler.active().unwrap().unwrap();
    assert_eq!(
        std::fs::read_to_string(active.snapshot.workspace_root.join("settings/quality.ini"))
            .unwrap(),
        "[graphics]\nquality=balanced\n"
    );
}

#[tokio::test]
async fn eden_profile_preparation_uses_managed_persistence() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir_all(input.join("user/config")).unwrap();
    std::fs::write(input.join("eden.exe"), b"not executed").unwrap();
    std::fs::write(input.join("user/config/qt-config.ini"), "[Controls]\nmouse_panning_x_sensitivity=68\n[Data%20Storage]\nnand_directory=F:/previous/user/nand\nnand_directory\\default=false\n").unwrap();
    let recipe = root.path().join("recipe.json");
    std::fs::write(&recipe, serde_json::to_vec(&serde_json::json!({
        "source_info":{"id":"owned","name":"Owned"}, "base_url":"https://offline.fusion.invalid/owned/",
        "games":[{"id":"demo","title":"Demo","version":"1","layers":[{"id":"runtime","kind":"runtime","local_path":"input","mount_path":"runtime","critical_files":[{"path":"eden.exe","role":"main_executable"}]}],"runtime":{"layer_id":"runtime","executable":"runtime/eden.exe"},"persistence":{"saves_paths":["workspace/runtime/user"]}}]
    })).unwrap()).unwrap();
    let source = root.path().join("source");
    super::super::packer::PackerEngine::new()
        .build_file(&recipe, &source)
        .unwrap();
    let library = root.path().join("library");
    let host = OfflineLibrary::open(&source, &library, "demo")
        .await
        .unwrap();
    host.install().await.unwrap();
    host.prepare_eden_profile().await.unwrap();
    let data = library.join(
        ".fusion-source/persistence/owned/demo/data/workspace/runtime/user/config/qt-config.ini",
    );
    let text = std::fs::read_to_string(data).unwrap();
    assert!(text.contains("mouse_panning_x_sensitivity=68"));
    assert!(text.contains("nand_directory\\default=true"));
    assert!(!text.contains("previous"));
    assert!(text.contains("Paths\\gamedirs\\size=1"));
    assert!(text.contains("Paths\\gamedirs\\1\\path="));
}

#[test]
fn eden_relocation_keeps_input_settings_and_current_paths() {
    let text = "[Controls]\nmouse_panning_x_sensitivity=68\n[Data%20Storage]\nnand_directory=F:/old/runtime/user/nand\nnand_directory\\default=false\n[Audio]\nvolume=100\n";
    let root = Path::new("F:/new/runtime/user");
    let relocated = relocate_eden_ini(text, root);
    assert!(relocated.contains("nand_directory=\nnand_directory\\default=true"));
    assert!(relocated.contains("mouse_panning_x_sensitivity=68"));
    assert!(relocated.contains("volume=100"));
    let current = text.replace("F:/old", "F:/new");
    assert_eq!(relocate_eden_ini(&current, root), current);
}

#[tokio::test]
async fn transport_is_bounded_offline_and_rejects_path_aliases() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source-index.json"), b"local bytes").unwrap();
    let transport = OfflineTransport::open(root.path(), "owned").unwrap();
    for url in [
        "https://other.example/source-index.json",
        "https://offline.fusion.invalid/owned/../secret",
        "https://offline.fusion.invalid/owned/%2e%2e/secret",
        "https://offline.fusion.invalid/owned/source-index.json?token=secret",
    ] {
        assert!(transport.resolve(url).is_err());
    }
    let target = root.path().join("copy");
    let limits = TransferLimits {
        max_bytes: 64,
        timeout: Duration::from_secs(2),
    };
    let receipt = transport
        .fetch(
            &transport.index_url(),
            &target,
            &limits,
            &Cancellation::default(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.bytes, 11);
    assert_eq!(std::fs::read(&target).unwrap(), b"local bytes");
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        transport
            .fetch(
                &transport.index_url(),
                &root.path().join("cancelled"),
                &limits,
                &cancel
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(!root.path().join("cancelled").exists());
    let short = TransferLimits {
        max_bytes: 1,
        ..limits
    };
    assert_eq!(
        transport
            .fetch(
                &transport.index_url(),
                &root.path().join("oversize"),
                &short,
                &Cancellation::default()
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::TransferLimitExceeded
    );
    assert!(!root.path().join("oversize").exists());
}

#[tokio::test]
async fn offline_install_uses_real_cas_and_exact_hash_trust_and_reopens() {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("input");
    std::fs::create_dir(&input).unwrap();
    std::fs::write(input.join("main.exe"), b"unexecuted fixture bytes").unwrap();
    std::fs::write(input.join("core.dll"), b"fixture library").unwrap();
    let recipe = root.path().join("recipe.json");
    std::fs::write(&recipe, serde_json::to_vec(&serde_json::json!({
        "source_info":{"id":"owned","name":"Owned"},
        "base_url":"https://offline.fusion.invalid/owned/",
        "games":[{"id":"demo","title":"Demo","version":"1","layers":[{"id":"runtime","kind":"runtime","local_path":"input","mount_path":"runtime","critical_files":[{"path":"main.exe","role":"main_executable"},{"path":"core.dll","role":"core_library"}]}],"runtime":{"layer_id":"runtime","executable":"runtime/main.exe"}}]
    })).unwrap()).unwrap();
    let source = root.path().join("source");
    super::super::packer::PackerEngine::new()
        .build_file(&recipe, &source)
        .unwrap();
    let library = root.path().join("library");
    let host = OfflineLibrary::open(&source, &library, "demo")
        .await
        .unwrap();
    assert!(OfflineLibrary::open(&source, &library, "demo")
        .await
        .is_err());
    host.install().await.unwrap();
    let state = host.state().await.unwrap();
    assert!(state.installed);
    assert!(!state.ready);
    let digest = blake3::hash(b"unexecuted fixture bytes")
        .to_hex()
        .to_string();
    assert!(host
        .trust("runtime/main.exe", &"0".repeat(64))
        .await
        .is_err());
    assert!(host
        .trust_many(vec![
            ("runtime/main.exe".into(), digest.clone()),
            ("runtime/core.dll".into(), "0".repeat(64))
        ])
        .await
        .is_err());
    assert!(host
        .state()
        .await
        .unwrap()
        .executables
        .iter()
        .all(|e| !e.trusted));
    host.trust_many(vec![
        ("runtime/main.exe".into(), digest),
        (
            "runtime/core.dll".into(),
            blake3::hash(b"fixture library").to_hex().to_string(),
        ),
    ])
    .await
    .unwrap();
    assert!(host.state().await.unwrap().ready);
    drop(host);
    let reopened = OfflineLibrary::open(&source, &library, "demo")
        .await
        .unwrap();
    assert!(reopened.state().await.unwrap().ready);
}
