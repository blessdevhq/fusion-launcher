use super::*;
use crate::fusion_source::{CapabilityRegistry, PlanBuilder, PlanOptions, SchemaUpcaster};

#[test]
fn program_schema_reader_canonical_retains_descriptor_without_enabling_execution() {
    let descriptor: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/creator/programs/supertuxkart-1.5.json"
    ))
    .unwrap();
    let mut wire = serde_json::json!({
        "schema_version":"1.1", "id":"generic-program", "title":"Generic program", "version":"1",
        "creator":{"program_descriptor":descriptor},
        "layers":[{"id":descriptor["components"][0]["layer_id"],"kind":"base",
            "source":{"kind":"user_provided"}}]
    });
    let parsed = SchemaUpcaster::parse_game_manifest(&wire.to_string()).unwrap();
    assert_eq!(
        serde_json::to_value(&parsed).unwrap()["creator"],
        wire["creator"]
    );
    let canonical =
        SchemaUpcaster::upcast_game_manifest("test-source", &wire.to_string(), &BTreeMap::new())
            .unwrap();
    let canonical_wire = serde_json::to_value(&canonical).unwrap();
    assert_eq!(canonical_wire["creator"], wire["creator"]);
    let original_hash = blake3::hash(&serde_json::to_vec(&canonical).unwrap());
    let mut changed = canonical_wire;
    changed["creator"]["program_descriptor"]["identity"]["title"] = "Another title".into();
    let changed: crate::fusion_source::CanonicalGameManifest =
        serde_json::from_value(changed).unwrap();
    assert_ne!(
        blake3::hash(&serde_json::to_vec(&changed).unwrap()),
        original_hash
    );
    assert_eq!(
        PlanBuilder::build(
            &canonical,
            &PlanOptions::default(),
            &CapabilityRegistry::new(["core.schema.v1"]),
        )
        .unwrap_err()
        .code,
        ErrorCode::ActionUnavailable
    );
    let duplicate = wire.to_string().replacen(
        "\"schema_version\":\"1.0\"",
        "\"schema_version\":\"1.0\",\"schema_version\":\"1.0\"",
        1,
    );
    assert!(
        SchemaUpcaster::parse_game_manifest(&duplicate).is_err(),
        "untagged creator variants must not erase duplicate keys"
    );
    wire["creator"]["program_descriptor"]["controls_profiles"][0] = serde_json::json!({
        "id":"defaults", "title":"Defaults", "kind":"eden_param_package_v1",
        "device":"keyboard_mouse", "config_id":"settings",
        "bindings":{"action":{"kind":"keyboard","code":"KeyA"}}
    });
    let valid_bindings = wire.to_string();
    assert!(SchemaUpcaster::parse_game_manifest(&valid_bindings).is_ok());
    let duplicate_bindings = valid_bindings.replacen(
        "\"bindings\":{\"action\":{\"code\":\"KeyA\",\"kind\":\"keyboard\"}}",
        "\"bindings\":{\"action\":{\"code\":\"KeyA\",\"kind\":\"keyboard\"},\"action\":{\"code\":\"KeyB\",\"kind\":\"keyboard\"}}",
        1,
    );
    assert_ne!(duplicate_bindings, valid_bindings);
    assert!(
        SchemaUpcaster::parse_game_manifest(&duplicate_bindings).is_err(),
        "portable binding maps must not discard duplicate action keys"
    );
    wire["schema_version"] = "1.0".into();
    assert!(SchemaUpcaster::parse_game_manifest(&wire.to_string()).is_err());
}
