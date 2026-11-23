use super::*;

#[test]
fn rejects_write_root_overlap_before_leasing_workspace_or_reading_signing_key() {
    for (location, case) in [
        ("workspaces", "ancestor of a not-yet-created workspace"),
        (
            ".fusion-source",
            "ancestor of a not-yet-created release CAS",
        ),
        (
            ".fusion-source/cas/owned-input",
            "descendant of release CAS",
        ),
        ("", "Packer scratch parent"),
    ] {
        let fixture = Fixture::new(managed(true), true);
        let root = fixture._dir.path().join("creator");
        let workspace = root.join("workspaces").join(
            blake3::hash(fixture.request.project_id.as_bytes())
                .to_hex()
                .as_str(),
        );
        let input = if location.is_empty() {
            std::env::temp_dir()
        } else {
            let input = root.join(location);
            fs::create_dir_all(&input).unwrap();
            fs::write(input.join("app.exe"), b"owned component").unwrap();
            input
        };
        project_storage::save_private_bindings(
            &fixture.runner.store().lock().unwrap(),
            1,
            Some(0),
            &CreatorPrivateBindings {
                project_id: fixture.request.project_id.clone(),
                component_paths: [("program".into(), input.to_str().unwrap().into())].into(),
                signing_key_ref: Some(fixture.key.to_str().unwrap().into()),
            },
        )
        .unwrap();
        // Also prove the check precedes input hashing, without walking the real temp tree.
        fs::remove_file(&fixture.key).unwrap();
        let error = fixture.start().expect_err(case);
        assert!(!workspace.exists(), "{case}: wrote to the workspace");
        assert!(
            !root.join(".fusion-source/cas/staging").exists(),
            "{case}: created CAS staging"
        );
        assert_eq!(error.code, ErrorCode::InvalidPath, "{case}: {error:?}");
        assert_eq!(
            error.message,
            "The managed component directory overlaps a Creator Build write location.",
            "{case}"
        );
        assert_eq!(error.details.len(), 1);
        assert_eq!(error.details["componentId"], "program", "{case}");
    }
}
