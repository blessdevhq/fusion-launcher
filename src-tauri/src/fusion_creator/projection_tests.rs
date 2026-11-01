use super::*;

#[test]
fn legacy_projection_preserves_last_provider_and_windows_case_override() {
    let layers = [
        layer("z-base", "Data/App.exe", b"old"),
        layer("a-mod", "data/app.exe", b"new"),
    ];
    let result = resolve_projection(ProjectionRequest {
        semantics: ProjectionSemantics::Legacy10,
        selected_layers: &layers,
        rules: &[],
    })
    .unwrap();
    assert_eq!(result.files["data/app.exe"].winning_layer, "a-mod");
    assert_eq!(result.files["data/app.exe"].path, "data/app.exe");
}

#[test]
fn explicit_projection_equal_providers_are_order_independent() {
    let layers = [
        layer("z", "app.exe", b"same"),
        layer("a", "app.exe", b"same"),
    ];
    let resolve = |layers: &[ProjectionLayer]| {
        resolve_projection(ProjectionRequest {
            semantics: ProjectionSemantics::Explicit11,
            selected_layers: layers,
            rules: &[],
        })
        .unwrap()
    };
    let first = resolve(&layers);
    assert_eq!(first, resolve(&[layers[1].clone(), layers[0].clone()]));
    assert_eq!(first.files["app.exe"].equivalent_layers, ["a", "z"]);
}

#[test]
fn explicit_projection_rejects_duplicate_inventory_layers_and_rules() {
    let mut duplicate = layer("base", "app.exe", b"same");
    duplicate
        .verified_inventory
        .push(duplicate.verified_inventory[0].clone());
    assert!(resolve_projection(ProjectionRequest {
        semantics: ProjectionSemantics::Explicit11,
        selected_layers: &[duplicate],
        rules: &[]
    })
    .is_err());
    let layers = [
        layer("base", "app.exe", b"base"),
        layer("mod", "app.exe", b"mod"),
    ];
    assert!(resolve_projection(ProjectionRequest {
        semantics: ProjectionSemantics::Explicit11,
        selected_layers: &[layers[0].clone(), layers[0].clone()],
        rules: &[]
    })
    .is_err());
    let rules = [
        ProjectionRule {
            path: "app.exe".into(),
            winning_layer: "base".into(),
        },
        ProjectionRule {
            path: "APP.exe".into(),
            winning_layer: "mod".into(),
        },
    ];
    assert!(resolve_projection(ProjectionRequest {
        semantics: ProjectionSemantics::Explicit11,
        selected_layers: &layers,
        rules: &rules
    })
    .is_err());
}

#[test]
fn explicit_projection_rejects_nonexistent_or_nonproviding_winner() {
    let layers = [
        layer("base", "app.exe", b"base"),
        layer("mod", "app.exe", b"mod"),
        layer("other", "other.exe", b"other"),
    ];
    for winner in ["missing", "other"] {
        let rules = [ProjectionRule {
            path: "app.exe".into(),
            winning_layer: winner.into(),
        }];
        assert!(resolve_projection(ProjectionRequest {
            semantics: ProjectionSemantics::Explicit11,
            selected_layers: &layers,
            rules: &rules
        })
        .is_err());
    }
}

#[test]
fn explicit_projection_rejects_redundant_rules_and_inconsistent_sizes() {
    let mut layers = [
        layer("base", "app.exe", b"same"),
        layer("mod", "app.exe", b"same"),
    ];
    let rules = [ProjectionRule {
        path: "app.exe".into(),
        winning_layer: "base".into(),
    }];
    assert!(resolve_projection(ProjectionRequest {
        semantics: ProjectionSemantics::Explicit11,
        selected_layers: &layers,
        rules: &rules
    })
    .is_err());
    layers[1].verified_inventory[0].size += 1;
    assert!(resolve_projection(ProjectionRequest {
        semantics: ProjectionSemantics::Explicit11,
        selected_layers: &layers,
        rules: &[]
    })
    .is_err());
}

#[test]
fn explicit_projection_rejects_windows_parent_and_case_collisions() {
    for (left, right) in [
        ("data", "data/file.bin"),
        ("Data/one", "data/two"),
        ("App.exe", "app.exe"),
    ] {
        let layers = [layer("base", left, b"same"), layer("mod", right, b"same")];
        assert!(
            resolve_projection(ProjectionRequest {
                semantics: ProjectionSemantics::Explicit11,
                selected_layers: &layers,
                rules: &[]
            })
            .is_err(),
            "{left} vs {right}"
        );
    }
}

fn layer(id: &str, path: &str, bytes: &[u8]) -> ProjectionLayer {
    ProjectionLayer {
        layer_id: id.into(),
        mount_path: String::new(),
        verified_inventory: vec![CasFile {
            path: path.into(),
            blake3: blake3::hash(bytes).to_hex().to_string(),
            size: bytes.len() as u64,
        }],
    }
}

#[test]
fn explicit_conflicting_bytes_require_an_actual_winner() {
    let layers = [
        layer("base", "app.exe", b"base"),
        layer("mod", "app.exe", b"mod"),
    ];
    assert!(resolve_projection(ProjectionRequest {
        semantics: ProjectionSemantics::Explicit11,
        selected_layers: &layers,
        rules: &[],
    })
    .is_err());
    let rules = [ProjectionRule {
        path: "app.exe".into(),
        winning_layer: "base".into(),
    }];
    let resolved = resolve_projection(ProjectionRequest {
        semantics: ProjectionSemantics::Explicit11,
        selected_layers: &layers,
        rules: &rules,
    })
    .unwrap();
    assert_eq!(resolved.files["app.exe"].winning_layer, "base");
    assert_eq!(
        resolved.files["app.exe"].blake3,
        layers[0].verified_inventory[0].blake3
    );
}
