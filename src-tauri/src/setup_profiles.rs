use crate::schema::{
    PlatformSetupEmulator, PlatformSetupGameFiles, PlatformSetupLaunch, PlatformSetupProfile,
    ProfileEmulatorDownload, ProfileSystemFileRequirement,
};

#[cfg(test)]
pub const PRODUCTION_PROFILE_IDS: &[&str] = &[
    "nes-mesen",
    "snes-mesen",
    "n64-rmg",
    "gba-mgba",
    "ps2-pcsx2",
    "psp-ppsspp",
    "ps1-manual",
    "switch-manual",
];

pub fn list_platform_setup_profiles() -> Vec<PlatformSetupProfile> {
    vec![
        profile(
            "nes-mesen",
            "nes",
            "NES / Mesen2",
            // Like every auto profile, the actual pinned/latest binary is
            // resolved from profiles/emulators.json via the orchestrator. The
            // setup profile only declares that NES is auto-installable.
            emulator_downloadable("Mesen2", &["Mesen.exe"], None),
            game_files(&[".nes"], false, &[], &["ines"]),
            vec![],
            launch("{game_path}", None),
        ),
        profile(
            "snes-mesen",
            "snes",
            "Super Nintendo / Mesen2",
            emulator_downloadable("Mesen2", &["Mesen.exe"], None),
            game_files(&[".sfc", ".smc"], false, &[], &[]),
            vec![],
            launch("{game_path}", None),
        ),
        profile(
            "n64-rmg",
            "n64",
            "Nintendo 64 / RMG",
            emulator_downloadable("RMG", &["RMG.exe"], None),
            game_files(&[".z64", ".n64", ".v64"], false, &[], &[]),
            vec![],
            launch("{game_path}", None),
        ),
        profile(
            "gba-mgba",
            "gba",
            "Game Boy Advance / mGBA",
            emulator_downloadable("mGBA", &["mGBA.exe"], None),
            game_files(&[".gba"], false, &[], &[]),
            vec![],
            launch("-f {game_path}", None),
        ),
        profile(
            "ps2-pcsx2",
            "ps2",
            "PlayStation 2 / PCSX2",
            emulator_downloadable("PCSX2", &["pcsx2-qt.exe"], None),
            game_files(
                &[".iso", ".bin", ".img", ".chd"],
                true,
                &["*.iso", "*.chd"],
                &[],
            ),
            vec![system_file(
                "ps2-bios",
                "PlayStation 2 BIOS",
                "bios",
                true,
                &[".bin", ".rom"],
                Some("bios/ps2-bios.bin"),
                Some("Import a BIOS image dumped from hardware you own."),
            )],
            launch("-fullscreen -- {game_path}", None),
        ),
        profile(
            "psp-ppsspp",
            "psp",
            "PSP / PPSSPP",
            emulator_downloadable(
                "PPSSPP",
                &["PPSSPPWindows64.exe", "PPSSPPWindows.exe"],
                None,
            ),
            game_files(&[".iso", ".cso", ".pbp"], false, &[], &[]),
            vec![],
            launch("--fullscreen {game_path}", None),
        ),
        profile(
            "ps1-manual",
            "ps1",
            "PlayStation 1 / DuckStation",
            emulator_manual(
                "DuckStation",
                &["duckstation-qt-x64-ReleaseLTCG.exe", "duckstation.exe"],
            ),
            game_files(
                &[".cue", ".bin", ".iso", ".img", ".pbp", ".chd"],
                true,
                &["*.cue", "*.chd"],
                &[],
            ),
            vec![system_file(
                "ps1-bios",
                "PlayStation BIOS",
                "bios",
                true,
                &[".bin", ".rom"],
                Some("bios/scph5501.bin"),
                Some("Import a BIOS image dumped from hardware you own."),
            )],
            launch("-batch \"{game_path}\"", None),
        ),
        profile(
            "switch-manual",
            "switch",
            "Nintendo Switch / Manual Emulator",
            emulator_manual(
                "Eden-compatible Switch emulator",
                &["eden.exe", "eden-cli.exe", "Ryujinx.exe", "suyu.exe"],
            ),
            game_files(&[".nsp", ".xci", ".nca"], true, &["*.nsp", "*.xci"], &[]),
            vec![
                system_file(
                    "switch-prod-keys",
                    "Switch prod.keys",
                    "keys",
                    true,
                    &[".keys"],
                    Some("prod.keys"),
                    Some("Import keys from your own legally owned console environment."),
                ),
                system_file(
                    "switch-firmware",
                    "Switch firmware",
                    "firmware",
                    false,
                    &[".zip"],
                    Some("firmware.zip"),
                    Some("Optional firmware package provided by the user."),
                ),
            ],
            launch("{game_path}", None),
        ),
        profile(
            "snes-manual",
            "snes",
            "Super Nintendo / Manual Emulator",
            emulator_manual("SNES emulator", &["bsnes.exe", "snes9x.exe"]),
            game_files(&[".sfc", ".smc"], false, &[], &[]),
            vec![],
            launch("{game_path}", None),
        ),
        profile(
            "ps2-manual",
            "ps2",
            "PlayStation 2 / PCSX2",
            emulator_manual("PCSX2", &["pcsx2-qt.exe"]),
            game_files(
                &[".iso", ".bin", ".img", ".chd"],
                true,
                &["*.iso", "*.chd"],
                &[],
            ),
            vec![system_file(
                "ps2-bios",
                "PlayStation 2 BIOS",
                "bios",
                true,
                &[".bin", ".rom"],
                Some("bios/ps2-bios.bin"),
                Some("Import a BIOS image dumped from hardware you own."),
            )],
            launch("-fullscreen -- \"{game_path}\"", None),
        ),
        profile(
            "psp-manual",
            "psp",
            "PSP / Manual Emulator",
            emulator_manual("PPSSPP", &["PPSSPPWindows64.exe", "PPSSPPWindows.exe"]),
            game_files(&[".iso", ".cso", ".pbp"], false, &[], &[]),
            vec![],
            launch("{game_path}", None),
        ),
    ]
}

pub fn get_platform_setup_profile(profile_id: &str) -> Option<PlatformSetupProfile> {
    list_platform_setup_profiles()
        .into_iter()
        .find(|profile| profile.id == profile_id)
}

pub fn get_default_platform_setup_profile(platform: &str) -> Option<PlatformSetupProfile> {
    let profile_id = match platform {
        "nes" => "nes-mesen",
        "snes" => "snes-mesen",
        "n64" => "n64-rmg",
        "gba" => "gba-mgba",
        "ps2" => "ps2-pcsx2",
        "psp" => "psp-ppsspp",
        "ps1" => "ps1-manual",
        "switch" => "switch-manual",
        _ => return None,
    };
    get_platform_setup_profile(profile_id)
}

pub fn default_launch_args_for(platform: &str) -> Option<String> {
    get_default_platform_setup_profile(platform).map(|profile| profile.launch.args_template)
}

pub fn platform_display_label(platform: &str) -> Option<String> {
    get_default_platform_setup_profile(platform).map(|profile| {
        profile
            .display_name
            .split_once(" / ")
            .map(|(label, _)| label)
            .unwrap_or(profile.display_name.as_str())
            .to_string()
    })
}

pub fn platform_emulator_name(platform: &str) -> Option<String> {
    get_default_platform_setup_profile(platform).map(|profile| profile.emulator.emulator_name)
}

pub fn mvp_platforms() -> impl Iterator<Item = PlatformSetupProfile> {
    ["nes", "snes", "n64", "gba", "ps2", "psp", "ps1", "switch"]
        .into_iter()
        .filter_map(get_default_platform_setup_profile)
}

pub fn has_default_setup_profile(platform: &str) -> bool {
    get_default_platform_setup_profile(platform).is_some()
}

#[cfg(test)]
pub fn is_known_profile(profile_id: &str) -> bool {
    get_platform_setup_profile(profile_id).is_some()
}

fn profile(
    id: &str,
    platform: &str,
    display_name: &str,
    emulator: PlatformSetupEmulator,
    game_files: PlatformSetupGameFiles,
    system_files: Vec<ProfileSystemFileRequirement>,
    launch: PlatformSetupLaunch,
) -> PlatformSetupProfile {
    PlatformSetupProfile {
        id: id.to_string(),
        platform: platform.to_string(),
        display_name: display_name.to_string(),
        emulator,
        game_files,
        system_files,
        launch,
    }
}

fn emulator_manual(emulator_name: &str, executable_candidates: &[&str]) -> PlatformSetupEmulator {
    PlatformSetupEmulator {
        install_mode: "manual".to_string(),
        emulator_name: emulator_name.to_string(),
        executable_name: executable_candidates
            .first()
            .map(|name| (*name).to_string()),
        executable_candidates: executable_candidates
            .iter()
            .map(|candidate| (*candidate).to_string())
            .collect(),
        download: None,
    }
}

fn emulator_downloadable(
    emulator_name: &str,
    executable_candidates: &[&str],
    download: Option<ProfileEmulatorDownload>,
) -> PlatformSetupEmulator {
    PlatformSetupEmulator {
        install_mode: "downloadable".to_string(),
        emulator_name: emulator_name.to_string(),
        executable_name: executable_candidates
            .first()
            .map(|name| (*name).to_string()),
        executable_candidates: executable_candidates
            .iter()
            .map(|candidate| (*candidate).to_string())
            .collect(),
        download,
    }
}

fn game_files(
    expected_extensions: &[&str],
    allow_directory: bool,
    preferred_file_patterns: &[&str],
    validators: &[&str],
) -> PlatformSetupGameFiles {
    PlatformSetupGameFiles {
        expected_extensions: expected_extensions
            .iter()
            .map(|extension| (*extension).to_string())
            .collect(),
        allow_directory,
        preferred_file_patterns: preferred_file_patterns
            .iter()
            .map(|pattern| (*pattern).to_string())
            .collect(),
        validators: validators
            .iter()
            .map(|validator| (*validator).to_string())
            .collect(),
    }
}

fn system_file(
    id: &str,
    label: &str,
    asset_kind: &str,
    required: bool,
    extensions: &[&str],
    target_name: Option<&str>,
    notes: Option<&str>,
) -> ProfileSystemFileRequirement {
    ProfileSystemFileRequirement {
        id: id.to_string(),
        label: label.to_string(),
        asset_kind: asset_kind.to_string(),
        required,
        extensions: extensions
            .iter()
            .map(|extension| (*extension).to_string())
            .collect(),
        target_name: target_name.map(ToString::to_string),
        checksum: None,
        source_mode: "user_provided".to_string(),
        notes: notes.map(ToString::to_string),
    }
}

fn launch(args_template: &str, preferred_file: Option<&str>) -> PlatformSetupLaunch {
    PlatformSetupLaunch {
        args_template: args_template.to_string(),
        working_directory: Some("emulator_dir".to_string()),
        preferred_file: preferred_file.map(ToString::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_exposes_required_mvp_profiles() {
        for profile_id in PRODUCTION_PROFILE_IDS {
            assert!(is_known_profile(profile_id));
        }
        assert!(is_known_profile("ps2-manual"));
    }

    #[test]
    fn profiles_do_not_publish_system_payload_urls() {
        for profile in list_platform_setup_profiles() {
            for requirement in profile.system_files {
                assert_eq!(requirement.source_mode, "user_provided");
                assert!(
                    requirement
                        .notes
                        .unwrap_or_default()
                        .to_lowercase()
                        .contains("import")
                        || !requirement.required
                );
            }
        }
    }

    #[test]
    fn setup_profiles_do_not_pin_emulator_downloads() {
        // Auto-install binaries are resolved exclusively from
        // profiles/emulators.json (GithubLatest) by the orchestrator. Setup
        // profiles must not duplicate pinned URLs/hashes, which would silently
        // drift from the real download.
        for profile in list_platform_setup_profiles() {
            assert!(
                profile.emulator.download.is_none(),
                "{} should not carry a pinned emulator download",
                profile.id
            );
        }
    }

    #[test]
    fn automatic_defaults_cover_six_profiles_and_keep_switch_manual() {
        for platform in ["nes", "snes", "n64", "gba", "ps2", "psp"] {
            let profile = get_default_platform_setup_profile(platform).unwrap();
            assert_eq!(profile.emulator.install_mode, "downloadable");
        }
        let switch = get_default_platform_setup_profile("switch").unwrap();
        assert_eq!(switch.emulator.install_mode, "manual");
        assert_eq!(switch.emulator.executable_name.as_deref(), Some("eden.exe"));
        assert_eq!(
            switch.emulator.executable_candidates,
            vec![
                "eden.exe".to_string(),
                "eden-cli.exe".to_string(),
                "Ryujinx.exe".to_string(),
                "suyu.exe".to_string()
            ]
        );
    }

    #[test]
    fn platform_helpers_are_derived_from_default_setup_profiles() {
        assert!(has_default_setup_profile("switch"));
        assert!(!has_default_setup_profile("dreamcast"));
        assert_eq!(
            default_launch_args_for("gba"),
            Some("-f {game_path}".to_string())
        );
        assert_eq!(
            platform_display_label("ps2"),
            Some("PlayStation 2".to_string())
        );
        assert_eq!(platform_emulator_name("nes"), Some("Mesen2".to_string()));
        assert_eq!(mvp_platforms().count(), 8);
    }
}
