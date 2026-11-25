use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::setup_profiles;
use crate::storage::RepositoryStore;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectedSystemKind {
    SwitchKeys,
    SwitchFirmware,
    Ps2Bios,
    Ps1Bios,
    Ps3Firmware,
    GameFile,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemFileDetection {
    pub kind: DetectedSystemKind,
    pub platform: Option<String>,
    pub profile_id: Option<String>,
    pub requirement_id: Option<String>,
    pub label: String,
    pub confidence: u8,
    pub details: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartImportReport {
    pub success: bool,
    pub detected_kind: String,
    pub platform: Option<String>,
    pub profile_id: Option<String>,
    pub requirement_id: Option<String>,
    pub label: String,
    pub installed_path: Option<String>,
    pub status: String,
    pub message: String,
}

pub fn detect_system_file(source_path: &Path) -> Result<SystemFileDetection, String> {
    if !source_path.exists() {
        return Err(format!("File does not exist: {}", source_path.display()));
    }
    if !source_path.is_file() {
        return Err(format!("Path is not a file: {}", source_path.display()));
    }

    let metadata =
        fs::metadata(source_path).map_err(|e| format!("Failed to read file metadata: {e}"))?;
    let file_size = metadata.len();
    let file_name = source_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = source_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    // Read header (up to 64KB for inspection)
    let mut header = Vec::with_capacity(65536);
    let mut file =
        File::open(source_path).map_err(|e| format!("Failed to open file for inspection: {e}"))?;
    let mut reader = (&mut file).take(65536);
    reader.read_to_end(&mut header).ok();

    // 1. PS3 Firmware (SCEUF magic at offset 0)
    if header.len() >= 5 && &header[0..5] == b"SCEUF" {
        return Ok(SystemFileDetection {
            kind: DetectedSystemKind::Ps3Firmware,
            platform: Some("ps3".to_string()),
            profile_id: Some("ps3-manual".to_string()),
            requirement_id: Some("ps3-firmware".to_string()),
            label: "PlayStation 3 Firmware".to_string(),
            confidence: 100,
            details: Some("Official PS3UPDAT.PUP container detected (SCEUF header)".to_string()),
        });
    }
    if file_name == "ps3updat.pup" || extension == "pup" {
        return Ok(SystemFileDetection {
            kind: DetectedSystemKind::Ps3Firmware,
            platform: Some("ps3".to_string()),
            profile_id: Some("ps3-manual".to_string()),
            requirement_id: Some("ps3-firmware".to_string()),
            label: "PlayStation 3 Firmware".to_string(),
            confidence: 90,
            details: Some("PS3 update file name pattern detected".to_string()),
        });
    }

    // 2. Switch Keys (prod.keys, title.keys or text with key tokens)
    if file_name == "prod.keys" || file_name == "title.keys" || extension == "keys" {
        return Ok(SystemFileDetection {
            kind: DetectedSystemKind::SwitchKeys,
            platform: Some("switch".to_string()),
            profile_id: Some("switch-manual".to_string()),
            requirement_id: Some("switch-prod-keys".to_string()),
            label: "Switch prod.keys".to_string(),
            confidence: 98,
            details: Some(format!("Nintendo Switch keys file: {file_name}")),
        });
    }
    if file_size > 0 && file_size < 524288 {
        if let Ok(text) = std::str::from_utf8(&header) {
            let lower_text = text.to_ascii_lowercase();
            if lower_text.contains("master_key_")
                || lower_text.contains("header_key")
                || lower_text.contains("aes_kek_")
                || lower_text.contains("key_area_key_")
            {
                return Ok(SystemFileDetection {
                    kind: DetectedSystemKind::SwitchKeys,
                    platform: Some("switch".to_string()),
                    profile_id: Some("switch-manual".to_string()),
                    requirement_id: Some("switch-prod-keys".to_string()),
                    label: "Switch prod.keys".to_string(),
                    confidence: 95,
                    details: Some(
                        "Nintendo Switch master/header key definitions detected".to_string(),
                    ),
                });
            }
        }
    }

    // 3. Switch Firmware: Zip containing NCA files, or standalone NCA
    if extension == "zip" {
        if let Ok(file) = File::open(source_path) {
            if let Ok(mut archive) = zip::ZipArchive::new(file) {
                let mut nca_count = 0;
                for i in 0..archive.len().min(300) {
                    if let Ok(entry) = archive.by_index(i) {
                        let name = entry.name().to_ascii_lowercase();
                        if name.ends_with(".nca") || name.ends_with(".cnmt.nca") {
                            nca_count += 1;
                        }
                    }
                }
                if nca_count >= 5 {
                    return Ok(SystemFileDetection {
                        kind: DetectedSystemKind::SwitchFirmware,
                        platform: Some("switch".to_string()),
                        profile_id: Some("switch-manual".to_string()),
                        requirement_id: Some("switch-firmware".to_string()),
                        label: "Switch firmware".to_string(),
                        confidence: 99,
                        details: Some(format!(
                            "Nintendo Switch firmware package containing {nca_count}+ NCA files"
                        )),
                    });
                }
            }
        }
    }
    if extension == "nca" {
        return Ok(SystemFileDetection {
            kind: DetectedSystemKind::SwitchFirmware,
            platform: Some("switch".to_string()),
            profile_id: Some("switch-manual".to_string()),
            requirement_id: Some("switch-firmware".to_string()),
            label: "Switch firmware".to_string(),
            confidence: 90,
            details: Some("Nintendo Switch NCA system archive file".to_string()),
        });
    }

    // 4. PS2 BIOS: ~4MB with ROMDIR signature
    if (2000000..=9000000).contains(&file_size) {
        let has_romdir = header.windows(6).any(|w| w == b"ROMDIR");
        let has_sony = header.windows(4).any(|w| w == b"Sony" || w == b"SONY");
        let name_match =
            file_name.contains("scph") || file_name.contains("ps2") || file_name.contains("bios");

        if has_romdir || (has_sony && name_match) {
            return Ok(SystemFileDetection {
                kind: DetectedSystemKind::Ps2Bios,
                platform: Some("ps2".to_string()),
                profile_id: Some("ps2-pcsx2".to_string()),
                requirement_id: Some("ps2-bios".to_string()),
                label: "PlayStation 2 BIOS".to_string(),
                confidence: 96,
                details: Some("PlayStation 2 BIOS signature verified (ROMDIR / Sony)".to_string()),
            });
        }
    }

    // 5. PS1 BIOS: ~512KB with Sony / System ROM signature
    if (500000..=550000).contains(&file_size) {
        let has_sony = header.windows(4).any(|w| w == b"Sony" || w == b"SONY");
        let has_psx = header.windows(4).any(|w| w == b"PS-X" || w == b"PSX ");
        let name_match =
            file_name.contains("scph") || file_name.contains("ps1") || file_name.contains("psx");

        if has_sony || has_psx || name_match {
            return Ok(SystemFileDetection {
                kind: DetectedSystemKind::Ps1Bios,
                platform: Some("ps1".to_string()),
                profile_id: Some("ps1-manual".to_string()),
                requirement_id: Some("ps1-bios".to_string()),
                label: "PlayStation BIOS".to_string(),
                confidence: 94,
                details: Some("PlayStation 1 BIOS signature verified".to_string()),
            });
        }
    }

    // 6. Check if it's a recognizable game file instead of a system requirement
    let game_platform = match extension.as_str() {
        "nsp" | "xci" => Some("switch"),
        "nes" => Some("nes"),
        "sfc" | "smc" => Some("snes"),
        "z64" | "n64" | "v64" => Some("n64"),
        "gba" => Some("gba"),
        "cso" | "pbp" => Some("psp"),
        "chd" | "iso" => Some("ps2"),
        "rvz" | "wbfs" | "gcm" => Some("gamecube"),
        "pkg" => Some("ps3"),
        _ => None,
    };

    if let Some(plat) = game_platform {
        return Ok(SystemFileDetection {
            kind: DetectedSystemKind::GameFile,
            platform: Some(plat.to_string()),
            profile_id: None,
            requirement_id: None,
            label: format!("Game payload ({extension})"),
            confidence: 85,
            details: Some(format!(
                "Playable game image for {plat}. Import this through the game library."
            )),
        });
    }

    Ok(SystemFileDetection {
        kind: DetectedSystemKind::Unknown,
        platform: None,
        profile_id: None,
        requirement_id: None,
        label: "Unknown file".to_string(),
        confidence: 0,
        details: Some(
            "Could not recognize file as a system key, BIOS, firmware, or game.".to_string(),
        ),
    })
}

pub fn auto_import_system_file_internal(
    store: &RepositoryStore,
    content_dir: &Path,
    source_path: &Path,
) -> Result<SmartImportReport, String> {
    let detection = detect_system_file(source_path)?;

    if detection.kind == DetectedSystemKind::Unknown {
        return Ok(SmartImportReport {
            success: false,
            detected_kind: "unknown".to_string(),
            platform: None,
            profile_id: None,
            requirement_id: None,
            label: detection.label,
            installed_path: None,
            status: "unrecognized".to_string(),
            message: detection
                .details
                .unwrap_or_else(|| "Unrecognized system file.".to_string()),
        });
    }

    if detection.kind == DetectedSystemKind::GameFile {
        return Ok(SmartImportReport {
            success: false,
            detected_kind: "game_file".to_string(),
            platform: detection.platform,
            profile_id: None,
            requirement_id: None,
            label: detection.label,
            installed_path: None,
            status: "game_file".to_string(),
            message: detection
                .details
                .unwrap_or_else(|| "This is a game ROM, not a system file.".to_string()),
        });
    }

    let profile_id = detection
        .profile_id
        .as_deref()
        .ok_or_else(|| "Detected system file missing setup profile mapping.".to_string())?;
    let requirement_id = detection
        .requirement_id
        .as_deref()
        .ok_or_else(|| "Detected system file missing requirement mapping.".to_string())?;

    let profile = setup_profiles::get_platform_setup_profile(profile_id)
        .ok_or_else(|| format!("Unknown setup profile: {profile_id}"))?;
    let requirement = profile
        .system_files
        .iter()
        .find(|req| req.id == requirement_id)
        .ok_or_else(|| format!("Requirement {requirement_id} not found in profile {profile_id}"))?;

    let report = crate::commands::import_profile_system_file_into_store(
        store,
        content_dir,
        &profile,
        requirement,
        source_path,
    )?;

    // Extra step for Switch firmware: unpack NCA files into System/switch/registered/
    if detection.kind == DetectedSystemKind::SwitchFirmware {
        let extension = source_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default();
        if extension.eq_ignore_ascii_case("zip") {
            let registered_dir = content_dir.join("System").join("switch").join("registered");
            if let Ok(unpacked_count) = unpack_switch_firmware_zip(source_path, &registered_dir) {
                return Ok(SmartImportReport {
                    success: true,
                    detected_kind: "switch_firmware".to_string(),
                    platform: Some("switch".to_string()),
                    profile_id: Some(profile.id),
                    requirement_id: Some(requirement.id.clone()),
                    label: detection.label,
                    installed_path: Some(report.installed_path),
                    status: "ready".to_string(),
                    message: format!(
                        "Nintendo Switch firmware installed! {unpacked_count} system components registered."
                    ),
                });
            }
        }
    }

    let success = report.status == "installed" || report.status == "already_installed";
    let message = if success {
        format!("Successfully installed {}", detection.label)
    } else {
        format!(
            "Failed to install {}: {:?}",
            detection.label, report.error_code
        )
    };

    Ok(SmartImportReport {
        success,
        detected_kind: match detection.kind {
            DetectedSystemKind::SwitchKeys => "switch_keys",
            DetectedSystemKind::SwitchFirmware => "switch_firmware",
            DetectedSystemKind::Ps2Bios => "ps2_bios",
            DetectedSystemKind::Ps1Bios => "ps1_bios",
            DetectedSystemKind::Ps3Firmware => "ps3_firmware",
            _ => "unknown",
        }
        .to_string(),
        platform: detection.platform,
        profile_id: Some(profile.id),
        requirement_id: Some(requirement.id.clone()),
        label: detection.label,
        installed_path: Some(report.installed_path),
        status: report.status,
        message,
    })
}

pub fn unpack_switch_firmware_zip(zip_path: &Path, registered_dir: &Path) -> Result<usize, String> {
    fs::create_dir_all(registered_dir)
        .map_err(|e| format!("Failed to create registered firmware dir: {e}"))?;
    let file = File::open(zip_path).map_err(|e| format!("Failed to open firmware zip: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Failed to read firmware zip: {e}"))?;

    let mut count = 0;
    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read zip entry: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let file_name = match Path::new(entry.name()).file_name() {
            Some(name) => name,
            None => continue,
        };
        let file_name_str = file_name.to_string_lossy();
        if file_name_str.ends_with(".nca") || file_name_str.ends_with(".cnmt.nca") {
            let out_path = registered_dir.join(&*file_name_str);
            let mut out_file =
                File::create(&out_path).map_err(|e| format!("Failed to write NCA file: {e}"))?;
            std::io::copy(&mut entry, &mut out_file)
                .map_err(|e| format!("Failed to extract NCA: {e}"))?;
            count += 1;
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    #[test]
    fn detects_ps3_pup_by_magic_bytes() {
        let temp = tempdir().unwrap();
        let pup_path = temp.path().join("update.bin");
        let mut file = File::create(&pup_path).unwrap();
        file.write_all(b"SCEUF\x00\x00\x00\x01data").unwrap();

        let detection = detect_system_file(&pup_path).unwrap();
        assert_eq!(detection.kind, DetectedSystemKind::Ps3Firmware);
        assert_eq!(detection.platform.as_deref(), Some("ps3"));
        assert_eq!(detection.requirement_id.as_deref(), Some("ps3-firmware"));
    }

    #[test]
    fn detects_switch_keys_by_tokens() {
        let temp = tempdir().unwrap();
        let keys_path = temp.path().join("custom.txt");
        let mut file = File::create(&keys_path).unwrap();
        file.write_all(b"master_key_00 = 0123456789abcdef0123456789abcdef\nheader_key = abc\n")
            .unwrap();

        let detection = detect_system_file(&keys_path).unwrap();
        assert_eq!(detection.kind, DetectedSystemKind::SwitchKeys);
        assert_eq!(detection.platform.as_deref(), Some("switch"));
        assert_eq!(
            detection.requirement_id.as_deref(),
            Some("switch-prod-keys")
        );
    }

    #[test]
    fn detects_switch_firmware_zip() {
        let temp = tempdir().unwrap();
        let zip_path = temp.path().join("firmware.zip");
        let file = File::create(&zip_path).unwrap();
        let mut writer = ZipWriter::new(file);
        for i in 0..10 {
            writer
                .start_file(format!("{i:02x}.nca"), SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"nca-bytes").unwrap();
        }
        writer.finish().unwrap();

        let detection = detect_system_file(&zip_path).unwrap();
        assert_eq!(detection.kind, DetectedSystemKind::SwitchFirmware);
        assert_eq!(detection.platform.as_deref(), Some("switch"));
        assert_eq!(detection.requirement_id.as_deref(), Some("switch-firmware"));
    }

    #[test]
    fn detects_ps2_bios_by_romdir() {
        let temp = tempdir().unwrap();
        let bios_path = temp.path().join("dump.bin");
        let mut file = File::create(&bios_path).unwrap();
        let mut buffer = vec![0_u8; 4_194_304]; // 4MB
        buffer[0x2000..0x2006].copy_from_slice(b"ROMDIR");
        file.write_all(&buffer).unwrap();

        let detection = detect_system_file(&bios_path).unwrap();
        assert_eq!(detection.kind, DetectedSystemKind::Ps2Bios);
        assert_eq!(detection.platform.as_deref(), Some("ps2"));
        assert_eq!(detection.requirement_id.as_deref(), Some("ps2-bios"));
    }

    #[test]
    fn identifies_game_files_as_game_payload() {
        let temp = tempdir().unwrap();
        let game_path = temp.path().join("zelda.nsp");
        File::create(&game_path).unwrap();

        let detection = detect_system_file(&game_path).unwrap();
        assert_eq!(detection.kind, DetectedSystemKind::GameFile);
        assert_eq!(detection.platform.as_deref(), Some("switch"));
    }
}
