use std::collections::{BTreeMap, BTreeSet};

use serde_json::{json, Value};

use super::canonical::{CanonicalGameManifest, CasAccess, QualifiedGameId};
use super::capabilities::{CapabilityNegotiator, CapabilityRegistry, CapabilitySelection};
use super::errors::{ErrorCode, FusionError};
use super::evolution::SchemaUpcaster;
use super::plan::{
    CriticalDigestExpectation, EnvStep, ExecStep, FetchStep, PlanBuilder, PlanOptions, PlanStep,
    TransformStep, WorkspaceLocation,
};
use super::schema::{
    ArchiveFormat, CapabilityFallback, CriticalFileRole, DigestAlgorithm, FallbackStrategy,
    GameManifest, LayerContent, LayerKind, LayerSource, LocalPlacementPolicy, ProjectionStrategy,
    RuntimeChannel,
};

const SOURCE: &str = "fusion-examples";
const SWITCH: &str = include_str!("fixtures/switch-zelda.json");
const AZIMUTH: &str = include_str!("fixtures/ps5-azimuth.json");
const LOCAL_BASE: &str = include_str!("fixtures/cyberpunk-base.json");
const COOP: &str = include_str!("fixtures/cyberpunk-minecraft-coop.json");
const INDEX: &str = include_str!("fixtures/source-index.json");

#[test]
fn source_index_rejects_case_insensitive_game_duplicates_without_normalizing_ids() {
    let mut document: Value = serde_json::from_str(INDEX).unwrap();
    document["games"][0]["id"] = json!("Game");
    document["games"][1]["id"] = json!("game");
    assert_eq!(
        SchemaUpcaster::upcast_source_index(&document.to_string())
            .unwrap_err()
            .code,
        ErrorCode::DuplicateId
    );
    document["games"][1]["id"] = json!("other-game");
    let canonical = SchemaUpcaster::upcast_source_index(&document.to_string()).unwrap();
    assert_eq!(canonical.games[0].id, "Game");
}

fn value(fixture: &str) -> Value {
    serde_json::from_str(fixture).expect("fixture JSON")
}

fn canonical_json(document: &Value) -> Result<CanonicalGameManifest, FusionError> {
    SchemaUpcaster::upcast_game_manifest(SOURCE, &document.to_string(), &BTreeMap::new())
}

#[test]
fn creator_manifest_11_is_readable_but_index_stays_10() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../fixtures/creator/contracts.json")).unwrap();
    let wire = serde_json::to_string(&fixture["manifest"]).unwrap();
    let manifest = SchemaUpcaster::parse_game_manifest(&wire).unwrap();
    assert_eq!(manifest.schema_version, "1.1");
    assert!(SchemaUpcaster::upcast_source_index(
        r#"{"schema_version":"1.1","source_info":{"id":"source","name":"Fixture"},"games":[]}"#
    )
    .is_err());
}

#[test]
fn creator_v11_planning_is_explicitly_unavailable_in_wave_zero() {
    let mut manifest = coop();
    manifest.schema_version = "1.1".into();
    assert_eq!(
        PlanBuilder::build(
            &manifest,
            &PlanOptions::default(),
            &CapabilityRegistry::current_client()
        )
        .unwrap_err()
        .code,
        ErrorCode::ActionUnavailable
    );
}

fn canonical(fixture: &str) -> CanonicalGameManifest {
    SchemaUpcaster::upcast_game_manifest(SOURCE, fixture, &BTreeMap::new())
        .expect("valid fixture manifest")
}

#[test]
fn creator_manifest_inheritance_version_matrix_preserves_legacy_rules() {
    for (child_version, parent_version, accepted) in [
        ("1.0", "1.0", true),
        ("1.1", "1.0", true),
        ("1.1", "1.1", true),
        ("1.0", "1.1", false),
    ] {
        let mut parent = value(LOCAL_BASE);
        parent["schema_version"] = json!(parent_version);
        let parent = SchemaUpcaster::parse_game_manifest(&parent.to_string()).unwrap();
        let parents = BTreeMap::from([(
            QualifiedGameId {
                source_id: SOURCE.into(),
                game_id: parent.id.clone(),
            },
            parent,
        )]);
        let mut child = value(COOP);
        child["schema_version"] = json!(child_version);
        let result = SchemaUpcaster::upcast_game_manifest(SOURCE, &child.to_string(), &parents);
        assert_eq!(
            result.is_ok(),
            accepted,
            "{child_version} extends {parent_version}: {result:?}"
        );
        if let Ok(manifest) = result {
            assert_eq!(manifest.schema_version, child_version);
            SchemaUpcaster::validate_canonical(&manifest).unwrap();
        }
    }
}

#[test]
fn creator_profile_options_are_retained_but_not_silently_applied() {
    let options: super::dto::GameOptions = serde_json::from_value(json!({
        "enabledAddons": null, "runtimeChannel": null,
        "buildProfileId": "default", "controlsProfileId": "keyboard"
    }))
    .unwrap();
    let plan_options = PlanOptions {
        build_profile_id: options.build_profile_id,
        controls_profile_id: options.controls_profile_id,
        ..PlanOptions::default()
    };
    assert_eq!(
        PlanBuilder::build(
            &coop(),
            &plan_options,
            &CapabilityRegistry::current_client()
        )
        .unwrap_err()
        .code,
        ErrorCode::ActionUnavailable
    );
    let legacy: super::dto::GameOptions = serde_json::from_value(json!({
        "enabledAddons": null, "runtimeChannel": null
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(legacy).unwrap(),
        json!({
            "enabledAddons": null, "runtimeChannel": null
        })
    );
}

fn coop() -> CanonicalGameManifest {
    canonical_coop(&value(COOP)).expect("valid inherited coop")
}

fn canonical_coop(document: &Value) -> Result<CanonicalGameManifest, FusionError> {
    let mut parents = BTreeMap::new();
    parents.insert(
        QualifiedGameId {
            source_id: SOURCE.into(),
            game_id: "cyberpunk-base".into(),
        },
        SchemaUpcaster::parse_game_manifest(LOCAL_BASE).expect("base fixture"),
    );
    SchemaUpcaster::upcast_game_manifest(SOURCE, &document.to_string(), &parents)
}

/// Test-only hypothetical implementation set. The production registry must not
/// claim Stage 2/3 adapters merely because Stage 1 can describe them.
fn execution_registry() -> CapabilityRegistry {
    CapabilityRegistry::new([
        "core.schema.v1",
        "core.plan.v1",
        "core.layers.v1",
        "vfs.hardlink.v1",
        "resolver.https.v1",
        "resolver.magnet.v1",
        "local_locator.v1",
        "patch.xdelta3.v1",
        "patch.bsdiff.v1",
        "provider.github_releases.v1",
        "proc.supervisor.v1",
        "core.config_templates.v1",
        "persistence.saves.v1",
    ])
}

fn assert_invalid(document: &Value) -> FusionError {
    canonical_json(document).expect_err("unsafe or invalid manifest must fail")
}

#[test]
fn switch_fixture_normalizes_shared_user_dependencies_and_optional_layers() {
    let manifest = canonical(SWITCH);
    assert_eq!(manifest.source_id, SOURCE);
    assert_eq!(manifest.id, "switch-zelda");
    assert_eq!(manifest.layers.len(), 6);
    for (id, content) in [
        ("switch-firmware", LayerContent::Firmware),
        ("switch-keys", LayerContent::Keys),
    ] {
        let layer = &manifest.layers[id];
        assert_eq!(layer.kind, LayerKind::SharedDeps);
        assert_eq!(layer.content, content);
        assert!(matches!(layer.source, LayerSource::UserProvided { .. }));
        assert_eq!(layer.cas_access, CasAccess::ReadOnly);
    }
    assert!(manifest.layers["addon-60fps"].default_enabled);
    assert!(!manifest.layers["shader-cache"].default_enabled);
    assert_eq!(manifest.layers["shader-cache"].kind, LayerKind::ShaderCache);
    let runtime_position = manifest
        .layer_order
        .iter()
        .position(|id| id == "runtime")
        .unwrap();
    for dependency in ["switch-firmware", "switch-keys"] {
        assert!(
            manifest
                .layer_order
                .iter()
                .position(|id| id == dependency)
                .unwrap()
                < runtime_position
        );
    }
}

#[test]
fn azimuth_fixture_preserves_container_patch_and_binary_digests_separately() {
    let manifest = canonical(AZIMUTH);
    assert_eq!(manifest.layers["base"].size_bytes, Some(120_000_000_000));
    let patch = manifest.layers["delta-update"].patch.as_ref().unwrap();
    assert_eq!(patch.base_layer, "base");
    assert_ne!(patch.input_digest.value, patch.output_digest.value);
    assert_eq!(
        manifest.layers["delta-update"]
            .transport_digest
            .as_ref()
            .unwrap()
            .algorithm,
        DigestAlgorithm::Sha256
    );
    let runtime = &manifest.layers["runtime"];
    assert_ne!(
        runtime.transport_digest.as_ref().unwrap().value,
        runtime.critical_files[0].blake3
    );
    assert_eq!(
        runtime.critical_files[0].role,
        CriticalFileRole::MainExecutable
    );
    assert!(runtime.dynamic_provider.is_some());
    assert_eq!(
        manifest.runtime.as_ref().unwrap().default_channel,
        RuntimeChannel::Pinned
    );
    assert_eq!(manifest.persistence.saves_paths.len(), 2);
}

#[test]
fn coop_fixture_extends_local_base_and_retains_verified_process_graph() {
    let manifest = coop();
    assert_eq!(
        manifest.extends.as_ref().unwrap().to_string(),
        "fusion-examples::cyberpunk-base"
    );
    assert_eq!(manifest.layers.len(), 3);
    assert!(matches!(
        manifest.layers["base"].source,
        LayerSource::LocalLocator { .. }
    ));
    assert!(
        manifest.launch.is_none(),
        "child execution group replaces inherited launch"
    );
    let orchestrator = manifest.orchestrator.as_ref().unwrap();
    assert!(orchestrator.strict_sync);
    assert_eq!(orchestrator.processes.len(), 3);
    assert_eq!(
        manifest.local_placement,
        LocalPlacementPolicy::CrossVolumeFallback
    );
    assert!(manifest.layers["coop-bridge"]
        .critical_files
        .iter()
        .any(|file| file.role == CriticalFileRole::InjectorDll));
}

#[test]
fn index_is_compact_and_upcasts_without_fetching_game_manifests() {
    let index = SchemaUpcaster::upcast_source_index(INDEX).unwrap();
    assert_eq!(index.source_info.id, SOURCE);
    assert_eq!(index.games.len(), 3);
    assert!(index.source_info.public_key.is_some());
    assert!(index
        .games
        .iter()
        .all(|game| game.manifest_blake3.len() == 64));
    // The index intentionally has no layer/process data or supplied manifest resolver.
    let serialized = serde_json::to_value(&index).unwrap();
    assert!(serialized["games"][0].get("layers").is_none());
}

#[test]
fn presentation_metadata_round_trips_and_preserves_absent_canonical_bytes() {
    // Serialized before Stage 6. Snapshot identities hash these exact bytes.
    const LEGACY: &str = r#"{"schemaVersion":"1.0","sourceId":"fusion-examples","id":"minimal","title":"Minimal","version":"1","platform":null,"extends":null,"capabilities":{"required":[],"optional":[]},"fallback":[],"layers":{"base":{"id":"base","kind":"base","content":"generic","dependsOn":[],"source":{"kind":"user_provided","instructions":null,"archive":"raw"},"transportDigest":null,"sizeBytes":null,"mountPath":"","optional":false,"defaultEnabled":false,"writableCopyPaths":[],"criticalFiles":[{"path":"main.exe","blake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","role":"main_executable"}],"dynamicProvider":null,"patch":null,"casAccess":"read_only"}},"layerOrder":["base"],"runtime":null,"launch":{"id":"main","executable":"main.exe","args":[],"env":{},"depends_on":[],"role":"primary"},"orchestrator":null,"persistence":{"saves_paths":[]},"configTemplates":[],"postSetupSteps":[],"localPlacement":"require_same_volume"}"#;
    let old: CanonicalGameManifest = serde_json::from_str(LEGACY).unwrap();
    SchemaUpcaster::validate_canonical(&old).unwrap();
    assert_eq!(serde_json::to_string(&old).unwrap(), LEGACY);
    let mut wire = value(SWITCH);
    wire["cover_url"] = json!("https://images.example.org/zelda.webp");
    wire["layers"][4]["name"] = json!("Плавность 60 FPS");
    wire["layers"][4]["description"] = json!("Повышает частоту кадров.");
    let manifest = canonical_json(&wire).unwrap();
    assert_eq!(
        manifest.cover_url.as_deref(),
        Some("https://images.example.org/zelda.webp")
    );
    let layer = &manifest.layers[wire["layers"][4]["id"].as_str().unwrap()];
    assert_eq!(layer.title.as_deref(), Some("Плавность 60 FPS"));
    assert_eq!(
        layer.description.as_deref(),
        Some("Повышает частоту кадров.")
    );
    let layer_wire = serde_json::to_value(super::schema::LayerSpec::from(layer.clone())).unwrap();
    assert_eq!(layer_wire["title"], "Плавность 60 FPS");
    assert!(layer_wire.get("name").is_none());
    wire["cover_url"] = json!("file:///C:/private/cover.png");
    assert!(canonical_json(&wire).is_err());
    let mut index = value(INDEX);
    index["games"][0]["platform"] = json!("pc");
    index["games"][0]["cover_url"] = json!("https://images.example.org/cover.webp");
    let parsed = SchemaUpcaster::upcast_source_index(&index.to_string()).unwrap();
    assert_eq!(parsed.games[0].platform.as_deref(), Some("pc"));
    assert_eq!(
        parsed.games[0].cover_url.as_deref(),
        Some("https://images.example.org/cover.webp")
    );
}

#[test]
fn canonical_cas_is_read_only_and_preserves_explicit_writable_paths() {
    let manifest = canonical(SWITCH);
    assert!(manifest
        .layers
        .values()
        .all(|layer| layer.cas_access == CasAccess::ReadOnly));
    assert_eq!(
        manifest.layers["runtime"].writable_copy_paths,
        ["config/**", "cache/*.bin"]
    );
}

#[test]
fn unsupported_game_schema_is_typed_instead_of_being_treated_as_legacy() {
    for version in ["0.9", "1.2", "2.0", "v1.0"] {
        let mut document = value(SWITCH);
        document["schema_version"] = json!(version);
        assert_eq!(assert_invalid(&document).code, ErrorCode::UnsupportedSchema);
    }
}

#[test]
fn unsupported_index_schema_is_rejected() {
    let mut document = value(INDEX);
    document["schema_version"] = json!("2.0");
    assert_eq!(
        SchemaUpcaster::upcast_source_index(&document.to_string())
            .unwrap_err()
            .code,
        ErrorCode::UnsupportedSchema
    );
}

#[test]
fn unknown_wire_fields_and_malformed_json_fail_closed() {
    let mut document = value(SWITCH);
    document["execute_shell"] = json!("dangerous");
    assert_eq!(assert_invalid(&document).code, ErrorCode::InvalidJson);
    assert_eq!(
        SchemaUpcaster::parse_game_manifest("{").unwrap_err().code,
        ErrorCode::InvalidJson
    );
}

#[test]
fn layer_graph_rejects_unknown_dependencies() {
    let mut document = value(SWITCH);
    document["layers"][3]["depends_on"] = json!(["absent"]);
    assert_eq!(assert_invalid(&document).code, ErrorCode::MissingLayer);
}

#[test]
fn layer_graph_rejects_cycles() {
    let mut document = value(SWITCH);
    document["layers"][0]["depends_on"] = json!(["runtime"]);
    document["layers"][3]["depends_on"] = json!(["base"]);
    assert_eq!(assert_invalid(&document).code, ErrorCode::LayerCycle);
}

#[test]
fn layer_ids_are_unique() {
    let mut document = value(SWITCH);
    document["layers"][1]["id"] = json!("base");
    assert_eq!(assert_invalid(&document).code, ErrorCode::DuplicateId);
}

#[test]
fn extends_rejects_a_to_b_to_a_cycle() {
    let mut a = value(LOCAL_BASE);
    a["id"] = json!("a");
    a["extends"] = json!("b");
    let mut b = value(LOCAL_BASE);
    b["id"] = json!("b");
    b["extends"] = json!("a");
    let parents: BTreeMap<QualifiedGameId, GameManifest> = [
        (
            QualifiedGameId {
                source_id: SOURCE.into(),
                game_id: "a".into(),
            },
            SchemaUpcaster::parse_game_manifest(&a.to_string()).unwrap(),
        ),
        (
            QualifiedGameId {
                source_id: SOURCE.into(),
                game_id: "b".into(),
            },
            SchemaUpcaster::parse_game_manifest(&b.to_string()).unwrap(),
        ),
    ]
    .into_iter()
    .collect();
    let error = SchemaUpcaster::upcast_game_manifest(SOURCE, &a.to_string(), &parents).unwrap_err();
    assert_eq!(error.code, ErrorCode::ExtendsCycle);
}

#[test]
fn extends_requires_explicitly_supplied_parent() {
    let error = SchemaUpcaster::upcast_game_manifest(SOURCE, COOP, &BTreeMap::new()).unwrap_err();
    assert_eq!(error.code, ErrorCode::MissingParent);
}

#[test]
fn extends_resolves_explicit_cross_source_parent() {
    let mut document = value(COOP);
    document["extends"] = json!("other-source::cyberpunk-base");
    let parents = [(
        QualifiedGameId {
            source_id: "other-source".into(),
            game_id: "cyberpunk-base".into(),
        },
        SchemaUpcaster::parse_game_manifest(LOCAL_BASE).unwrap(),
    )]
    .into_iter()
    .collect();
    let manifest =
        SchemaUpcaster::upcast_game_manifest(SOURCE, &document.to_string(), &parents).unwrap();
    assert_eq!(manifest.extends.unwrap().source_id, "other-source");
    assert_eq!(manifest.source_id, SOURCE);
}

#[test]
fn inherits_capabilities_with_required_taking_precedence() {
    let mut document = value(COOP);
    document["capabilities"]["optional"] = json!(["vfs.hardlink.v1"]);
    let parents = [(
        QualifiedGameId {
            source_id: SOURCE.into(),
            game_id: "cyberpunk-base".into(),
        },
        SchemaUpcaster::parse_game_manifest(LOCAL_BASE).unwrap(),
    )]
    .into_iter()
    .collect();
    let manifest =
        SchemaUpcaster::upcast_game_manifest(SOURCE, &document.to_string(), &parents).unwrap();
    assert!(manifest
        .capabilities
        .required
        .contains(&"vfs.hardlink.v1".into()));
    assert!(!manifest
        .capabilities
        .optional
        .contains(&"vfs.hardlink.v1".into()));
}

#[test]
fn child_cannot_replace_inherited_layer_ids() {
    let mut document = value(COOP);
    document["layers"][0]["id"] = json!("base");
    let parents = [(
        QualifiedGameId {
            source_id: SOURCE.into(),
            game_id: "cyberpunk-base".into(),
        },
        SchemaUpcaster::parse_game_manifest(LOCAL_BASE).unwrap(),
    )]
    .into_iter()
    .collect();
    assert_eq!(
        SchemaUpcaster::upcast_game_manifest(SOURCE, &document.to_string(), &parents)
            .unwrap_err()
            .code,
        ErrorCode::DuplicateId
    );
}

#[test]
fn unpack_mount_paths_block_windows_and_posix_escape_forms() {
    for path in [
        "../outside",
        "safe/../../outside",
        "safe\\..\\outside",
        "C:/outside",
        "C:\\outside",
        "\\\\server\\share\\outside",
        "/absolute",
        "file:stream",
    ] {
        let mut document = value(SWITCH);
        document["layers"][0]["mount_path"] = json!(path);
        assert_eq!(
            assert_invalid(&document).code,
            ErrorCode::InvalidPath,
            "{path}"
        );
    }
}

#[test]
fn writable_copy_masks_block_traversal_and_absolute_paths() {
    for path in [
        "../**",
        "runtime/../../*.ini",
        "runtime\\..\\..\\*.ini",
        "D:/runtime/*.ini",
        "\\\\server\\share\\*",
        "runtime/file:stream",
    ] {
        let mut document = value(SWITCH);
        document["layers"][3]["writable_copy_paths"] = json!([path]);
        assert_eq!(
            assert_invalid(&document).code,
            ErrorCode::InvalidPath,
            "{path}"
        );
    }
}

#[test]
fn config_paths_block_traversal_and_alternate_streams() {
    for path in [
        "../config.ini",
        "runtime\\..\\..\\config.ini",
        "C:/config.ini",
        "config.ini:stream",
    ] {
        let mut document = value(SWITCH);
        document["config_templates"][0]["path"] = json!(path);
        assert_eq!(
            assert_invalid(&document).code,
            ErrorCode::InvalidPath,
            "{path}"
        );
    }
}

#[test]
fn critical_binary_paths_and_patch_outputs_cannot_escape_workspace() {
    let mut document = value(SWITCH);
    document["layers"][3]["critical_files"][0]["path"] = json!("../emulator.exe");
    assert_eq!(assert_invalid(&document).code, ErrorCode::InvalidPath);
    let mut document = value(AZIMUTH);
    document["layers"][1]["patch"]["output_path"] = json!("../patched.bin");
    assert_eq!(assert_invalid(&document).code, ErrorCode::InvalidPath);
}

#[test]
fn persistence_roots_are_explicit_and_cannot_traverse() {
    for path in [
        "workspace/../outside",
        "{{APPDATA}}/../outside",
        "D:/saves",
        "{{HOME}}/saves",
    ] {
        let mut document = value(SWITCH);
        document["persistence"]["saves_paths"] = json!([path]);
        assert_eq!(
            assert_invalid(&document).code,
            ErrorCode::InvalidPath,
            "{path}"
        );
    }
}

#[test]
fn blake3_and_sha256_require_exact_hex_digest_width() {
    for algorithm in ["blake3", "sha256"] {
        for hash in [
            "",
            "abcd",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa00",
        ] {
            let mut document = value(SWITCH);
            document["layers"][3]["transport_digest"] =
                json!({"algorithm":algorithm, "value":hash});
            assert_eq!(
                assert_invalid(&document).code,
                ErrorCode::InvalidDigest,
                "{algorithm}: {hash}"
            );
        }
    }
}

#[test]
fn upcaster_normalizes_hex_digests_and_implicit_patch_dependencies() {
    let mut document = value(AZIMUTH);
    document["layers"][1]["depends_on"] = json!([]);
    document["layers"][2]["transport_digest"]["value"] =
        json!("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
    document["layers"][2]["critical_files"][0]["blake3"] =
        json!("BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB");
    let manifest = canonical_json(&document).unwrap();
    assert_eq!(
        manifest.layers["runtime"]
            .transport_digest
            .as_ref()
            .unwrap()
            .value,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(
        manifest.layers["runtime"].critical_files[0].blake3,
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert!(manifest.layers["delta-update"]
        .depends_on
        .contains(&"base".to_owned()));
}

#[test]
fn critical_file_and_index_hashes_are_validated_separately() {
    let mut document = value(SWITCH);
    document["layers"][3]["critical_files"][0]["blake3"] = json!("bad");
    assert_eq!(assert_invalid(&document).code, ErrorCode::InvalidDigest);
    let mut document = value(INDEX);
    document["games"][0]["manifest_blake3"] = json!("bad");
    assert_eq!(
        SchemaUpcaster::upcast_source_index(&document.to_string())
            .unwrap_err()
            .code,
        ErrorCode::InvalidDigest
    );
}

#[test]
fn unsupported_mirror_schemes_fail_before_any_transport_action() {
    for uri in [
        "ftp://example.org/file.zip",
        "file:///C:/file.zip",
        "javascript:alert(1)",
        "http://example.org/file.zip",
    ] {
        let mut document = value(SWITCH);
        document["layers"][3]["source"]["mirrors"] = json!([uri]);
        assert_eq!(
            assert_invalid(&document).code,
            ErrorCode::InvalidUri,
            "{uri}"
        );
    }
}

#[test]
fn index_manifest_urls_disallow_traversal_and_unsupported_schemes() {
    for uri in [
        "../outside.json",
        "C:/manifest.json",
        "file:///manifest.json",
        "ftp://example.org/manifest.json",
    ] {
        let mut document = value(INDEX);
        document["games"][0]["manifest_url"] = json!(uri);
        let error = SchemaUpcaster::upcast_source_index(&document.to_string()).unwrap_err();
        assert!(
            matches!(error.code, ErrorCode::InvalidPath | ErrorCode::InvalidUri),
            "{uri}: {error}"
        );
    }
}

#[test]
fn public_key_metadata_must_decode_to_ed25519_key_width() {
    for key in ["not-base64", "YQ=="] {
        let mut document = value(INDEX);
        document["source_info"]["public_key"] = json!(key);
        assert_eq!(
            SchemaUpcaster::upcast_source_index(&document.to_string())
                .unwrap_err()
                .code,
            ErrorCode::InvalidPublicKey
        );
    }
}

#[test]
fn execution_error_serialization_preserves_code_and_camel_case_details() {
    let error = FusionError::new(ErrorCode::MissingInjectorDll, "Injector is missing")
        .with_detail("sourceId", SOURCE)
        .with_detail("relativePath", "mods/coop-injector.dll");
    let encoded = serde_json::to_value(&error).unwrap();
    assert_eq!(encoded["code"], "missing_injector_dll");
    assert_eq!(encoded["details"]["relativePath"], "mods/coop-injector.dll");
    assert!(encoded["details"].get("relative_path").is_none());
    assert_eq!(
        serde_json::from_value::<FusionError>(encoded).unwrap(),
        error
    );
}

#[test]
fn current_registry_only_advertises_implemented_capabilities() {
    let registry = CapabilityRegistry::current_client();
    assert!(registry.supports("core.schema.v1"));
    assert!(registry.supports("core.plan.v1"));
    for capability in [
        "vfs.overlay_mount.v2",
        "patch.xdelta3.v1",
        "ext.wasm_transform.v1",
        "resolver.magnet.v1",
        "resolver.pixeldrain.v1",
        "resolver.gofile.v1",
    ] {
        assert!(
            !registry.supports(capability),
            "The local engine has no implementation of {capability}"
        );
    }
    assert!(registry.supports("resolver.https.v1"));
    assert!(registry.supports("provider.github_releases.v1"));
    assert!(registry.supports("local_locator.v1"));
    assert_eq!(registry.supports("proc.supervisor.v1"), cfg!(windows));
}

#[test]
fn capability_negotiation_prefers_an_available_implementation() {
    let manifest = canonical(SWITCH);
    let report = CapabilityNegotiator::negotiate(&manifest, &execution_registry());
    assert!(!report.is_blocked());
    assert!(report.warnings.is_empty());
    assert_eq!(
        report.selection("vfs.hardlink.v1"),
        Some(&CapabilitySelection::Preferred)
    );
}

#[test]
fn optional_overlay_and_wasm_negotiate_supported_fallbacks() {
    let mut manifest = canonical(SWITCH);
    manifest.capabilities.optional.extend([
        "vfs.overlay_mount.v2".into(),
        "ext.wasm_transform.v1".into(),
    ]);
    manifest.fallback.extend([
        CapabilityFallback {
            capability: "vfs.overlay_mount.v2".into(),
            strategy: FallbackStrategy::HardlinkProjection,
            requires: vec!["vfs.hardlink.v1".into()],
        },
        CapabilityFallback {
            capability: "ext.wasm_transform.v1".into(),
            strategy: FallbackStrategy::BuiltinConfigTemplates,
            requires: vec!["core.config_templates.v1".into()],
        },
    ]);
    let report = CapabilityNegotiator::negotiate(&manifest, &execution_registry());
    assert!(!report.is_blocked());
    assert_eq!(report.warnings.len(), 2);
    assert_eq!(
        report.selection("vfs.overlay_mount.v2"),
        Some(&CapabilitySelection::Fallback {
            strategy: FallbackStrategy::HardlinkProjection
        })
    );
    assert_eq!(
        report.selection("ext.wasm_transform.v1"),
        Some(&CapabilitySelection::Fallback {
            strategy: FallbackStrategy::BuiltinConfigTemplates
        })
    );
}

#[test]
fn missing_required_capability_blocks_only_its_game() {
    let mut blocked = canonical(SWITCH);
    blocked
        .capabilities
        .required
        .push("ext.not_implemented.v1".into());
    let report = CapabilityNegotiator::negotiate(&blocked, &execution_registry());
    assert!(report.is_blocked());
    assert_eq!(report.blockers.len(), 1);
    assert_eq!(report.blockers[0].code, ErrorCode::MissingCapability);
    assert_eq!(report.game.game_id, "switch-zelda");
    assert!(
        !CapabilityNegotiator::negotiate(&canonical(AZIMUTH), &execution_registry()).is_blocked()
    );
    assert_eq!(
        SchemaUpcaster::upcast_source_index(INDEX)
            .unwrap()
            .games
            .len(),
        3
    );
}

#[test]
fn missing_optional_capability_without_fallback_is_omitted_with_warning() {
    let mut manifest = canonical(SWITCH);
    manifest
        .capabilities
        .optional
        .push("ext.not_implemented.v1".into());
    let report = CapabilityNegotiator::negotiate(&manifest, &execution_registry());
    assert!(!report.is_blocked());
    assert_eq!(
        report.selection("ext.not_implemented.v1"),
        Some(&CapabilitySelection::Omitted)
    );
    assert_eq!(report.warnings.len(), 1);
}

#[test]
fn fallback_cannot_claim_an_unimplemented_adapter() {
    let mut manifest = canonical(SWITCH);
    manifest.capabilities.required = vec!["vfs.overlay_mount.v2".into()];
    manifest.fallback = vec![CapabilityFallback {
        capability: "vfs.overlay_mount.v2".into(),
        strategy: FallbackStrategy::HardlinkProjection,
        requires: vec![],
    }];
    let report = CapabilityNegotiator::negotiate(
        &manifest,
        &CapabilityRegistry::new(["core.schema.v1", "core.plan.v1"]),
    );
    assert!(report.is_blocked());
    assert_eq!(
        report.selection("vfs.overlay_mount.v2"),
        Some(&CapabilitySelection::Blocked)
    );
}

#[test]
fn required_capability_cannot_be_satisfied_by_skipping_optional_behavior() {
    let mut manifest = canonical(SWITCH);
    manifest
        .capabilities
        .required
        .push("ext.not_implemented.v1".into());
    manifest.fallback.push(CapabilityFallback {
        capability: "ext.not_implemented.v1".into(),
        strategy: FallbackStrategy::SkipOptional,
        requires: vec![],
    });
    let report = CapabilityNegotiator::negotiate(&manifest, &execution_registry());
    assert!(report.is_blocked());
}

#[test]
fn orchestrator_rejects_cycle_and_unknown_process_dependencies() {
    let mut document = value(COOP);
    document["orchestrator"]["processes"][0]["depends_on"] = json!(["cyberpunk"]);
    assert!(canonical_coop(&document).is_err());
    let mut document = value(COOP);
    document["orchestrator"]["processes"][0]["depends_on"] = json!(["absent"]);
    assert!(canonical_coop(&document).is_err());
}

#[test]
fn orchestrator_rejects_duplicate_process_ids_and_multiple_primary_processes() {
    let mut document = value(COOP);
    document["orchestrator"]["processes"][0]["id"] = json!("minecraft");
    assert!(canonical_coop(&document).is_err());
    let mut document = value(COOP);
    document["orchestrator"]["processes"][0]["role"] = json!("primary");
    assert!(canonical_coop(&document).is_err());
}

#[test]
fn executable_requires_a_corresponding_critical_file_hash() {
    let mut document = value(SWITCH);
    document["layers"][3]["critical_files"] = json!([]);
    assert_eq!(assert_invalid(&document).code, ErrorCode::InvalidManifest);
}

#[test]
fn local_locator_key_file_paths_are_relative_even_when_path_hints_are_external() {
    let mut document = value(LOCAL_BASE);
    document["layers"][0]["source"]["locator"]["key_files"][0]["path"] = json!("D:/outside.exe");
    assert_eq!(assert_invalid(&document).code, ErrorCode::InvalidPath);
    assert!(
        canonical_json(&value(LOCAL_BASE)).is_ok(),
        "absolute discovery hint is valid and is never a workspace write target"
    );
}

fn phase(step: &PlanStep) -> usize {
    match step {
        PlanStep::Fetch(_) => 0,
        PlanStep::Transform(_) => 1,
        PlanStep::Env(_) => 2,
        PlanStep::Exec(_) => 3,
    }
}

#[test]
fn switch_fixture_plans_fetch_transform_env_exec_and_manifest_default_addons() {
    let plan = PlanBuilder::build(
        &canonical(SWITCH),
        &PlanOptions::default(),
        &execution_registry(),
    )
    .unwrap();
    assert!(plan.selected_layers.contains(&"base".into()));
    assert!(plan.selected_layers.contains(&"switch-firmware".into()));
    assert!(plan.selected_layers.contains(&"switch-keys".into()));
    assert!(plan.selected_layers.contains(&"runtime".into()));
    assert!(plan.selected_layers.contains(&"addon-60fps".into()));
    assert!(!plan.selected_layers.contains(&"shader-cache".into()));
    assert_eq!(plan.runtime_channel, Some(RuntimeChannel::Pinned));
    assert!(plan
        .steps
        .windows(2)
        .all(|steps| phase(&steps[0]) <= phase(&steps[1])));
    assert!((0..4).all(|expected| plan.steps.iter().any(|step| phase(step) == expected)));
}

#[test]
fn explicitly_empty_addon_selection_disables_manifest_defaults() {
    let options = PlanOptions {
        enabled_addons: Some(BTreeSet::new()),
        ..PlanOptions::default()
    };
    let plan = PlanBuilder::build(&canonical(SWITCH), &options, &execution_registry()).unwrap();
    assert!(!plan.selected_layers.contains(&"addon-60fps".into()));
    assert!(!plan.selected_layers.contains(&"shader-cache".into()));
    assert!(plan.selected_layers.contains(&"runtime".into()));
}

#[test]
fn explicit_addon_selection_is_exact_and_rejects_unknown_ids() {
    let options = PlanOptions {
        enabled_addons: Some(["shader-cache".to_owned()].into_iter().collect()),
        ..PlanOptions::default()
    };
    let plan = PlanBuilder::build(&canonical(SWITCH), &options, &execution_registry()).unwrap();
    assert!(plan.selected_layers.contains(&"shader-cache".into()));
    assert!(!plan.selected_layers.contains(&"addon-60fps".into()));
    let options = PlanOptions {
        enabled_addons: Some(["nonexistent-addon".to_owned()].into_iter().collect()),
        ..PlanOptions::default()
    };
    assert_eq!(
        PlanBuilder::build(&canonical(SWITCH), &options, &execution_registry())
            .unwrap_err()
            .code,
        ErrorCode::UnknownAddon
    );
}

#[test]
fn writable_paths_and_patch_outputs_are_private_copies_while_cas_stays_read_only() {
    let manifest = canonical(AZIMUTH);
    let plan =
        PlanBuilder::build(&manifest, &PlanOptions::default(), &execution_registry()).unwrap();
    assert!(plan.steps.iter().any(|step| matches!(step,
        PlanStep::Transform(TransformStep::MaterializeWritablePaths { layer_id, paths, strategy: ProjectionStrategy::CopyMutable })
        if layer_id == "runtime" && paths.contains(&"runtime/config/**".to_owned()))));
    assert!(plan.steps.iter().any(|step| matches!(step,
        PlanStep::Transform(TransformStep::ApplyPatch { layer_id, output_strategy: ProjectionStrategy::CopyMutable, .. })
        if layer_id == "delta-update")));
    for step in &plan.steps {
        if let PlanStep::Transform(TransformStep::ProjectLayer { cas_access, .. }) = step {
            assert_eq!(*cas_access, CasAccess::ReadOnly);
        }
    }
}

#[test]
fn pinned_runtime_acquires_exact_container_and_dynamic_runtime_requires_provider_lock() {
    let manifest = canonical(AZIMUTH);
    let pinned =
        PlanBuilder::build(&manifest, &PlanOptions::default(), &execution_registry()).unwrap();
    assert!(pinned.steps.iter().any(|step| matches!(step,
        PlanStep::Fetch(FetchStep::AcquireLayer { layer_id, transport_digest: Some(digest), .. })
        if layer_id == "runtime" && digest == manifest.layers["runtime"].transport_digest.as_ref().unwrap())));
    assert!(!pinned.steps.iter().any(|step| matches!(
        step,
        PlanStep::Fetch(FetchStep::ResolveGithubRelease { .. })
    )));
    let options = PlanOptions {
        runtime_channel: Some(RuntimeChannel::Dynamic),
        ..PlanOptions::default()
    };
    let dynamic = PlanBuilder::build(&manifest, &options, &execution_registry()).unwrap();
    assert_eq!(dynamic.runtime_channel, Some(RuntimeChannel::Dynamic));
    assert!(dynamic.steps.iter().any(|step| matches!(step, PlanStep::Fetch(FetchStep::ResolveGithubRelease { layer_id, .. }) if layer_id == "runtime")));
    assert!(!dynamic.steps.iter().any(|step| matches!(step, PlanStep::Transform(TransformStep::VerifyTransport { layer_id, .. }) if layer_id == "runtime")), "an arbitrary dynamic release must not be checked against the pinned container's digest");
}

#[test]
fn dynamic_channel_requires_a_declared_provider() {
    let options = PlanOptions {
        runtime_channel: Some(RuntimeChannel::Dynamic),
        ..PlanOptions::default()
    };
    assert_eq!(
        PlanBuilder::build(&canonical(SWITCH), &options, &execution_registry())
            .unwrap_err()
            .code,
        ErrorCode::InvalidRuntimeChannel
    );
}

#[test]
fn unavailable_required_capability_prevents_building_only_that_game_plan() {
    let mut blocked = canonical(SWITCH);
    blocked
        .capabilities
        .required
        .push("ext.unavailable.v1".into());
    assert_eq!(
        PlanBuilder::build(&blocked, &PlanOptions::default(), &execution_registry())
            .unwrap_err()
            .code,
        ErrorCode::MissingCapability
    );
    assert!(PlanBuilder::build(
        &canonical(AZIMUTH),
        &PlanOptions::default(),
        &execution_registry()
    )
    .is_ok());
}

#[test]
fn overlay_fallback_is_reflected_in_hardlink_projection_steps() {
    let mut manifest = canonical(SWITCH);
    manifest
        .capabilities
        .optional
        .push("vfs.overlay_mount.v2".into());
    manifest.fallback.push(CapabilityFallback {
        capability: "vfs.overlay_mount.v2".into(),
        strategy: FallbackStrategy::HardlinkProjection,
        requires: vec!["vfs.hardlink.v1".into()],
    });
    let plan =
        PlanBuilder::build(&manifest, &PlanOptions::default(), &execution_registry()).unwrap();
    assert!(plan
        .negotiation
        .uses_fallback(&FallbackStrategy::HardlinkProjection));
    assert!(plan.steps.iter().any(|step| matches!(
        step,
        PlanStep::Transform(TransformStep::ProjectLayer {
            strategy: ProjectionStrategy::Hardlink,
            ..
        })
    )));
    assert!(!plan.steps.iter().any(|step| matches!(
        step,
        PlanStep::Transform(TransformStep::MountOverlay { .. })
    )));
}

#[test]
fn local_locator_volume_mismatch_without_explicit_policy_is_typed() {
    let options = PlanOptions {
        local_base_volume: Some("D:".into()),
        library_volume: Some("C:".into()),
        ..PlanOptions::default()
    };
    assert_eq!(
        PlanBuilder::build(&canonical(LOCAL_BASE), &options, &execution_registry())
            .unwrap_err()
            .code,
        ErrorCode::CrossVolumeLinkNotSupported
    );
}

#[test]
fn explicit_cross_volume_fallback_is_recorded_for_local_projection() {
    let options = PlanOptions {
        local_base_volume: Some("D:".into()),
        library_volume: Some("C:".into()),
        ..PlanOptions::default()
    };
    let plan = PlanBuilder::build(&coop(), &options, &execution_registry()).unwrap();
    assert!(plan.steps.iter().any(|step| matches!(step,
        PlanStep::Transform(TransformStep::ProjectLayer { layer_id, strategy: ProjectionStrategy::CrossVolumeFallback, .. })
        if layer_id == "base")));
}

#[test]
fn wasm_template_fallback_emits_builtin_config_generation_and_no_wasm_step() {
    let mut document = value(SWITCH);
    document["capabilities"]["optional"] = json!(["ext.wasm_transform.v1"]);
    document["fallback"] = json!([{
        "capability": "ext.wasm_transform.v1", "strategy": "builtin_config_templates",
        "requires": ["core.config_templates.v1"]
    }]);
    document["post_setup_steps"] = json!([{
        "kind": "wasm_transform", "id": "generator", "layer_id": "runtime",
        "input_path": "runtime/config/source.bin", "output_path": "runtime/config/display.toml",
        "optional": true, "fallback_template": "display"
    }]);
    let manifest = canonical_json(&document).unwrap();
    let plan =
        PlanBuilder::build(&manifest, &PlanOptions::default(), &execution_registry()).unwrap();
    assert!(plan
        .negotiation
        .uses_fallback(&FallbackStrategy::BuiltinConfigTemplates));
    assert!(plan.steps.iter().any(|step| matches!(step,
        PlanStep::Env(EnvStep::RenderConfigTemplate { template, output_strategy: ProjectionStrategy::CopyMutable })
        if template.id == "display")));
    assert!(!plan
        .steps
        .iter()
        .any(|step| matches!(step, PlanStep::Env(EnvStep::WasmTransform { .. }))));
}

#[test]
fn coop_plan_checks_sync_and_integrity_before_starting_dependency_ordered_processes() {
    let plan = PlanBuilder::build(&coop(), &PlanOptions::default(), &execution_registry()).unwrap();
    let sync = plan
        .steps
        .iter()
        .position(|step| matches!(step, PlanStep::Exec(ExecStep::RequireManifestSync { .. })))
        .unwrap();
    let first_process = plan
        .steps
        .iter()
        .position(|step| matches!(step, PlanStep::Exec(ExecStep::StartProcess { .. })))
        .unwrap();
    assert!(sync < first_process);
    assert!(plan.steps.iter().enumerate().all(|(index, step)| !matches!(
        step,
        PlanStep::Exec(ExecStep::CheckCriticalFile { .. })
    ) || index < first_process));
    let process_ids: Vec<&str> = plan
        .steps
        .iter()
        .filter_map(|step| match step {
            PlanStep::Exec(ExecStep::StartProcess { process }) => Some(process.id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(process_ids, ["bridge", "minecraft", "cyberpunk"]);
    assert!(plan.steps.iter().any(|step| matches!(step,
        PlanStep::Exec(ExecStep::CheckCriticalFile { file })
        if file.workspace_path == "mods/coop-injector.dll" && file.role == CriticalFileRole::InjectorDll)));
    assert!(plan.steps.iter().any(|step| matches!(
        step,
        PlanStep::Env(EnvStep::PreserveSaves {
            snapshot_before_layer_update: true,
            preserve_originals_on_import: true,
            ..
        })
    )));
}

#[test]
fn local_locator_can_explicitly_place_workspace_on_the_discovered_base_volume() {
    let mut manifest = canonical(LOCAL_BASE);
    manifest.local_placement = LocalPlacementPolicy::WorkspaceOnBaseVolume;
    let options = PlanOptions {
        local_base_volume: Some("D:".into()),
        library_volume: Some("C:".into()),
        ..PlanOptions::default()
    };
    let plan = PlanBuilder::build(&manifest, &options, &execution_registry()).unwrap();
    assert_eq!(
        plan.placement.location,
        WorkspaceLocation::LocatedBaseVolume
    );
    assert_eq!(
        plan.placement.local_projection,
        ProjectionStrategy::Hardlink
    );
    assert_eq!(
        plan.placement.cas_projection,
        ProjectionStrategy::CrossVolumeFallback
    );
    assert!(!plan.placement.allow_full_base_copy);
    assert!(!plan.placement.requires_volume_check);
    let same_volume = PlanOptions {
        local_base_volume: Some("c:".into()),
        library_volume: Some("C:".into()),
        ..PlanOptions::default()
    };
    assert!(
        PlanBuilder::build(&canonical(LOCAL_BASE), &same_volume, &execution_registry()).is_ok()
    );
}

#[test]
fn dynamic_provider_plan_locks_runtime_binary_digests_independently_of_pinned_binaries() {
    let options = PlanOptions {
        runtime_channel: Some(RuntimeChannel::Dynamic),
        ..PlanOptions::default()
    };
    let plan = PlanBuilder::build(&canonical(AZIMUTH), &options, &execution_registry()).unwrap();
    assert!(plan.steps.iter().any(|step| matches!(step,
        PlanStep::Fetch(FetchStep::ResolveGithubRelease { required_lock, .. })
        if required_lock.release_and_asset && required_lock.transport_digest && required_lock.critical_files.len() == 2)));
    let runtime_checks: Vec<_> = plan
        .steps
        .iter()
        .filter_map(|step| match step {
            PlanStep::Exec(ExecStep::CheckCriticalFile { file }) if file.layer_id == "runtime" => {
                Some(file)
            }
            _ => None,
        })
        .collect();
    assert_eq!(runtime_checks.len(), 2);
    assert!(runtime_checks.iter().all(|file| matches!(
        file.expected,
        CriticalDigestExpectation::ProviderLock { .. }
    )));
}

#[test]
fn planner_revalidates_mutated_canonical_writable_paths() {
    let mut manifest = canonical(SWITCH);
    manifest
        .layers
        .get_mut("runtime")
        .unwrap()
        .writable_copy_paths = vec!["../outside/**".into()];
    assert_eq!(
        PlanBuilder::build(&manifest, &PlanOptions::default(), &execution_registry())
            .unwrap_err()
            .code,
        ErrorCode::InvalidPath
    );
}

#[test]
fn planner_rejects_canonical_layer_map_key_and_value_identity_mismatch() {
    let mut manifest = canonical(SWITCH);
    manifest.layers.get_mut("runtime").unwrap().id = "different-runtime".into();
    assert_eq!(
        PlanBuilder::build(&manifest, &PlanOptions::default(), &execution_registry())
            .unwrap_err()
            .code,
        ErrorCode::InvalidId
    );
}

#[test]
fn planner_rejects_missing_unknown_duplicate_and_out_of_order_canonical_layers() {
    let mut missing = canonical(SWITCH);
    missing.layer_order.retain(|id| id != "shader-cache");
    assert!(PlanBuilder::build(&missing, &PlanOptions::default(), &execution_registry()).is_err());

    let mut unknown = canonical(SWITCH);
    unknown.layer_order.push("unknown-layer".into());
    assert_eq!(
        PlanBuilder::build(&unknown, &PlanOptions::default(), &execution_registry())
            .unwrap_err()
            .code,
        ErrorCode::MissingLayer
    );

    let mut duplicate = canonical(SWITCH);
    duplicate.layer_order.push("base".into());
    assert_eq!(
        PlanBuilder::build(&duplicate, &PlanOptions::default(), &execution_registry())
            .unwrap_err()
            .code,
        ErrorCode::DuplicateId
    );

    let mut out_of_order = canonical(SWITCH);
    let runtime = out_of_order
        .layer_order
        .iter()
        .position(|id| id == "runtime")
        .unwrap();
    let firmware = out_of_order
        .layer_order
        .iter()
        .position(|id| id == "switch-firmware")
        .unwrap();
    out_of_order.layer_order.swap(runtime, firmware);
    assert!(PlanBuilder::build(
        &out_of_order,
        &PlanOptions::default(),
        &execution_registry()
    )
    .is_err());
}

#[test]
fn duplicate_security_relevant_json_fields_are_rejected_before_upcasting() {
    let duplicate_game_id = SWITCH.replacen(
        "\"id\": \"switch-zelda\"",
        "\"id\": \"switch-zelda\", \"id\": \"other-game\"",
        1,
    );
    assert_eq!(
        SchemaUpcaster::parse_game_manifest(&duplicate_game_id)
            .unwrap_err()
            .code,
        ErrorCode::InvalidJson
    );
    let duplicate_version = SWITCH.replacen(
        "\"schema_version\": \"1.0\"",
        "\"schema_version\": \"1.0\", \"schema_version\": \"1.0\"",
        1,
    );
    assert_eq!(
        SchemaUpcaster::parse_game_manifest(&duplicate_version)
            .unwrap_err()
            .code,
        ErrorCode::InvalidJson
    );
    let duplicate_manifest_hash = INDEX.replacen("\"manifest_blake3\": \"1111111111111111111111111111111111111111111111111111111111111111\"", "\"manifest_blake3\": \"1111111111111111111111111111111111111111111111111111111111111111\", \"manifest_blake3\": \"2222222222222222222222222222222222222222222222222222222222222222\"", 1);
    assert_eq!(
        SchemaUpcaster::upcast_source_index(&duplicate_manifest_hash)
            .unwrap_err()
            .code,
        ErrorCode::InvalidJson
    );
}

#[test]
fn undeclared_capabilities_do_not_bypass_actual_primitive_support_checks() {
    let mut manifest = canonical(SWITCH);
    manifest.capabilities.required.clear();
    manifest.capabilities.optional.clear();
    manifest.fallback.clear();
    assert_eq!(
        PlanBuilder::build(
            &manifest,
            &PlanOptions::default(),
            &CapabilityRegistry::new(["core.schema.v1", "core.plan.v1"])
        )
        .unwrap_err()
        .code,
        ErrorCode::MissingCapability
    );
    for capability in [
        "vfs.hardlink.v1",
        "resolver.https.v1",
        "core.config_templates.v1",
        "persistence.saves.v1",
        "proc.supervisor.v1",
    ] {
        let all = execution_registry();
        let restricted =
            CapabilityRegistry::new(all.iter().filter(|supported| *supported != capability));
        let error =
            PlanBuilder::build(&manifest, &PlanOptions::default(), &restricted).unwrap_err();
        assert_eq!(error.code, ErrorCode::MissingCapability, "{capability}");
        assert_eq!(error.details.get("capability"), Some(&json!(capability)));
    }
}

#[test]
fn planned_remote_mirrors_preserve_order_and_exclude_unsupported_transports() {
    let mut document = value(SWITCH);
    document["layers"][3]["source"]["mirrors"] = json!([
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567",
        "https://example.org/runtime-primary.zip",
        "https://example.org/runtime-backup.zip"
    ]);
    let manifest = canonical_json(&document).unwrap();
    let all = execution_registry();
    let registry = CapabilityRegistry::new(
        all.iter()
            .filter(|capability| *capability != "resolver.magnet.v1"),
    );
    let plan = PlanBuilder::build(&manifest, &PlanOptions::default(), &registry).unwrap();
    let mirrors = plan
        .steps
        .iter()
        .find_map(|step| match step {
            PlanStep::Fetch(FetchStep::AcquireLayer {
                layer_id,
                source: LayerSource::Remote { mirrors, .. },
                ..
            }) if layer_id == "runtime" => Some(mirrors),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        mirrors,
        &[
            "https://example.org/runtime-primary.zip",
            "https://example.org/runtime-backup.zip"
        ]
    );
}

#[test]
fn plan_enum_named_fields_serialize_in_camel_case() {
    let plan = PlanBuilder::build(
        &canonical(SWITCH),
        &PlanOptions::default(),
        &execution_registry(),
    )
    .unwrap();
    let step = plan
        .steps
        .iter()
        .find(|step| {
            matches!(step,
        PlanStep::Fetch(FetchStep::AcquireLayer { layer_id, .. }) if layer_id == "runtime")
        })
        .unwrap();
    let serialized = serde_json::to_value(step).unwrap();
    assert_eq!(serialized["stage"], "fetch");
    assert_eq!(serialized["operation"]["kind"], "acquireLayer");
    assert_eq!(serialized["operation"]["layerId"], "runtime");
    assert!(serialized["operation"].get("transportDigest").is_some());
    assert!(serialized["operation"].get("layer_id").is_none());
    assert!(serialized["operation"].get("transport_digest").is_none());
}

#[test]
fn wildcard_mount_paths_cannot_widen_writable_copy_projection() {
    for path in ["runtime*", "run?ime", "runtime[ab]"] {
        let mut document = value(SWITCH);
        document["layers"][3]["mount_path"] = json!(path);
        assert_eq!(
            assert_invalid(&document).code,
            ErrorCode::InvalidPath,
            "{path}"
        );
    }
    let mut document = value(SWITCH);
    document["layers"][3]["writable_copy_paths"] = json!(["**/*.cfg"]);
    let plan = PlanBuilder::build(
        &canonical_json(&document).unwrap(),
        &PlanOptions::default(),
        &execution_registry(),
    )
    .unwrap();
    assert!(plan.steps.iter().any(|step| matches!(step,
        PlanStep::Transform(TransformStep::MaterializeWritablePaths { layer_id, paths, .. })
        if layer_id == "runtime" && paths == &["runtime/**/*.cfg"])));
}

#[test]
fn selected_mod_overrides_critical_paths_case_insensitively_on_windows() {
    let mut document = value(SWITCH);
    document["layers"][0]["critical_files"] = json!([{
        "path": "core.dll", "blake3": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "role": "core_library"
    }]);
    document["layers"].as_array_mut().unwrap().push(json!({
        "id": "case-override", "kind": "mod", "depends_on": ["base"],
        "source": {"kind":"user_provided"}, "mount_path": "GAME",
        "optional": true, "default_enabled": true,
        "critical_files": [{"path":"CORE.DLL", "blake3":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee", "role":"core_library"}]
    }));
    let plan = PlanBuilder::build(
        &canonical_json(&document).unwrap(),
        &PlanOptions::default(),
        &execution_registry(),
    )
    .unwrap();
    let checks: Vec<_> = plan
        .steps
        .iter()
        .filter_map(|step| match step {
            PlanStep::Exec(ExecStep::CheckCriticalFile { file })
                if file.workspace_path.eq_ignore_ascii_case("game/core.dll") =>
            {
                Some(file)
            }
            _ => None,
        })
        .collect();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].layer_id, "case-override");
    assert_eq!(
        checks[0].expected,
        CriticalDigestExpectation::DeclaredBlake3 {
            value: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into()
        }
    );
}

fn runtime_patch_document() -> Value {
    let mut document = value(SWITCH);
    let input = document["layers"][3]["critical_files"][0]["blake3"].clone();
    document["capabilities"]["required"]
        .as_array_mut()
        .unwrap()
        .push(json!("patch.xdelta3.v1"));
    document["layers"].as_array_mut().unwrap().push(json!({
        "id":"runtime-patch", "kind":"patch", "depends_on":["runtime"],
        "source":{"kind":"remote", "mirrors":["https://example.org/fusion-fixtures/runtime-patch.xdelta"], "archive":"raw"},
        "transport_digest":{"algorithm":"blake3", "value":"ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"},
        "mount_path":"runtime",
        "patch":{
            "format":"xdelta3", "base_layer":"runtime",
            "input_path":"runtime/emulator.exe", "output_path":"runtime/emulator.exe",
            "input_digest":{"algorithm":"blake3", "value": input},
            "output_digest":{"algorithm":"blake3", "value":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"}
        }
    }));
    document
}

#[test]
fn critical_binary_patch_requires_updated_integrity_metadata() {
    assert_eq!(
        assert_invalid(&runtime_patch_document()).code,
        ErrorCode::InvalidManifest
    );
}

#[test]
fn critical_binary_patch_guards_only_the_updated_final_binary_digest() {
    let mut document = runtime_patch_document();
    document["layers"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["critical_files"] = json!([{
        "path":"emulator.exe", "blake3":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee", "role":"main_executable"
    }]);
    let plan = PlanBuilder::build(
        &canonical_json(&document).unwrap(),
        &PlanOptions::default(),
        &execution_registry(),
    )
    .unwrap();
    let checks: Vec<_> = plan
        .steps
        .iter()
        .filter_map(|step| match step {
            PlanStep::Exec(ExecStep::CheckCriticalFile { file })
                if file.workspace_path == "runtime/emulator.exe" =>
            {
                Some(file)
            }
            _ => None,
        })
        .collect();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].layer_id, "runtime-patch");
    assert_eq!(
        checks[0].expected,
        CriticalDigestExpectation::DeclaredBlake3 {
            value: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into()
        }
    );
}

#[test]
fn critical_binary_patch_rejects_contradictory_output_digest() {
    let mut document = runtime_patch_document();
    document["layers"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["critical_files"] = json!([{
        "path":"emulator.exe", "blake3":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd", "role":"main_executable"
    }]);
    assert_eq!(assert_invalid(&document).code, ErrorCode::InvalidDigest);
}

#[test]
fn version_one_rejects_multiple_local_locator_bases() {
    let mut document = value(LOCAL_BASE);
    let mut second = document["layers"][0].clone();
    second["id"] = json!("second-local-base");
    second["mount_path"] = json!("other-game");
    document["layers"].as_array_mut().unwrap().push(second);
    assert_eq!(assert_invalid(&document).code, ErrorCode::InvalidManifest);
}

#[test]
fn user_provided_archive_import_plans_read_only_extraction_and_defaults_to_raw() {
    let mut document = value(SWITCH);
    document["layers"][1]["source"]["archive"] = json!("zip");
    let manifest = canonical_json(&document).unwrap();
    assert!(matches!(
        manifest.layers["base"].source,
        LayerSource::UserProvided {
            archive: ArchiveFormat::Raw,
            ..
        }
    ));
    let plan =
        PlanBuilder::build(&manifest, &PlanOptions::default(), &execution_registry()).unwrap();
    assert!(plan.steps.iter().any(|step| matches!(step,
        PlanStep::Transform(TransformStep::ExtractLayer { layer_id, archive: ArchiveFormat::Zip, cas_access: CasAccess::ReadOnly })
        if layer_id == "switch-firmware")));
}

#[test]
fn every_final_critical_file_requires_exact_digest_trust_before_process_start() {
    let dynamic = PlanOptions {
        runtime_channel: Some(RuntimeChannel::Dynamic),
        ..PlanOptions::default()
    };
    for (manifest, options) in [
        (canonical(SWITCH), PlanOptions::default()),
        (canonical(AZIMUTH), dynamic),
        (coop(), PlanOptions::default()),
    ] {
        let plan = PlanBuilder::build(&manifest, &options, &execution_registry()).unwrap();
        let first_start = plan
            .steps
            .iter()
            .position(|step| {
                matches!(
                    step,
                    PlanStep::Exec(ExecStep::StartRuntime { .. } | ExecStep::StartProcess { .. })
                )
            })
            .unwrap();
        let checks: Vec<_> = plan
            .steps
            .iter()
            .enumerate()
            .filter_map(|(index, step)| match step {
                PlanStep::Exec(ExecStep::CheckCriticalFile { file }) => Some((index, file)),
                _ => None,
            })
            .collect();
        let trust: Vec<_> = plan
            .steps
            .iter()
            .enumerate()
            .filter_map(|(index, step)| match step {
                PlanStep::Exec(ExecStep::RequireExecutableTrust { file }) => Some((index, file)),
                _ => None,
            })
            .collect();
        assert!(!checks.is_empty());
        assert_eq!(trust.len(), checks.len());
        let last_check = checks.iter().map(|(index, _)| *index).max().unwrap();
        for (index, file) in trust {
            assert!(last_check < index && index < first_start);
            assert!(checks.iter().any(|(_, checked)| *checked == file));
        }
    }
}
