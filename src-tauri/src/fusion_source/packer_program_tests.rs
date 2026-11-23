use std::fs;

use serde_json::{json, Value};

use crate::fusion_source::cas::CasStore;
use crate::fusion_source::dto::{GameOptions, InstallRequest};
use crate::fusion_source::layers::LayerInput;
use crate::local_sources::LocalSourceCandidate;

use super::{Fixture, PackerEngine, PackerRecipe};

const GAME: &str = "inline-program";
const SOURCE: &str = "packer-inline-fixture";
const SOURCE_URL: &str = "https://offline.fusion.invalid/packer-inline-fixture/";

#[test]
fn inline_program_authoring_rejects_unsigned_unsupported_and_unbuildable_inputs() {
    use crate::fusion_source::test_support::publication_hashes;
    use crate::fusion_source::ErrorCode;

    let author = Fixture::new(false);
    let engine = PackerEngine::new();
    let root = author.temporary.path();
    let key = root.join("private-signing.pem");
    crate::fusion_source::signatures::keygen(&key).unwrap();
    let build = |recipe: Value, key| {
        let recipe =
            serde_json::from_value::<PackerRecipe>(recipe).expect("inline recipe must parse");
        engine.build_with_signing_key(&recipe, root, &author.dist, key)
    };
    assert_eq!(
        build(recipe(), None).unwrap_err().code,
        ErrorCode::InvalidSourceSignature
    );
    assert!(!author.dist.exists());
    build(recipe(), Some(key.as_path())).unwrap();
    let before = publication_hashes(&author.dist);

    for (pointer, replacement, expected) in [
        (
            "/games/0/program_descriptor/components/0/origin/redistribution_allowed",
            json!(false),
            ErrorCode::ActionUnavailable,
        ),
        (
            "/games/0/program_descriptor/components/0/origin",
            json!({"kind":"external","url":"https://example.com/pinned.zip","transport_blake3":"a".repeat(64),"inventory_blake3":"b".repeat(64)}),
            ErrorCode::ActionUnavailable,
        ),
        (
            "/games/0/program_descriptor/components/0/source_root",
            json!("inner"),
            ErrorCode::ActionUnavailable,
        ),
        (
            "/games/0/layers/0",
            json!({"id":"program","kind":"runtime","local_path":"runtime","mount_path":"conflicting"}),
            ErrorCode::InvalidManifest,
        ),
        (
            "/games/0/layers/0",
            json!({"id":"program","kind":"runtime","local_path":"runtime/main.exe","archive":"raw"}),
            ErrorCode::ActionUnavailable,
        ),
        (
            "/games/0/program_descriptor/required_files",
            json!([{"path":"app/main.exe","role":"main_executable"},{"path":"app/missing.dat","role":"data"}]),
            ErrorCode::InvalidManifest,
        ),
        (
            "/games/0/program_descriptor/build_profiles/0/settings/0/value",
            json!("out of domain"),
            ErrorCode::InvalidManifest,
        ),
    ] {
        let mut invalid = recipe();
        invalid["games"][0]["version"] = json!("2");
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert_eq!(
            build(invalid, Some(key.as_path())).unwrap_err().code,
            expected,
            "{pointer}"
        );
        assert_eq!(publication_hashes(&author.dist), before, "{pointer}");
    }
}

#[test]
fn inline_program_rejects_signing_key_in_component() {
    use crate::fusion_source::{signatures, ErrorCode};
    let author = Fixture::new(false);
    let root = author.temporary.path();
    let key = root.join("local-test-key.pem");
    signatures::keygen(&key).unwrap();
    let recipe = serde_json::from_value::<PackerRecipe>(recipe()).unwrap();
    let engine = PackerEngine::new();
    engine
        .build_with_signing_key(&recipe, root, &author.dist, Some(&key))
        .unwrap();
    let previous = crate::fusion_source::test_support::publication_hashes(&author.dist);
    let nested = root.join("runtime/private-input");
    fs::create_dir(&nested).unwrap();
    let copied_key = nested.join("local-test-key.pem");
    fs::copy(&key, &copied_key).unwrap();
    for selected_key in [&copied_key, &key] {
        let error = engine
            .build_with_signing_key(&recipe, root, &author.dist, Some(selected_key))
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidPath);
        assert!(error.details.is_empty());
        assert_eq!(
            crate::fusion_source::test_support::publication_hashes(&author.dist),
            previous
        );
    }
}

#[test]
fn inline_program_rejects_prebuilt_archive_and_key_in_output() {
    use crate::fusion_source::{signatures, ErrorCode};
    let author = Fixture::new(false);
    let root = author.temporary.path();
    let key = root.join("local-test-key.pem");
    signatures::keygen(&key).unwrap();
    let engine = PackerEngine::new();
    let mut input = recipe();
    input["games"][0]["layers"][0]["local_path"] = json!("runtime.zip");
    let archive_recipe = serde_json::from_value::<PackerRecipe>(input).unwrap();
    let err = engine
        .build_with_signing_key(&archive_recipe, root, &author.dist, Some(&key))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::ActionUnavailable);
    assert!(!author.dist.join("source-index.json").exists());
    fs::create_dir_all(&author.dist).unwrap();
    let output_key = author.dist.join("local-test-key.pem");
    fs::rename(&key, &output_key).unwrap();
    let directory_recipe = serde_json::from_value::<PackerRecipe>(recipe()).unwrap();
    let err = engine
        .build_with_signing_key(&directory_recipe, root, &author.dist, Some(&output_key))
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidPath);
    assert!(err.details.is_empty());
    assert!(!author.dist.join("source-index.json").exists());
}

#[test]
fn inline_program_verify_refuses_unsigned_source() {
    use crate::fusion_source::{signatures, ErrorCode};
    let author = Fixture::new(false);
    let root = author.temporary.path();
    let key = root.join("local-test-key.pem");
    signatures::keygen(&key).unwrap();
    let recipe = serde_json::from_value::<PackerRecipe>(recipe()).unwrap();
    let engine = PackerEngine::new();
    engine
        .build_with_signing_key(&recipe, root, &author.dist, Some(&key))
        .unwrap();
    let mut index = author.index();
    index.signature = None;
    index.source_info.public_key = None;
    fs::write(
        author.dist.join("source-index.json"),
        serde_json::to_vec(&index).unwrap(),
    )
    .unwrap();
    assert_eq!(
        engine.verify(&author.dist).unwrap_err().code,
        ErrorCode::InvalidSourceSignature
    );
    let before = crate::fusion_source::test_support::publication_hashes(&author.dist);
    let mut legacy = author.recipe.clone();
    legacy["source_info"]["id"] = json!(SOURCE);
    legacy["base_url"] = json!(SOURCE_URL);
    let legacy = serde_json::from_value::<PackerRecipe>(legacy).unwrap();
    assert_eq!(
        engine.build(&legacy, root, &author.dist).unwrap_err().code,
        ErrorCode::InvalidSourceSignature
    );
    assert_eq!(
        crate::fusion_source::test_support::publication_hashes(&author.dist),
        before
    );
}

fn recipe() -> Value {
    json!({
        "source_info":{"id":SOURCE,"name":"Owned inline program test"},
        "base_url":SOURCE_URL,
        "games":[{
            "id":GAME, "title":"Inline demo", "version":"1",
            "layers":[{"id":"program","kind":"runtime","local_path":"runtime", "archive":"zip"}],
            "program_descriptor":{
                "document_type":"program_descriptor", "schema_version":"1.0",
                "identity":{"id":"inline-demo","title":"Inline demo","version":"1","os":"windows","architecture":"x86_64"},
                "executor":{"id":"fusion.declarative","version":"1.0"},
                "components":[{"id":"program","layer_id":"program","origin":{
                    "kind":"managed","artifact_ref":"fixture-artifact","redistribution_allowed":true
                },"source_root":"","mount_path":"app"}],
                "required_files":[{"path":"app/main.exe","role":"main_executable"}],
                "projection_rules":[],"directories":["userdata"],
                "entrypoint":{"executable":"app/main.exe","working_directory":"app","arguments":[],"environment_paths":[]},
                "configs":[{"id":"main","path":"userdata/main.ini","format":"ini","seed":{"kind":"empty_ini"},"preserve_unmanaged":true,
                    "keys":[{"id":"enabled","address":{"kind":"ini","section":"Display","key":"enabled"},"scalar_type":"boolean","allowed_values":[true,false]}]}],
                "build_profiles":[{"id":"standard","title":"Standard","layer_ids":["program"],"controls_profile_ids":["native"],
                    "settings":[{"config_id":"main","key_id":"enabled","value":true}]}],
                "controls_profiles":[{"id":"native","title":"Application defaults","kind":"application_defaults"}],
                "persistence":[]
            }
        }]
    })
}

#[tokio::test]
async fn packer_signed_inline_program_installs_from_published_source() {
    use crate::fusion_source::service::{FusionStorage, ServiceEnvironment};
    use crate::fusion_source::test_support::CreatorHarness;
    use crate::fusion_source::{ErrorCode, QualifiedGameId};

    let author = Fixture::new(false);
    // The second binary is not in required_files, but is still loadable.
    fs::write(
        author.temporary.path().join("runtime/plugin.dll"),
        b"owned DLL fixture",
    )
    .unwrap();
    let recipe = recipe();
    let key = author.temporary.path().join("private-signing.pem");
    let identity = crate::fusion_source::signatures::keygen(&key).unwrap();
    let engine = PackerEngine::new();
    let report = engine
        .build_with_signing_key(
            &serde_json::from_value::<PackerRecipe>(recipe.clone())
                .expect("inline recipe must parse"),
            author.temporary.path(),
            &author.dist,
            Some(&key),
        )
        .unwrap();
    let checked = engine.verify(&author.dist).unwrap();
    assert!(checked.signature_verified);
    assert_eq!(
        (checked.manifests_verified, checked.artifacts_verified),
        (1, 1)
    );
    let index = author.index();
    assert_eq!(index.schema_version, "1.0");
    let entry = &index.games[0];
    let relative = entry.manifest_url.strip_prefix(SOURCE_URL).unwrap();
    let manifest_path = author.dist.join(relative);
    let bytes = fs::read(&manifest_path).unwrap();
    assert_eq!(
        entry.manifest_blake3,
        blake3::hash(&bytes).to_hex().as_str()
    );
    let wire: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(wire["schema_version"], "1.1");
    assert_eq!(
        wire["creator"],
        json!({"program_descriptor":recipe["games"][0]["program_descriptor"]})
    );
    assert!(wire["runtime"].is_null());
    assert!(wire["launch"].is_null());
    let critical = wire["layers"][0]["critical_files"].as_array().unwrap();
    assert!(critical
        .iter()
        .any(|file| file["path"] == "plugin.dll" && file["role"] == "core_library"));
    assert!(!author.dist.join("private-signing.pem").exists());
    // Remove author inputs before the player connects; only the build output is available.
    fs::remove_dir_all(author.temporary.path().join("runtime")).unwrap();
    let player = CreatorHarness::synthetic().await;
    let game = QualifiedGameId {
        source_id: SOURCE.into(),
        game_id: GAME.into(),
    };
    let objects = player.library.join(".fusion-source/cas/objects");
    assert!(!objects
        .read_dir()
        .is_ok_and(|mut entries| entries.next().is_some()));
    assert!(player.storage.get_active_snapshot(&game).unwrap().is_none());
    let candidate = LocalSourceCandidate::open(
        &author.dist,
        &player.library,
        player.storage.clone(),
        ServiceEnvironment {
            appdata: player.library.join("appdata"),
            local_appdata: player.library.join("local-appdata"),
            display_width: 1280,
            display_height: 720,
            refresh_rate: 60,
        },
    )
    .unwrap();
    assert!(candidate.preview().await.unwrap().source.signature_verified);
    candidate
        .connect(&player.storage, SOURCE, false)
        .await
        .unwrap();
    let pin = player
        .storage
        .pinned_source_identity(SOURCE, &format!("{SOURCE_URL}source-index.json"))
        .unwrap()
        .unwrap();
    assert_eq!(
        pin.public_key.as_deref(),
        Some(identity.public_key.as_str())
    );
    player
        .service
        .game_set_options(
            &game,
            GameOptions {
                build_profile_id: Some("standard".into()),
                controls_profile_id: Some("native".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        player
            .service
            .game_install(&game, InstallRequest::default())
            .await
            .unwrap()
            .status,
        "completed"
    );
    let active = player.storage.get_active_snapshot(&game).unwrap().unwrap();
    let snapshot: crate::fusion_source::workspace::WorkspaceSnapshot =
        serde_json::from_str(&active.snapshot_json).unwrap();
    let digest = &report.artifacts[0].cas_digest;
    assert!(
        matches!(&snapshot.inputs["program"], LayerInput::Cas { digest: actual, runtime_lock: None } if actual == digest)
    );
    assert_eq!(
        CasStore::open(&player.library)
            .unwrap()
            .verify(digest)
            .unwrap()
            .files
            .len(),
        2
    );
    assert_eq!(
        fs::read(snapshot.workspace_root.join("app/main.exe")).unwrap(),
        b"test runtime version one"
    );
    assert!(
        fs::read_to_string(snapshot.workspace_root.join("userdata/main.ini"))
            .unwrap()
            .contains("enabled=true")
    );
    assert_eq!(
        player.service.game_launch(&game).await.unwrap_err().code,
        ErrorCode::UntrustedExecutable
    );

    // Change only valid inline descriptor data; do not touch index or its signature.
    let mut changed = wire;
    changed["creator"]["program_descriptor"]["identity"]["title"] = json!("Other title");
    fs::write(manifest_path, serde_json::to_vec(&changed).unwrap()).unwrap();
    assert_eq!(
        engine.verify(&author.dist).unwrap_err().code,
        ErrorCode::ManifestDigestMismatch
    );
}
