use super::*;

type Inventories = BTreeMap<(String, String), Vec<CasFile>>;

fn legacy_creator(
    manifest: &mut CanonicalGameManifest,
) -> &mut crate::fusion_source::creator_schema::CreatorManifest {
    manifest.creator.as_mut().unwrap().as_legacy_mut().unwrap()
}

fn fixture() -> (CanonicalGameManifest, Inventories) {
    let manifest = SchemaUpcaster::upcast_game_manifest(
        "projection-source",
        &serde_json::json!({
            "schema_version":"1.0", "id":"game", "title":"Game", "version":"1",
            "launch":{"id":"play", "executable":"app.exe", "role":"primary"},
            "layers":[
                {"id":"base", "kind":"base", "critical_files":[{"path":"app.exe", "blake3":"0".repeat(64), "role":"main_executable"}], "source":{"kind":"remote", "mirrors":["https://example.com/base.zip"], "archive":"zip"}, "transport_digest":{"algorithm":"blake3","value":"0".repeat(64)}},
                {"id":"addon", "kind":"addon", "depends_on":["base"], "source":{"kind":"remote", "mirrors":["https://example.com/addon.zip"], "archive":"zip"}, "transport_digest":{"algorithm":"blake3","value":"1".repeat(64)}}
            ]
        })
        .to_string(),
        &BTreeMap::new(),
    )
    .unwrap();
    let file = CasFile {
        path: "data.bin".into(),
        blake3: blake3::hash(b"same").to_hex().to_string(),
        size: 4,
    };
    let inventories = BTreeMap::from([
        (("game".into(), "base".into()), vec![file.clone()]),
        (("game".into(), "addon".into()), vec![file]),
    ]);
    (manifest, inventories)
}

fn creator_fixture() -> (CanonicalGameManifest, Inventories) {
    let (mut manifest, mut inventories) = fixture();
    manifest.schema_version = "1.1".into();
    manifest.layers.get_mut("addon").unwrap().optional = true;
    manifest.creator = Some(serde_json::from_value(serde_json::json!({
        "adapter":{"id":"portable", "version":"1"},
        "components":[
            {"layer_id":"base", "origin":{"kind":"managed", "artifact_ref":"base", "redistribution_allowed":true}},
            {"layer_id":"addon", "origin":{"kind":"managed", "artifact_ref":"addon", "redistribution_allowed":true}}
        ],
        "build_profiles":[
            {"id":"safe", "title":"Safe", "layer_ids":["base"], "controls_profile_ids":["keyboard"]},
            {"id":"conflict", "title":"Conflict", "layer_ids":["base", "addon"], "controls_profile_ids":["keyboard"]}
        ],
        "controls_profiles":[{"id":"keyboard", "title":"Keyboard", "device":"keyboard_mouse", "bindings":{}}],
        "projection_rules":[]
    })).unwrap());
    inventories
        .get_mut(&("game".into(), "base".into()))
        .unwrap()
        .push(CasFile {
            path: "app.exe".into(),
            blake3: "0".repeat(64),
            size: 8,
        });
    inventories
        .get_mut(&("game".into(), "addon".into()))
        .unwrap()[0]
        .blake3 = "1".repeat(64);
    (manifest, inventories)
}

#[test]
fn packer_preflight_checks_every_declared_profile_collision() {
    let (manifest, inventories) = creator_fixture();
    assert_eq!(
        validate_layer_inventory(&manifest, &inventories)
            .unwrap_err()
            .code,
        ErrorCode::InvalidManifest,
        "the later finite profile needs an Explicit11 winner, not legacy layer order"
    );
}

#[test]
fn packer_inventory_projection_rejects_inconsistent_verified_sizes() {
    let (manifest, mut inventories) = fixture();
    inventories
        .get_mut(&("game".into(), "addon".into()))
        .unwrap()[0]
        .size = 5;
    assert_eq!(
        validate_layer_inventory(&manifest, &inventories)
            .unwrap_err()
            .code,
        ErrorCode::InvalidManifest
    );
}

#[test]
fn packer_inventory_projection_keeps_legacy_last_winner_and_critical_guard() {
    let (mut manifest, mut inventories) = fixture();
    let winning = blake3::hash(b"new").to_hex().to_string();
    let addon = &mut inventories
        .get_mut(&("game".into(), "addon".into()))
        .unwrap()[0];
    addon.path = "DATA.bin".into();
    addon.blake3 = winning.clone();
    addon.size = 3;
    validate_layer_inventory(&manifest, &inventories).unwrap();
    manifest
        .layers
        .get_mut("base")
        .unwrap()
        .critical_files
        .push(CriticalFileSpec {
            path: "data.bin".into(),
            blake3: winning,
            role: CriticalFileRole::CoreLibrary,
        });
    validate_layer_inventory(&manifest, &inventories).unwrap();
    manifest.layers.get_mut("base").unwrap().critical_files[1].blake3 = "0".repeat(64);
    assert_eq!(
        validate_layer_inventory(&manifest, &inventories)
            .unwrap_err()
            .code,
        ErrorCode::CorruptedCriticalBinary
    );
}

fn compatible_creator_fixture() -> (CanonicalGameManifest, Inventories) {
    let (manifest, mut inventories) = creator_fixture();
    let digest = inventories[&("game".into(), "base".into())][0]
        .blake3
        .clone();
    inventories
        .get_mut(&("game".into(), "addon".into()))
        .unwrap()[0]
        .blake3 = digest;
    (manifest, inventories)
}

fn profile_inventories(
    manifest: &CanonicalGameManifest,
    inventories: &Inventories,
) -> Vec<ProjectionLayer> {
    manifest
        .layers
        .values()
        .filter_map(|layer| {
            inventories
                .get(&(manifest.id.clone(), layer.id.clone()))
                .map(|files| ProjectionLayer {
                    layer_id: layer.id.clone(),
                    mount_path: layer.mount_path.clone(),
                    verified_inventory: files.clone(),
                })
        })
        .collect()
}

#[test]
fn finite_profiles_resolve_every_pair_without_granting_execution() {
    use crate::fusion_creator::profiles::{resolve_declared_profiles, validate_profile_selection};
    let (manifest, inventories) = compatible_creator_fixture();
    validate_layer_inventory(&manifest, &inventories).unwrap();
    let profiles =
        resolve_declared_profiles(&manifest, &profile_inventories(&manifest, &inventories))
            .unwrap();
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles["safe"].files["data.bin"].winning_layer, "base");
    assert_eq!(
        profiles["conflict"].files["data.bin"].winning_layer,
        "addon"
    );
    assert_eq!(
        profiles["conflict"].files["data.bin"].equivalent_layers,
        ["addon", "base"]
    );
    let options = PlanOptions {
        build_profile_id: Some("safe".into()),
        controls_profile_id: Some("keyboard".into()),
        ..PlanOptions::default()
    };
    assert_eq!(
        validate_profile_selection(&manifest, &options)
            .unwrap()
            .unwrap()
            .layer_ids,
        ["base"]
    );
    assert_eq!(
        PlanBuilder::build(&manifest, &options, &CapabilityRegistry::current_client())
            .unwrap_err()
            .code,
        ErrorCode::ActionUnavailable
    );
}

#[test]
fn finite_profile_selection_rejects_unknown_half_pairs_and_all_addon_overrides() {
    use crate::fusion_creator::profiles::validate_profile_selection;
    let (manifest, _) = compatible_creator_fixture();
    for (build, controls, addons) in [
        (Some("safe"), None, None),
        (None, Some("keyboard"), None),
        (Some("missing"), Some("keyboard"), None),
        (Some("safe"), Some("missing"), None),
        (Some("safe"), Some("keyboard"), Some(BTreeSet::new())),
        (None, None, Some(BTreeSet::new())),
    ] {
        let options = PlanOptions {
            build_profile_id: build.map(str::to_owned),
            controls_profile_id: controls.map(str::to_owned),
            enabled_addons: addons,
            ..PlanOptions::default()
        };
        assert_eq!(
            validate_profile_selection(&manifest, &options)
                .unwrap_err()
                .code,
            ErrorCode::InvalidManifest
        );
    }
}

#[test]
fn finite_profile_metadata_and_inventory_cannot_be_silently_incomplete() {
    let (original, inventories) = compatible_creator_fixture();
    let mut manifest = original.clone();
    let duplicate = legacy_creator(&mut manifest).build_profiles[0].clone();
    legacy_creator(&mut manifest).build_profiles.push(duplicate);
    assert!(validate_layer_inventory(&manifest, &inventories).is_err());
    let mut manifest = original.clone();
    legacy_creator(&mut manifest).build_profiles[0]
        .layer_ids
        .push("base".into());
    assert!(validate_layer_inventory(&manifest, &inventories).is_err());
    let mut manifest = original.clone();
    legacy_creator(&mut manifest).build_profiles[0].controls_profile_ids = vec!["unknown".into()];
    assert!(validate_layer_inventory(&manifest, &inventories).is_err());
    let mut manifest = original.clone();
    legacy_creator(&mut manifest).build_profiles[0].layer_ids = vec!["addon".into()];
    assert!(validate_layer_inventory(&manifest, &inventories).is_err());
    let mut missing = inventories.clone();
    missing.remove(&("game".into(), "addon".into()));
    assert!(validate_layer_inventory(&original, &missing).is_err());
    let mut manifest = original;
    legacy_creator(&mut manifest).projection_rules.push(
        crate::fusion_source::creator_schema::ProjectionRule {
            path: "data.bin".into(),
            winning_layer: "addon".into(),
        },
    );
    assert!(
        validate_layer_inventory(&manifest, &inventories).is_err(),
        "unused/excluded decisions must not be silently filtered"
    );
}

#[test]
fn profile_integrity_uses_the_winning_binary_provider_not_layer_order() {
    use crate::fusion_creator::profiles::resolve_declared_profiles;
    let (mut manifest, mut inventories) = compatible_creator_fixture();
    legacy_creator(&mut manifest).build_profiles.remove(0);
    let digest = blake3::hash(b"replacement binary").to_hex().to_string();
    inventories
        .get_mut(&("game".into(), "addon".into()))
        .unwrap()
        .push(CasFile {
            path: "app.exe".into(),
            blake3: digest.clone(),
            size: 18,
        });
    manifest
        .layers
        .get_mut("addon")
        .unwrap()
        .critical_files
        .push(CriticalFileSpec {
            path: "app.exe".into(),
            blake3: digest.clone(),
            role: CriticalFileRole::MainExecutable,
        });
    legacy_creator(&mut manifest).projection_rules.push(
        crate::fusion_source::creator_schema::ProjectionRule {
            path: "app.exe".into(),
            winning_layer: "addon".into(),
        },
    );
    let projection =
        resolve_declared_profiles(&manifest, &profile_inventories(&manifest, &inventories))
            .unwrap();
    assert_eq!(
        projection["conflict"].files["app.exe"].winning_layer,
        "addon"
    );
    assert_eq!(projection["conflict"].files["app.exe"].blake3, digest);
    manifest.layers.get_mut("addon").unwrap().critical_files[0].role =
        CriticalFileRole::CoreLibrary;
    assert_eq!(
        validate_layer_inventory(&manifest, &inventories)
            .unwrap_err()
            .code,
        ErrorCode::InvalidManifest
    );
    for field in ["runtime", "orchestrator"] {
        let mut other = manifest.clone();
        other.launch = None;
        if field == "runtime" {
            other.runtime = Some(
                serde_json::from_value(serde_json::json!({
                    "layer_id":"addon", "executable":"app.exe"
                }))
                .unwrap(),
            );
        } else {
            other.orchestrator = Some(
                serde_json::from_value(serde_json::json!({
                    "processes":[{"id":"main","role":"primary","executable":"app.exe"}]
                }))
                .unwrap(),
            );
        }
        assert_eq!(
            validate_layer_inventory(&other, &inventories)
                .unwrap_err()
                .code,
            ErrorCode::InvalidManifest,
            "{field} must use its actual provider's role"
        );
    }
    manifest.layers.get_mut("addon").unwrap().critical_files[0].role =
        CriticalFileRole::MainExecutable;
    let mut ambiguous = manifest.clone();
    ambiguous.layers.get_mut("addon").unwrap().source = serde_json::from_value(
        serde_json::json!({"kind":"local_locator","locator":{"store":"manual",
            "path_hints":[],"key_files":[{"path":"app.exe","blake3":digest,
                "role":"core_library"}]}}),
    )
    .unwrap();
    assert_eq!(
        validate_layer_inventory(&ambiguous, &inventories)
            .unwrap_err()
            .code,
        ErrorCode::InvalidManifest,
        "conflicting provider declarations must fail closed"
    );
    manifest.layers.get_mut("addon").unwrap().critical_files[0].blake3 = "0".repeat(64);
    assert_eq!(
        validate_layer_inventory(&manifest, &inventories)
            .unwrap_err()
            .code,
        ErrorCode::CorruptedCriticalBinary
    );
    manifest
        .layers
        .get_mut("addon")
        .unwrap()
        .critical_files
        .clear();
    assert!(validate_layer_inventory(&manifest, &inventories).is_err());
}
