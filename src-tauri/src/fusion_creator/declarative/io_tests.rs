use super::*;
use crate::fusion_source::creator_schema::ComponentOrigin;
use tempfile::tempdir;

// Mutate only headers of real, tiny ZIPs; hostile declared lengths must be
// rejected before the extractor reads data or creates staging output.
fn zip_fixture(path: &Path, entries: &[(&str, &[u8])], deflate: bool) {
    use std::io::Write;
    let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
    let options = zip::write::SimpleFileOptions::default().compression_method(if deflate {
        zip::CompressionMethod::Deflated
    } else {
        zip::CompressionMethod::Stored
    });
    for (name, bytes) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(bytes).unwrap();
    }
    zip.finish().unwrap();
}

fn patch_central(path: &Path, mut patch: impl FnMut(usize, &mut [u8])) {
    let mut bytes = fs::read(path).unwrap();
    let mut cursor = 0;
    let mut index = 0;
    while cursor + 46 <= bytes.len() {
        if bytes[cursor..cursor + 4] != *b"PK\x01\x02" {
            cursor += 1;
            continue;
        }
        let name = u16::from_le_bytes(bytes[cursor + 28..cursor + 30].try_into().unwrap()) as usize;
        let extra =
            u16::from_le_bytes(bytes[cursor + 30..cursor + 32].try_into().unwrap()) as usize;
        let comment =
            u16::from_le_bytes(bytes[cursor + 32..cursor + 34].try_into().unwrap()) as usize;
        let end = cursor + 46 + name + extra + comment;
        patch(index, &mut bytes[cursor..end]);
        cursor = end;
        index += 1;
    }
    assert!(index > 0);
    fs::write(path, bytes).unwrap();
}

fn assert_zip_preflight_rejects(archive: &Path, root: &Path) {
    let cas = CasStore::open(&root.join("library")).unwrap();
    let staging = root.join("staging");
    assert!(normalize_zip(&cas, archive, &staging, &component("")).is_err());
    assert!(
        !staging.exists(),
        "preflight must reject before creating output"
    );
    assert_eq!(fs::read_dir(cas.root().join("objects")).unwrap().count(), 0);
}

#[test]
fn zip_rejects_slip_absolute_unc_drive_and_backslash_names_before_output() {
    for name in [
        "../escape",
        "safe/../../escape",
        "/absolute",
        "//server/share/file",
        "\\\\server\\share\\file",
        "C:/drive/file",
        "C:relative",
        "safe\\..\\escape",
    ] {
        let temp = tempdir().unwrap();
        let archive = temp.path().join("input.zip");
        zip_fixture(&archive, &[("safe", b"ok"), (name, b"bad")], false);
        assert_zip_preflight_rejects(&archive, temp.path());
    }
}

#[test]
fn zip_rejects_duplicate_case_implicit_parent_and_file_parent_conflicts() {
    for names in [
        ["same-a", "same-b"],
        ["File", "file"],
        ["Parent/a", "parent/b"],
        ["Parent/a", "parent/"],
        ["file", "file/child"],
        ["file/child", "file"],
    ] {
        let temp = tempdir().unwrap();
        let archive = temp.path().join("input.zip");
        zip_fixture(&archive, &[(names[0], b"a"), (names[1], b"b")], false);
        if names[0] == "same-a" {
            // ZipWriter itself forbids duplicate names, so forge the second header.
            patch_central(&archive, |index, header| {
                if index == 1 {
                    header[46..52].copy_from_slice(b"same-a");
                }
            });
        }
        assert_zip_preflight_rejects(&archive, temp.path());
    }
}

#[test]
fn zip_rejects_symlinks_and_every_unix_special_file_kind() {
    for kind in [0o120000u32, 0o010000, 0o020000, 0o060000, 0o140000] {
        let temp = tempdir().unwrap();
        let archive = temp.path().join("input.zip");
        zip_fixture(&archive, &[("safe", b"ok"), ("special", b"target")], false);
        patch_central(&archive, |index, header| {
            if index == 1 {
                header[5] = 3; // Unix creator system.
                header[38..42].copy_from_slice(&((kind | 0o644) << 16).to_le_bytes());
            }
        });
        assert_zip_preflight_rejects(&archive, temp.path());
    }
}

#[test]
fn zip_rejects_extreme_declared_total_before_any_output() {
    let temp = tempdir().unwrap();
    let archive = temp.path().join("input.zip");
    let names: Vec<_> = (0..17).map(|i| format!("file-{i}")).collect();
    let entries: Vec<_> = names
        .iter()
        .map(|n| (n.as_str(), b"x".as_slice()))
        .collect();
    zip_fixture(&archive, &entries, false);
    patch_central(&archive, |_, header| {
        header[24..28].copy_from_slice(&(u32::MAX - 1).to_le_bytes());
    });
    assert_zip_preflight_rejects(&archive, temp.path());
}

#[test]
fn zip_rejects_high_compression_ratio_before_any_output() {
    let temp = tempdir().unwrap();
    let archive = temp.path().join("input.zip");
    zip_fixture(&archive, &[("bomb", &vec![0; 2 * 1024 * 1024])], true);
    // Encoder backends differ near DEFLATE's maximum ratio. Forge a declared
    // ratio strictly above the ceiling instead of assuming an encoder result.
    let compressed = zip::ZipArchive::new(fs::File::open(&archive).unwrap())
        .unwrap()
        .by_index(0)
        .unwrap()
        .compressed_size();
    let declared = u32::try_from(
        compressed
            .checked_mul(ZipNormalizationLimits::default().max_compression_ratio + 1)
            .unwrap(),
    )
    .unwrap();
    patch_central(&archive, |_, header| {
        header[24..28].copy_from_slice(&declared.to_le_bytes())
    });
    assert_zip_preflight_rejects(&archive, temp.path());
}

#[test]
fn zip_host_ratio_limit_rejects_real_deflated_data_before_output() {
    let temp = tempdir().unwrap();
    let archive = temp.path().join("input.zip");
    zip_fixture(&archive, &[("bomb", &vec![0; 64 * 1024])], true);
    let cas = CasStore::open(&temp.path().join("library")).unwrap();
    let staging = temp.path().join("staging");
    let error = normalize_zip_with_limits(
        &cas,
        &archive,
        &staging,
        &component(""),
        ZipNormalizationLimits {
            max_compression_ratio: 16,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.message.contains("compression ratio"));
    assert!(!staging.exists());
}

#[test]
fn zip64_extreme_declared_lengths_cannot_wrap_preflight_arithmetic() {
    for (uncompressed, compressed) in [(u64::MAX, 1u64), (1u64, u64::MAX)] {
        let temp = tempdir().unwrap();
        let archive = temp.path().join("input.zip");
        zip_fixture(&archive, &[("a", b"x")], false);
        let mut bytes = fs::read(&archive).unwrap();
        let central = bytes.windows(4).position(|v| v == b"PK\x01\x02").unwrap();
        let eocd = bytes.windows(4).rposition(|v| v == b"PK\x05\x06").unwrap();
        let name_len =
            u16::from_le_bytes(bytes[central + 28..central + 30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(bytes[central + 30..central + 32].try_into().unwrap());
        let directory_size = u32::from_le_bytes(bytes[eocd + 12..eocd + 16].try_into().unwrap());
        bytes[central + 20..central + 28].fill(0xff);
        bytes[central + 30..central + 32].copy_from_slice(&(extra_len + 20).to_le_bytes());
        let mut extra = vec![1, 0, 16, 0];
        extra.extend_from_slice(&uncompressed.to_le_bytes());
        extra.extend_from_slice(&compressed.to_le_bytes());
        let insert = central + 46 + name_len + usize::from(extra_len);
        bytes.splice(insert..insert, extra);
        bytes[eocd + 32..eocd + 36].copy_from_slice(&(directory_size + 20).to_le_bytes());
        fs::write(&archive, bytes).unwrap();
        {
            let mut parsed = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
            let entry = parsed.by_index(0).unwrap();
            assert_eq!(entry.size(), uncompressed);
            assert_eq!(entry.compressed_size(), compressed);
        }
        assert_zip_preflight_rejects(&archive, temp.path());
    }
}

#[test]
fn zip_host_limits_bound_entries_and_both_aggregate_byte_counts() {
    let defaults = ZipNormalizationLimits::default();
    for limits in [
        ZipNormalizationLimits {
            max_entries: 1,
            ..defaults
        },
        ZipNormalizationLimits {
            max_uncompressed_bytes: 5,
            ..defaults
        },
        ZipNormalizationLimits {
            max_compressed_bytes: 5,
            ..defaults
        },
    ] {
        let temp = tempdir().unwrap();
        let archive = temp.path().join("input.zip");
        zip_fixture(&archive, &[("a", b"abc"), ("b", b"def")], false);
        let cas = CasStore::open(&temp.path().join("library")).unwrap();
        let staging = temp.path().join("staging");
        assert!(
            normalize_zip_with_limits(&cas, &archive, &staging, &component(""), limits).is_err()
        );
        assert!(!staging.exists());
        assert_eq!(fs::read_dir(cas.root().join("objects")).unwrap().count(), 0);
    }
    let temp = tempdir().unwrap();
    let archive = temp.path().join("input.zip");
    zip_fixture(&archive, &[("a", b"abc"), ("b", b"def")], false);
    let cas = CasStore::open(&temp.path().join("library")).unwrap();
    let object = normalize_zip_with_limits(
        &cas,
        &archive,
        &temp.path().join("staging"),
        &component(""),
        ZipNormalizationLimits {
            max_entries: 2,
            max_uncompressed_bytes: 6,
            max_compressed_bytes: 6,
            max_compression_ratio: 1,
        },
    )
    .unwrap();
    assert_eq!(object.files.len(), 2);
}

#[test]
fn zip_host_limits_cannot_be_disabled_or_made_extreme() {
    let defaults = ZipNormalizationLimits::default();
    for limits in [
        ZipNormalizationLimits {
            max_entries: 0,
            ..defaults
        },
        ZipNormalizationLimits {
            max_entries: usize::MAX,
            ..defaults
        },
        ZipNormalizationLimits {
            max_uncompressed_bytes: 0,
            ..defaults
        },
        ZipNormalizationLimits {
            max_uncompressed_bytes: u64::MAX,
            ..defaults
        },
        ZipNormalizationLimits {
            max_compressed_bytes: 0,
            ..defaults
        },
        ZipNormalizationLimits {
            max_compressed_bytes: u64::MAX,
            ..defaults
        },
        ZipNormalizationLimits {
            max_compression_ratio: 0,
            ..defaults
        },
        ZipNormalizationLimits {
            max_compression_ratio: u64::MAX,
            ..defaults
        },
    ] {
        let temp = tempdir().unwrap();
        let archive = temp.path().join("input.zip");
        zip_fixture(&archive, &[("a", b"abc")], false);
        let cas = CasStore::open(&temp.path().join("library")).unwrap();
        let staging = temp.path().join("staging");
        let error = normalize_zip_with_limits(&cas, &archive, &staging, &component(""), limits)
            .unwrap_err();
        assert!(error.message.contains("host safety ceilings"));
        assert!(!staging.exists());
    }
    // Metadata-only compatibility check, not a claim to have acquired STK.
    assert!(defaults.max_uncompressed_bytes >= 1_580_000_000);
    assert!(defaults.max_compressed_bytes >= 922_000_000);
    check_zip_ratio(1_580_000_000, 922_000_000, defaults.max_compression_ratio).unwrap();
    check_zip_ratio(u64::MAX, u64::MAX, u64::MAX).unwrap();
    assert!(check_zip_ratio(1, 0, defaults.max_compression_ratio).is_err());
    check_zip_ratio(0, 0, defaults.max_compression_ratio).unwrap();
}

#[test]
fn zip_ratio_is_per_member_not_hidden_by_incompressible_siblings() {
    let temp = tempdir().unwrap();
    let archive = temp.path().join("input.zip");
    zip_fixture(
        &archive,
        &[("padding", &[42; 100]), ("bomb", b"abc")],
        false,
    );
    patch_central(&archive, |index, header| {
        if index == 1 {
            header[24..28].copy_from_slice(&7u32.to_le_bytes());
        }
    });
    let cas = CasStore::open(&temp.path().join("library")).unwrap();
    let staging = temp.path().join("staging");
    let error = normalize_zip_with_limits(
        &cas,
        &archive,
        &staging,
        &component(""),
        ZipNormalizationLimits {
            max_compression_ratio: 2,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.message.contains("compression ratio"));
    assert!(!staging.exists());
}

#[test]
fn zip_nonempty_member_with_zero_compressed_bytes_is_rejected() {
    let temp = tempdir().unwrap();
    let archive = temp.path().join("input.zip");
    zip_fixture(&archive, &[("a", b"abc")], false);
    patch_central(&archive, |_, header| {
        header[20..24].copy_from_slice(&0u32.to_le_bytes())
    });
    assert_zip_preflight_rejects(&archive, temp.path());
}

#[test]
fn zip_existing_streaming_transformer_rejects_false_declared_lengths_and_cleans_staging() {
    let temp = tempdir().unwrap();
    let archive = temp.path().join("input.zip");
    zip_fixture(&archive, &[("a", b"abc")], false);
    patch_central(&archive, |_, header| {
        header[24..28].copy_from_slice(&2u32.to_le_bytes())
    });
    let cas = CasStore::open(&temp.path().join("library")).unwrap();
    let staging = temp.path().join("staging");
    assert!(normalize_zip_with_limits(
        &cas,
        &archive,
        &staging,
        &component(""),
        ZipNormalizationLimits {
            max_uncompressed_bytes: 2,
            ..Default::default()
        }
    )
    .is_err());
    assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
    assert_eq!(fs::read_dir(cas.root().join("objects")).unwrap().count(), 0);
}

#[test]
fn zip_external_transport_and_selected_inventory_are_independently_pinned() {
    let temp = tempdir().unwrap();
    let archive = temp.path().join("input.zip");
    zip_fixture(
        &archive,
        &[("selected/program.exe", b"program"), ("other", b"ignored")],
        false,
    );
    let cas = CasStore::open(&temp.path().join("library")).unwrap();
    let staging = temp.path().join("staging");
    let mut value = component("selected");
    let expected = normalize_zip(&cas, &archive, &staging, &value).unwrap();
    assert_eq!(expected.files.len(), 1);
    assert_eq!(expected.files[0].path, "program.exe");
    let transport = blake3::hash(&fs::read(&archive).unwrap())
        .to_hex()
        .to_string();
    value.origin = ComponentOrigin::External {
        url: "https://example.test/pin.zip".into(),
        transport_blake3: "0".repeat(64),
        inventory_blake3: expected.digest.clone(),
    };
    assert!(normalize_zip(&cas, &archive, &staging, &value).is_err());
    if let ComponentOrigin::External {
        transport_blake3,
        inventory_blake3,
        ..
    } = &mut value.origin
    {
        *transport_blake3 = transport.clone();
        *inventory_blake3 = "0".repeat(64);
    }
    assert_eq!(
        normalize_zip(&cas, &archive, &staging, &value)
            .unwrap_err()
            .code,
        ErrorCode::IntegrityViolation
    );
    if let ComponentOrigin::External {
        inventory_blake3, ..
    } = &mut value.origin
    {
        *inventory_blake3 = expected.digest.clone();
    }
    let actual = normalize_zip(&cas, &archive, &staging, &value).unwrap();
    assert_eq!(actual.digest, expected.digest);
    assert_eq!(actual.files, expected.files);
    assert_eq!(fs::read_dir(&staging).unwrap().count(), 0);
}

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

#[test]
fn zip_advertised_entry_count_is_rejected_before_library_indexing() {
    let temp = tempdir().unwrap();
    let archive = temp.path().join("input.zip");
    zip_fixture(&archive, &[("a", b"x")], false);
    let mut bytes = fs::read(&archive).unwrap();
    let end = bytes.windows(4).rposition(|v| v == b"PK\x05\x06").unwrap();
    bytes[end + 8..end + 12].copy_from_slice(&[254, 255, 254, 255]);
    fs::write(&archive, bytes).unwrap();
    let cas = CasStore::open(&temp.path().join("library")).unwrap();
    let staging = temp.path().join("staging");
    let error = normalize_zip_with_limits(
        &cas,
        &archive,
        &staging,
        &component(""),
        ZipNormalizationLimits {
            max_entries: 1,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        error.message.contains("entry limit before indexing"),
        "{error:?}"
    );
    assert!(!staging.exists());
}

#[test]
fn zip_central_metadata_budget_is_enforced_before_library_indexing() {
    let temp = tempdir().unwrap();
    let archive = temp.path().join("input.zip");
    zip_fixture(&archive, &[("a", b"x")], false);
    let mut bytes = fs::read(&archive).unwrap();
    let central = bytes.windows(4).position(|v| v == b"PK\x01\x02").unwrap();
    let end = bytes.windows(4).rposition(|v| v == b"PK\x05\x06").unwrap();
    let size = u32::from_le_bytes(bytes[end + 12..end + 16].try_into().unwrap());
    bytes[central + 32..central + 34].copy_from_slice(&8192u16.to_le_bytes());
    bytes[end + 12..end + 16].copy_from_slice(&(size + 8192).to_le_bytes());
    bytes.splice(end..end, vec![b'x'; 8192]);
    fs::write(&archive, bytes).unwrap();
    let cas = CasStore::open(&temp.path().join("library")).unwrap();
    let staging = temp.path().join("staging");
    let result = normalize_zip_with_limits(
        &cas,
        &archive,
        &staging,
        &component(""),
        ZipNormalizationLimits {
            max_entries: 1,
            ..Default::default()
        },
    );
    assert!(
        matches!(result, Err(ref error) if error.message.contains("metadata budget before indexing")),
        "{result:?}"
    );
    assert!(!staging.exists());
}

#[test]
fn zip64_end_record_bounds_precede_indexing_and_valid_zip64_still_normalizes() {
    for corruption in ["none", "count", "record_size", "directory_offset"] {
        let temp = tempdir().unwrap();
        let archive = temp.path().join("input.zip");
        zip_fixture(&archive, &[("a", b"x")], false);
        let mut bytes = fs::read(&archive).unwrap();
        let end = bytes.windows(4).rposition(|v| v == b"PK\x05\x06").unwrap();
        let mut classic = bytes.split_off(end);
        let size = u32::from_le_bytes(classic[12..16].try_into().unwrap());
        let offset = u32::from_le_bytes(classic[16..20].try_into().unwrap());
        let mut record = [0u8; 56];
        record[..4].copy_from_slice(b"PK\x06\x06");
        record[4..12].copy_from_slice(&44u64.to_le_bytes());
        record[12..14].copy_from_slice(&45u16.to_le_bytes());
        record[14..16].copy_from_slice(&45u16.to_le_bytes());
        record[24..32].copy_from_slice(&1u64.to_le_bytes());
        record[32..40].copy_from_slice(&1u64.to_le_bytes());
        record[40..48].copy_from_slice(&u64::from(size).to_le_bytes());
        record[48..56].copy_from_slice(&u64::from(offset).to_le_bytes());
        match corruption {
            "count" => {
                record[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
                record[32..40].copy_from_slice(&u64::MAX.to_le_bytes());
            }
            "record_size" => record[4..12].copy_from_slice(&u64::MAX.to_le_bytes()),
            "directory_offset" => record[48..56].copy_from_slice(&u64::MAX.to_le_bytes()),
            _ => {}
        }
        bytes.extend_from_slice(&record);
        bytes.extend_from_slice(b"PK\x06\x07");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(end as u64).to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        classic[8..20].fill(255);
        bytes.extend_from_slice(&classic);
        fs::write(&archive, bytes).unwrap();
        let cas = CasStore::open(&temp.path().join("library")).unwrap();
        let staging = temp.path().join("staging");
        let result = normalize_zip(&cas, &archive, &staging, &component(""));
        if corruption == "none" {
            let object = result.unwrap();
            assert_eq!(object.files.len(), 1);
            assert_eq!(fs::read(object.root.join("a")).unwrap(), b"x");
        } else {
            assert!(result.is_err(), "{corruption}");
            assert!(!staging.exists());
        }
    }
}

#[test]
fn zip_end_records_embedded_in_comments_cannot_bypass_index_budgets() {
    for advertised in [65534u16, u16::MAX] {
        let temp = tempdir().unwrap();
        let archive = temp.path().join("ambiguous.zip");
        zip_fixture(&archive, &[("padding", &vec![b'x'; 100_000])], false);
        let mut bytes = fs::read(&archive).unwrap();
        let end = bytes.windows(4).rposition(|v| v == b"PK\x05\x06").unwrap();
        let mut embedded = bytes[end..end + 22].to_vec();
        embedded[8..10].copy_from_slice(&advertised.to_le_bytes());
        embedded[10..12].copy_from_slice(&advertised.to_le_bytes());
        bytes[end + 20..end + 22].copy_from_slice(&23u16.to_le_bytes());
        bytes.extend_from_slice(&embedded);
        bytes.push(b'x'); // Library accepts a candidate whose comment ends before EOF.
        fs::write(&archive, bytes).unwrap();
        let cas = CasStore::open(&temp.path().join("library")).unwrap();
        let staging = temp.path().join("staging");
        let result = normalize_zip_with_limits(
            &cas,
            &archive,
            &staging,
            &component(""),
            ZipNormalizationLimits {
                max_entries: 1,
                ..Default::default()
            },
        );
        assert!(
            matches!(result, Err(ref error) if error.message.contains("Ambiguous ZIP metadata")),
            "{result:?}"
        );
        assert!(!staging.exists());
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
