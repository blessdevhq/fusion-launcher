use serde::Serialize;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformConfig {
    pub id: &'static str,
    pub label: &'static str,
    pub emulator_name: &'static str,
    pub executable_hint: &'static str,
    pub launch_args_template: &'static str,
    pub system_path_hint: &'static str,
    pub expected_extensions: &'static [&'static str],
}

pub const MVP_PLATFORM_IDS: &[&str] = &["switch", "ps1", "ps2", "gba", "nes"];

pub const MVP_PLATFORM_CONFIGS: &[PlatformConfig] = &[
    PlatformConfig {
        id: "switch",
        label: "Nintendo Switch",
        emulator_name: "Ryujinx",
        executable_hint: "Ryujinx.exe",
        launch_args_template: "{game_path}",
        system_path_hint: "portable/system",
        expected_extensions: &[".nsp", ".xci"],
    },
    PlatformConfig {
        id: "ps1",
        label: "PlayStation 1",
        emulator_name: "DuckStation",
        executable_hint: "duckstation-qt-x64-ReleaseLTCG.exe",
        launch_args_template: "-batch \"{game_path}\"",
        system_path_hint: "bios",
        expected_extensions: &[".bin", ".cue", ".iso", ".img", ".pbp"],
    },
    PlatformConfig {
        id: "ps2",
        label: "PlayStation 2",
        emulator_name: "PCSX2",
        executable_hint: "pcsx2-qt.exe",
        launch_args_template: "-fullscreen -- \"{game_path}\"",
        system_path_hint: "bios",
        expected_extensions: &[".iso", ".bin", ".img"],
    },
    PlatformConfig {
        id: "gba",
        label: "Game Boy Advance",
        emulator_name: "mGBA",
        executable_hint: "mGBA.exe",
        launch_args_template: "{game_path}",
        system_path_hint: "",
        expected_extensions: &[".gba"],
    },
    PlatformConfig {
        id: "nes",
        label: "NES / Famicom",
        emulator_name: "Mesen",
        executable_hint: "Mesen.exe",
        launch_args_template: "{game_path}",
        system_path_hint: "",
        expected_extensions: &[".nes"],
    },
];

pub fn platform_config(platform: &str) -> Option<&'static PlatformConfig> {
    MVP_PLATFORM_CONFIGS
        .iter()
        .find(|config| config.id == platform)
}

pub fn is_mvp_platform(platform: &str) -> bool {
    MVP_PLATFORM_IDS.contains(&platform)
}

pub fn default_launch_args_template(platform: &str) -> Option<&'static str> {
    platform_config(platform).map(|config| config.launch_args_template)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mvp_platform_matrix_has_expected_defaults() {
        assert!(is_mvp_platform("switch"));
        assert!(!is_mvp_platform("dreamcast"));
        assert_eq!(default_launch_args_template("ps1"), Some("-batch \"{game_path}\""));
        assert_eq!(platform_config("ps2").map(|config| config.executable_hint), Some("pcsx2-qt.exe"));
    }
}
