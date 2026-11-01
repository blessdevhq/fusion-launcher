use super::*;
use crate::fusion_source::creator_schema::ComponentOrigin;
use tempfile::tempdir;

#[test]
fn zip_normalization_validates_whole_archive_and_selects_only_exact_root() {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let temp = tempdir().unwrap();
    let archive = temp.path().join("program.zip");
    let cas = CasStore::open(&temp.path().join("library")).unwrap();
    for unsafe_name in [
        None,
        Some("unselected/../escape"),
        Some("Selected/alias.bin"),
    ] {
        let mut zip = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
        zip.start_file("selected/program.exe", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"program bytes").unwrap();
        if let Some(name) = unsafe_name {
            zip.start_file(name, SimpleFileOptions::default()).unwrap();
            zip.write_all(b"unsafe").unwrap();
        }
        zip.finish().unwrap();
        let result = normalize_zip(
            &cas,
            &archive,
            &temp.path().join("staging"),
            &component("selected"),
        );
        if unsafe_name.is_none() {
            let object = result.unwrap();
            assert_eq!(object.files.len(), 1);
            assert_eq!(object.files[0].path, "program.exe");
        } else {
            assert!(result.is_err());
        }
    }
}

fn component(source_root: &str) -> ProgramComponent {
    ProgramComponent {
        id: "component".into(),
        layer_id: "layer".into(),
        origin: ComponentOrigin::Managed {
            artifact_ref: "artifact-not-a-digest".into(),
            redistribution_allowed: false,
        },
        source_root: source_root.into(),
        mount_path: "app".into(),
    }
}

#[test]
fn checks_expected_inventory_for_each_nonmanaged_origin() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("input");
    fs::create_dir(&input).unwrap();
    fs::write(input.join("file"), b"same bytes").unwrap();
    let cas = CasStore::open(&temp.path().join("library")).unwrap();
    let expected = cas.import_directory(&input).unwrap();
    let mut value = component("");
    for external in [false, true] {
        let origin = |inventory: String| {
            if external {
                ComponentOrigin::External {
                    url: "https://example.test/pin.zip".into(),
                    transport_blake3: "a".repeat(64),
                    inventory_blake3: inventory,
                }
            } else {
                ComponentOrigin::UserProvided {
                    requirement_id: "user".into(),
                    inventory_blake3: inventory,
                    instructions: "Select local files".into(),
                }
            }
        };
        value.origin = origin("0".repeat(64));
        assert!(normalize_directory(&cas, &input, &value).is_err());
        value.origin = origin(expected.digest.clone());
        assert_eq!(
            normalize_directory(&cas, &input, &value).unwrap().digest,
            expected.digest
        );
    }
}

#[test]
fn rejects_an_unselected_reparse_tree_before_import() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("input");
    let outside = temp.path().join("outside");
    fs::create_dir_all(input.join("selected")).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(input.join("selected/program.exe"), b"bytes").unwrap();
    let link = input.join("unselected");
    #[cfg(windows)]
    assert!(std::process::Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&link)
        .arg(&outside)
        .output()
        .unwrap()
        .status
        .success());
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).unwrap();
    let cas = CasStore::open(&temp.path().join("library")).unwrap();
    let result = normalize_directory(&cas, &input, &component("selected"));
    #[cfg(windows)]
    fs::remove_dir(&link).unwrap();
    #[cfg(unix)]
    fs::remove_file(&link).unwrap();
    assert!(
        result.is_err(),
        "an unsafe unselected sibling must not be ignored"
    );
    assert_eq!(fs::read_dir(cas.root().join("objects")).unwrap().count(), 0);
}

#[test]
fn normalizes_exact_root_using_existing_cas_identity_without_modifying_source() {
    let temp = tempdir().unwrap();
    let input = temp.path().join("input");
    fs::create_dir_all(input.join("distribution/bin")).unwrap();
    fs::write(input.join("distribution/bin/program.exe"), b"program bytes").unwrap();
    fs::write(input.join("other.txt"), b"not selected").unwrap();
    let cas = CasStore::open(&temp.path().join("library")).unwrap();
    let expected = cas.import_directory(&input.join("distribution")).unwrap();
    let actual = normalize_directory(&cas, &input, &component("distribution")).unwrap();
    assert_eq!(actual.digest, expected.digest);
    assert_eq!(actual.files, expected.files);
    assert_eq!(actual.files[0].path, "bin/program.exe");
    assert!(!fs::metadata(input.join("distribution/bin/program.exe"))
        .unwrap()
        .permissions()
        .readonly());
    assert_ne!(
        fs_safety::file_identity(&actual.root.join("bin/program.exe")).unwrap(),
        fs_safety::file_identity(&input.join("distribution/bin/program.exe")).unwrap()
    );
}
