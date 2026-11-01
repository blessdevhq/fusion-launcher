use super::*;
use crate::fusion_source::cas::CasFile;
use serde_json::json;

#[test]
fn binds_complete_inventories_to_exact_descriptor_mounts() {
    let (d, inventory) = fixture();
    let mut cases = vec![("missing", vec![])];
    let mut inputs = inventory.clone();
    inputs.push(inputs[0].clone());
    cases.push(("duplicate", inputs));
    let mut inputs = inventory.clone();
    inputs[0].mount_path = "elsewhere".into();
    cases.push(("wrong mount", inputs));
    let mut inputs = inventory.clone();
    inputs[0].mount_path = "APP".into();
    cases.push(("mount alias", inputs));
    let mut inputs = inventory.clone();
    let mut extra = inputs[0].clone();
    extra.layer_id = "undeclared".into();
    inputs.push(extra);
    cases.push(("extra", inputs));
    for (name, inputs) in cases {
        assert!(compile_descriptor(&d, &inputs).is_err(), "accepted {name}");
    }
    let mut d = d;
    let mut unused = d.components[0].clone();
    unused.id = "unused".into();
    unused.layer_id = "unused".into();
    d.components.push(unused);
    assert!(
        compile_descriptor(&d, &inventory).is_err(),
        "missing unselected component"
    );
}

#[test]
fn requires_real_files_main_role_and_directory_cwd_in_every_profile() {
    let (base, inventory) = fixture();
    let mut cases = Vec::new();
    let mut d = base.clone();
    d.required_files.push(ProgramFile {
        path: "app/missing.dat".into(),
        role: ProgramFileRole::Data,
    });
    cases.push(("missing data", d));
    let mut d = base.clone();
    d.required_files[0].role = ProgramFileRole::Data;
    cases.push(("missing main role", d));
    let mut d = base.clone();
    d.required_files.push(ProgramFile {
        path: "app/bin/demo.exe".into(),
        role: ProgramFileRole::MainExecutable,
    });
    cases.push(("duplicate required", d));
    let mut d = base.clone();
    d.entrypoint.executable = "app/bin/absent.exe".into();
    cases.push(("mismatched entrypoint", d));
    let mut d = base.clone();
    d.entrypoint.working_directory = "app/bin/demo.exe".into();
    cases.push(("file cwd", d));
    let mut d = base.clone();
    d.entrypoint.working_directory = "app/bin/demo.exe/child".into();
    cases.push(("cwd below file", d));
    let mut d = base.clone();
    d.entrypoint.working_directory = "absent".into();
    cases.push(("missing cwd", d));
    let mut d = base.clone();
    d.entrypoint.executable = "app/bin/demo.cmd".into();
    d.required_files[0].path = d.entrypoint.executable.clone();
    cases.push(("script entrypoint", d));
    let mut d = base.clone();
    let mut component = d.components[0].clone();
    component.id = "other".into();
    component.layer_id = "other".into();
    d.components.push(component);
    let mut p = d.build_profiles[0].clone();
    p.id = "broken-unselected".into();
    p.layer_ids = vec!["other".into()];
    d.build_profiles.push(p);
    let mut inputs = inventory.clone();
    inputs.push(ProjectionLayer {
        layer_id: "other".into(),
        mount_path: "app".into(),
        verified_inventory: vec![file("unrelated.dat", b"data")],
    });
    assert!(
        compile_descriptor(&d, &inputs).is_err(),
        "unselected missing executable"
    );
    for (name, d) in cases {
        assert!(
            compile_descriptor(&d, &inventory).is_err(),
            "accepted {name}"
        );
    }
    let mut d = base;
    d.directories.push("empty-cwd".into());
    d.entrypoint.working_directory = "empty-cwd".into();
    compile_descriptor(&d, &inventory).unwrap();
    d.entrypoint.working_directory.clear();
    compile_descriptor(&d, &inventory).unwrap();
}

#[test]
fn preserves_actual_resolver_winner_and_rejects_unresolved_profiles() {
    let (mut d, mut inputs) = fixture();
    let mut component = d.components[0].clone();
    component.id = "update".into();
    component.layer_id = "update".into();
    d.components.push(component);
    d.build_profiles[0].layer_ids.push("update".into());
    let updated = file("bin/demo.exe", b"updated executable");
    inputs.push(ProjectionLayer {
        layer_id: "update".into(),
        mount_path: "app".into(),
        verified_inventory: vec![updated.clone()],
    });
    assert!(compile_descriptor(&d, &inputs).is_err());
    d.projection_rules.push(ProgramProjectionRule {
        path: "app/bin/demo.exe".into(),
        winning_layer: "update".into(),
    });
    let compiled = compile_descriptor(&d, &inputs).unwrap();
    let winner = &compiled.projections["standard"].files["app/bin/demo.exe"];
    assert_eq!(winner.winning_layer, "update");
    assert_eq!(winner.blake3, updated.blake3);
    assert_eq!(winner.size, updated.size);
    assert_eq!(winner.source_relative_path, "bin/demo.exe");
    let mut unselected = d.build_profiles[0].clone();
    unselected.id = "without-winner".into();
    unselected.layer_ids = vec!["runtime".into()];
    d.build_profiles.push(unselected);
    assert!(
        compile_descriptor(&d, &inputs).is_err(),
        "rule invalid in unselected profile"
    );
}

#[test]
fn rejects_mutable_immutable_and_file_directory_collisions() {
    let (base, inventory) = fixture();
    let mut cases = Vec::new();
    for path in ["app/bin/demo.exe", "app/bin/demo.exe/child.ini", "app/bin"] {
        let mut d = base.clone();
        d.configs[0].path = path.into();
        d.persistence.clear();
        cases.push((format!("config {path}"), d));
    }
    for path in ["app/bin/demo.exe", "app/bin/demo.exe/child", "app"] {
        let mut d = base.clone();
        d.persistence[0].path = path.into();
        d.persistence[0].kind = ProgramPersistenceKind::Directory;
        cases.push((format!("persistence {path}"), d));
    }
    for path in [
        "app/bin/demo.exe",
        "app/bin/demo.exe/child",
        "userdata/main.ini",
    ] {
        let mut d = base.clone();
        d.directories.push(path.into());
        cases.push((format!("directory {path}"), d));
    }
    let mut d = base.clone();
    let mut c = d.configs[0].clone();
    c.id = "alias".into();
    c.keys.clear();
    d.configs.push(c);
    cases.push(("duplicate config path".into(), d));
    for (path, kind) in [
        ("userdata/main.ini", ProgramPersistenceKind::File),
        ("userdata", ProgramPersistenceKind::Directory),
        ("userdata/main.ini/child", ProgramPersistenceKind::File),
    ] {
        let mut d = base.clone();
        let mut p = d.persistence[0].clone();
        p.path = path.into();
        p.kind = kind;
        d.persistence.push(p);
        cases.push((format!("overlapping roots {path}"), d));
    }
    let mut d = base.clone();
    d.persistence[0].path = "userdata".into();
    d.persistence[0].kind = ProgramPersistenceKind::Directory;
    cases.push(("directory persistence contains config".into(), d));
    for (name, d) in cases {
        assert!(
            compile_descriptor(&d, &inventory).is_err(),
            "accepted {name}"
        );
    }
    let mut d = base;
    d.persistence.push(ProgramPersistence {
        path: "saves".into(),
        kind: ProgramPersistenceKind::Directory,
        policy: ProgramPersistencePolicy::PreserveLatest,
        export: ProgramPersistenceExport::Never,
    });
    d.directories.push("saves".into());
    compile_descriptor(&d, &inventory).unwrap();
}

#[test]
fn rejects_windows_aliases_and_revalidates_rust_constructed_shapes() {
    let (base, inventory) = fixture();
    for path in [
        "USERDATA",
        "app/BIN/new",
        "COM¹",
        "LPT².txt",
        "CONIN$",
        "CONOUT$",
        "bad.",
        "../escape",
        "a:b",
        "a\\b",
    ] {
        let mut d = base.clone();
        d.directories.push(path.into());
        assert!(
            compile_descriptor(&d, &inventory).is_err(),
            "accepted {path}"
        );
    }
    let mut d = base.clone();
    d.entrypoint.working_directory = "APP/bin".into();
    assert!(compile_descriptor(&d, &inventory).is_err());
    let mut d = base.clone();
    d.configs[0].preserve_unmanaged = false;
    assert!(validate_descriptor_metadata(&d).is_err());
    let mut d = base.clone();
    d.build_profiles.clear();
    assert!(compile_descriptor(&d, &inventory).is_err());
    let mut d = base;
    d.components[0].source_root = "COM³/data".into();
    assert!(validate_descriptor_metadata(&d).is_err());
}

#[test]
fn rejects_duplicate_protected_and_loader_environment_names() {
    let (base, _) = fixture();
    for name in [
        "PATH",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "DYLD_INSERT_LIBRARIES",
        "QT_PLUGIN_PATH",
        "QT_QML_IMPORT_PATH",
        "QML2_IMPORT_PATH",
        "PYTHONSTARTUP",
        "NODE_PATH",
        "JAVA_TOOL_OPTIONS",
        "JDK_JAVA_OPTIONS",
        "DOTNET_STARTUP_HOOKS",
        "COR_ENABLE_PROFILING",
        "COMPLUS_PROFILER",
        "__COMPAT_LAYER",
    ] {
        let mut d = base.clone();
        d.entrypoint.environment_paths.push(ProgramEnvironmentPath {
            name: name.into(),
            path: "userdata".into(),
        });
        assert!(validate_descriptor_metadata(&d).is_err(), "accepted {name}");
    }
    let mut d = base;
    d.entrypoint.environment_paths.push(ProgramEnvironmentPath {
        name: "EXAMPLE_SAVEDIR".into(),
        path: "userdata".into(),
    });
    validate_descriptor_metadata(&d).unwrap();
    d.entrypoint
        .environment_paths
        .push(d.entrypoint.environment_paths[0].clone());
    assert!(
        validate_descriptor_metadata(&d).is_err(),
        "duplicate variable"
    );
}

#[test]
fn selects_only_declared_allowed_build_controls_pairs() {
    let (mut d, inventory) = fixture();
    d.controls_profiles
        .push(ProgramControlsProfile::ApplicationDefaults {
            id: "other".into(),
            title: "Other".into(),
        });
    let compiled = compile_descriptor(&d, &inventory).unwrap();
    let (build, controls) = compiled.selected("standard", "native").unwrap();
    assert_eq!(build.id, "standard");
    assert_eq!(controls.id(), "native");
    assert!(compiled.selected("missing", "native").is_err());
    assert!(compiled.selected("standard", "missing").is_err());
    assert!(compiled.selected("standard", "other").is_err());
}

fn fixture() -> (ProgramDescriptor, Vec<ProjectionLayer>) {
    let descriptor = ProgramDescriptor::read_json(&json!({
        "document_type":"program_descriptor", "schema_version":"1.0",
        "identity":{"id":"example", "title":"Example", "version":"7", "os":"windows", "architecture":"x86_64"},
        "executor":{"id":"fusion.declarative", "version":"1.0"},
        "components":[{"id":"program", "layer_id":"runtime", "origin":{"kind":"managed", "artifact_ref":"artifact", "redistribution_allowed":false}, "source_root":"", "mount_path":"app"}],
        "required_files":[{"path":"app/bin/demo.exe", "role":"main_executable"}],
        "projection_rules":[], "directories":["userdata"],
        "entrypoint":{"executable":"app/bin/demo.exe", "working_directory":"app/bin", "arguments":[], "environment_paths":[]},
        "configs":[{"id":"main", "path":"userdata/main.ini", "format":"ini", "seed":{"kind":"empty_ini"}, "preserve_unmanaged":true,
            "keys":[{"id":"fullscreen", "address":{"kind":"ini", "section":"Video", "key":"fullscreen"}, "scalar_type":"boolean", "allowed_values":[false,true]}]}],
        "build_profiles":[{"id":"standard", "title":"Standard", "layer_ids":["runtime"], "controls_profile_ids":["native"], "settings":[{"config_id":"main", "key_id":"fullscreen", "value":false}]}],
        "controls_profiles":[{"kind":"application_defaults", "id":"native", "title":"Native"}],
        "persistence":[{"path":"userdata/main.ini", "kind":"file", "policy":"preserve_latest", "export":"never"}]
    }).to_string()).unwrap();
    let inventory = vec![ProjectionLayer {
        layer_id: "runtime".into(),
        mount_path: "app".into(),
        verified_inventory: vec![file("bin/demo.exe", b"verified exe")],
    }];
    (descriptor, inventory)
}

fn file(path: &str, bytes: &[u8]) -> CasFile {
    CasFile {
        path: path.into(),
        blake3: blake3::hash(bytes).to_hex().to_string(),
        size: bytes.len() as u64,
    }
}

#[test]
fn compiles_verified_inventory_with_stable_descriptor_digest() {
    let (descriptor, inventory) = fixture();
    let compiled = compile_descriptor(&descriptor, &inventory).unwrap();
    assert_eq!(compiled.descriptor, descriptor);
    assert_eq!(
        compiled.projections["standard"].files["app/bin/demo.exe"].winning_layer,
        "runtime"
    );
    assert_eq!(
        compiled.projections["standard"].files["app/bin/demo.exe"].blake3,
        inventory[0].verified_inventory[0].blake3
    );
    let canonical = serde_json::to_vec(&serde_json::to_value(&descriptor).unwrap()).unwrap();
    assert_eq!(
        compiled.descriptor_blake3,
        blake3::hash(&canonical).to_hex().to_string()
    );
}

#[test]
fn rejects_duplicate_ids_and_dangling_references_before_projection() {
    let (base, inventory) = fixture();
    let mut cases = Vec::new();
    let mut d = base.clone();
    d.components.push(d.components[0].clone());
    cases.push(("component", d));
    let mut d = base.clone();
    let mut c = d.components[0].clone();
    c.id = "other".into();
    d.components.push(c);
    cases.push(("layer", d));
    let mut d = base.clone();
    d.configs.push(d.configs[0].clone());
    cases.push(("config", d));
    let mut d = base.clone();
    let key = d.configs[0].keys[0].clone();
    d.configs[0].keys.push(key);
    cases.push(("key", d));
    let mut d = base.clone();
    d.build_profiles.push(d.build_profiles[0].clone());
    cases.push(("build", d));
    let mut d = base.clone();
    d.controls_profiles.push(d.controls_profiles[0].clone());
    cases.push(("controls", d));
    let mut d = base.clone();
    d.build_profiles[0].layer_ids = vec!["missing".into()];
    cases.push(("layer ref", d));
    let mut d = base.clone();
    d.build_profiles[0].controls_profile_ids = vec!["missing".into()];
    cases.push(("controls ref", d));
    let mut d = base.clone();
    d.build_profiles[0].settings[0].config_id = "missing".into();
    cases.push(("config ref", d));
    let mut d = base.clone();
    d.build_profiles[0].settings[0].key_id = "missing".into();
    cases.push(("key ref", d));
    let mut d = base.clone();
    d.projection_rules.push(ProgramProjectionRule {
        path: "app/bin/demo.exe".into(),
        winning_layer: "missing".into(),
    });
    cases.push(("winner ref", d));
    let mut d = base.clone();
    d.controls_profiles
        .push(ProgramControlsProfile::EdenParamPackageV1 {
            id: "writer".into(),
            title: "Writer".into(),
            device: crate::fusion_source::creator_schema::InputDeviceKind::KeyboardMouse,
            bindings: Default::default(),
            config_id: "missing".into(),
        });
    cases.push(("writer config ref", d));
    for (name, descriptor) in cases {
        assert!(
            compile_descriptor(&descriptor, &inventory).is_err(),
            "accepted {name}"
        );
    }
}

#[test]
fn enforces_codec_domains_and_exactly_once_assignments_in_every_profile() {
    let (base, _) = fixture();
    let mut cases = Vec::new();
    let mut d = base.clone();
    d.configs[0].seed = ProgramConfigSeed::XmlRoot {
        root: "root".into(),
        attributes: vec![],
    };
    cases.push(("seed", d));
    let mut d = base.clone();
    d.configs[0].keys[0].address = ProgramConfigAddress::XmlAttribute {
        elements: vec![],
        attribute: "value".into(),
    };
    cases.push(("address", d));
    let mut d = base.clone();
    let mut key = d.configs[0].keys[0].clone();
    key.id = "alias".into();
    d.configs[0].keys.push(key);
    cases.push(("duplicate address", d));
    let mut d = base.clone();
    d.configs[0].keys[0].scalar_type = ProgramScalarType::String;
    cases.push(("domain type", d));
    let mut d = base.clone();
    d.build_profiles[0].settings.clear();
    cases.push(("missing assignment", d));
    let mut d = base.clone();
    let s = d.build_profiles[0].settings[0].clone();
    d.build_profiles[0].settings.push(s);
    cases.push(("duplicate assignment", d));
    let mut d = base.clone();
    d.build_profiles[0].settings[0].value = ProgramScalar::String("false".into());
    cases.push(("assignment type", d));
    let mut d = base.clone();
    d.configs[0].keys[0].allowed_values = vec![ProgramScalar::Boolean(true)];
    cases.push(("assignment domain", d));
    let mut d = base.clone();
    let mut p = d.build_profiles[0].clone();
    p.id = "broken-unselected".into();
    p.settings.clear();
    d.build_profiles.push(p);
    cases.push(("unselected profile", d));
    let mut d = base.clone();
    d.configs[0].format = ProgramConfigFormat::XmlAttributes;
    d.configs[0].seed = ProgramConfigSeed::XmlRoot {
        root: "root".into(),
        attributes: vec![
            ProgramXmlAttribute {
                name: "version".into(),
                value: ProgramScalar::String("1".into()),
            },
            ProgramXmlAttribute {
                name: "version".into(),
                value: ProgramScalar::String("2".into()),
            },
        ],
    };
    d.configs[0].keys.clear();
    d.build_profiles[0].settings.clear();
    cases.push(("duplicate seed attribute", d));
    for (name, d) in cases {
        assert!(validate_descriptor_metadata(&d).is_err(), "accepted {name}");
    }
    let mut d = base;
    d.configs[0].keys[0].scalar_type = ProgramScalarType::Integer;
    d.configs[0].keys[0].allowed_values = vec![ProgramScalar::Number(1.into())];
    d.build_profiles[0].settings[0].value =
        ProgramScalar::Number(serde_json::Number::from_f64(1.0).unwrap());
    validate_descriptor_metadata(&d).unwrap();
    d.configs[0].keys[0].allowed_values = vec![ProgramScalar::Number(
        serde_json::Number::from_f64(1.5).unwrap(),
    )];
    assert!(
        validate_descriptor_metadata(&d).is_err(),
        "fractional integer domain"
    );
}
