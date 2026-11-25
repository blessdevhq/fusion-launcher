use super::*;

const FIXTURE: &str = include_str!("../../../../fixtures/creator/programs/eden-0.2.1.json");
fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}

#[test]
fn program_schema_external_origins_require_uri_syntax_not_iri_normalization() {
    for url in [
        "https://example.com/a\u{feff}.zip",
        "https://example.com/a\u{85}.zip",
        "https://example.com/a|b",
        "https://example.com/a%XX",
        "https://example.com/a\0b",
        "https://[bad]/a.zip",
        "https://@example.com/a.zip",
        " https://example.com/a.zip",
        "not-a-url",
    ] {
        let mut value = fixture();
        value["components"][0]["origin"] = serde_json::json!({
            "kind":"external", "url":url,
            "transport_blake3":"0".repeat(64), "inventory_blake3":"1".repeat(64)
        });
        assert!(
            ProgramDescriptor::read_json(&value.to_string()).is_err(),
            "{url:?}"
        );
    }
    for url in [
        "https://example.com/%E2%9C%93.zip",
        "HTTPS://example.com:443/a.zip?download=1#asset",
        "https://[::1]/a.zip",
    ] {
        let mut value = fixture();
        value["components"][0]["origin"] = serde_json::json!({
            "kind":"external", "url":url,
            "transport_blake3":"0".repeat(64), "inventory_blake3":"1".repeat(64)
        });
        ProgramDescriptor::read_json(&value.to_string()).unwrap();
    }
}

#[test]
fn program_schema_project_versions_bounds_and_private_fields_are_strict() {
    let mut value = serde_json::json!({
        "schema_version":"1.1", "project_id":"project", "revision":0, "title":"Project",
        "source_id":"source", "game_id":"game", "game_version":"1", "program_descriptor":fixture()
    });
    for revision in [0, MAX_REVISION] {
        value["revision"] = revision.into();
        let project = CreatorProgramProject::read_json(&value.to_string()).unwrap();
        project.validate().unwrap();
        assert_eq!(project.revision, revision);
        assert!(
            serde_json::from_value::<crate::fusion_creator::CreatorProject>(value.clone()).is_err()
        );
    }
    for revision in [
        serde_json::json!(-1),
        serde_json::json!(0.5),
        serde_json::json!(MAX_REVISION + 1),
    ] {
        value["revision"] = revision;
        assert!(CreatorProgramProject::read_json(&value.to_string()).is_err());
    }
    value["revision"] = 0.into();
    for key in ["components", "adapter", "localPath", "signingKeyRef"] {
        let mut private = value.clone();
        private[key] = serde_json::json!({});
        assert!(CreatorProgramProject::read_json(&private.to_string()).is_err());
    }
    value["schema_version"] = "1.0".into();
    assert!(CreatorProgramProject::read_json(&value.to_string()).is_err());
    let manifest = serde_json::json!({"program_descriptor":fixture()});
    assert_eq!(
        serde_json::to_value(ProgramCreatorManifest::read_json(&manifest.to_string()).unwrap())
            .unwrap(),
        manifest
    );
    let mut mixed = manifest;
    mixed["adapter"] = serde_json::json!({"id":"native","version":"1"});
    assert!(ProgramCreatorManifest::read_json(&mixed.to_string()).is_err());
}

#[test]
fn program_schema_structural_constants_paths_and_scalar_bounds() {
    for (pointer, invalid) in [
        ("/schema_version", serde_json::json!("1.1")),
        ("/executor/id", serde_json::json!("native")),
        ("/executor/version", serde_json::json!("2.0")),
        ("/identity/os", serde_json::json!("linux")),
        ("/identity/architecture", serde_json::json!("unknown")),
        ("/configs/0/preserve_unmanaged", serde_json::json!(false)),
        ("/persistence/0/export", serde_json::json!("always")),
        (
            "/configs/0/keys/0/allowed_values",
            serde_json::json!([0, 0.0]),
        ),
        (
            "/configs/0/keys/0/allowed_values",
            serde_json::json!([MAX_REVISION + 1]),
        ),
        (
            "/configs/0/keys/0/allowed_values",
            serde_json::json!(["bad\u{85}scalar"]),
        ),
    ] {
        let mut value = fixture();
        *value.pointer_mut(pointer).unwrap() = invalid;
        assert!(
            ProgramDescriptor::read_json(&value.to_string()).is_err(),
            "{pointer}"
        );
    }
    for path in [
        "",
        "/absolute",
        "C:/host",
        "../escape",
        "a/../b",
        "a//b",
        "a/./b",
        "a\\b",
        "CON",
        "aux.txt",
        "dir/LPT9.ini",
        "trailing.",
        "trailing ",
        "wild*",
        "wild?",
        "a\0b",
        "a\u{7f}b",
        "a\u{9f}b",
    ] {
        let mut value = fixture();
        value["entrypoint"]["executable"] = path.into();
        assert!(
            ProgramDescriptor::read_json(&value.to_string()).is_err(),
            "{path:?}"
        );
    }
    let mut unicode = fixture();
    unicode["identity"]["id"] = "🎮".repeat(128).into();
    unicode["entrypoint"]["executable"] = "🎮".repeat(512).into();
    ProgramDescriptor::read_json(&unicode.to_string()).unwrap();
    unicode["identity"]["id"] = "🎮".repeat(129).into();
    assert!(ProgramDescriptor::read_json(&unicode.to_string()).is_err());
}

#[test]
fn program_schema_limits_are_bytes_and_duplicate_keys_fail_before_maps() {
    let padded = format!(
        "{}{}",
        " ".repeat(MAX_PROGRAM_BYTES - FIXTURE.len()),
        FIXTURE
    );
    ProgramDescriptor::read_json(&padded).unwrap();
    assert_eq!(
        ProgramDescriptor::read_json(&(padded + " "))
            .unwrap_err()
            .code,
        ErrorCode::InvalidJson
    );
    for duplicate in [
        r#"{"a":1,"a":2}"#,
        r#"{"nested":{"action":1,"action":2}}"#,
        r#"[{"nested":{"action":1,"action":2}}]"#,
    ] {
        assert_eq!(
            parse_unique_json(duplicate, MAX_PROGRAM_BYTES)
                .unwrap_err()
                .code,
            ErrorCode::InvalidJson
        );
    }
    assert!(parse_unique_json("{} {}", MAX_PROGRAM_BYTES).is_err());
    let mut descriptor = ProgramDescriptor::read_json(FIXTURE).unwrap();
    descriptor.components[0].source_root = "../escape".into();
    assert!(descriptor.validate().is_err());
}

#[test]
fn program_schema_collection_cardinality_unknown_fields_and_environment() {
    for (field, max) in [
        ("components", 64),
        ("required_files", 256),
        ("projection_rules", 4096),
        ("directories", 64),
        ("configs", 16),
        ("build_profiles", 32),
        ("controls_profiles", 16),
        ("persistence", 64),
    ] {
        let mut value = fixture();
        let item = value[field]
            .as_array()
            .unwrap()
            .first()
            .cloned()
            .unwrap_or_else(|| {
                if field == "projection_rules" {
                    serde_json::json!({"path":"data.bin", "winning_layer":"base"})
                } else {
                    "dir".into()
                }
            });
        value[field] = Value::Array(vec![item; max + 1]);
        assert!(
            ProgramDescriptor::read_json(&value.to_string()).is_err(),
            "{field}"
        );
    }
    for name in [
        "PATH",
        "COMSPEC",
        "APPDATA",
        "PYTHONPATH",
        "NODE_OPTIONS",
        "RUSTC_WRAPPER",
        "lowercase",
    ] {
        let mut value = fixture();
        value["entrypoint"]["environment_paths"] = serde_json::json!([{"name":name,"path":""}]);
        assert!(
            ProgramDescriptor::read_json(&value.to_string()).is_err(),
            "{name}"
        );
    }
    for pointer in [
        "/components/0",
        "/configs/0/seed",
        "/entrypoint",
        "/identity",
    ] {
        let mut value = fixture();
        value.pointer_mut(pointer).unwrap()["private_path"] = "C:/private".into();
        assert!(
            ProgramDescriptor::read_json(&value.to_string()).is_err(),
            "{pointer}"
        );
    }
}
