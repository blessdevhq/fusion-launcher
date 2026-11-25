//! Local, explicitly approved patch tools. This trust store has no dependency
//! on source indexes, source/game executable approvals, or downloaded metadata.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::canonical::QualifiedGameId;
use super::errors::{ErrorCode, FusionError};
use super::fs_safety;
use super::integrity::ExecutableTrust;
use super::schema::PatchFormat;
use super::transforms::{PatchToolRegistry, PinnedPatchTool, VerifiedPatchRegistration};

const CONFIG_LIMIT: u64 = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostConfig {
    schema_version: u32,
    #[serde(default)]
    patch_tools: Vec<HostToolConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HostToolConfig {
    format: PatchFormat,
    executable: PathBuf,
    sha256: String,
    version: String,
    #[serde(default)]
    trusted: bool,
}

impl HostToolConfig {
    fn pinned(&self) -> PinnedPatchTool {
        PinnedPatchTool {
            executable: self.executable.clone(),
            sha256: self.sha256.to_ascii_lowercase(),
            version: self.version.clone(),
            format: self.format,
        }
    }
}

#[derive(Debug, Default)]
pub struct HostPatchTools {
    pub registry: PatchToolRegistry,
    pub warnings: Vec<String>,
}

/// Approvals exist only for checked SHA-256 config entries. A game approval in
/// SQLite, or a matching source path, can never populate this separate store.
#[derive(Debug, Default)]
pub struct HostPatchToolTrust {
    approved: BTreeMap<String, String>,
}

impl HostPatchToolTrust {
    fn subject() -> QualifiedGameId {
        QualifiedGameId {
            source_id: "fusion-host".into(),
            game_id: "patch-tools".into(),
        }
    }

    fn approve_checked(
        &mut self,
        tool: &PinnedPatchTool,
        registration: &VerifiedPatchRegistration,
    ) {
        self.approved
            .insert(path_key(&tool.executable), registration.blake3().to_owned());
    }
}

fn path_key(path: &Path) -> String {
    #[cfg(windows)]
    {
        path.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        path.to_string_lossy().to_string()
    }
}

impl ExecutableTrust for HostPatchToolTrust {
    fn is_trusted(
        &self,
        game: &QualifiedGameId,
        path: &str,
        blake3: &str,
    ) -> Result<bool, FusionError> {
        Ok(*game == Self::subject()
            && self
                .approved
                .get(&path_key(Path::new(path)))
                .is_some_and(|approved| approved.eq_ignore_ascii_case(blake3)))
    }
}

/// Load once at host startup. A missing file is an empty optional registry;
/// invalid structure rejects the configuration; unusable individual tools are
/// disabled and reported to the caller for logging. This API never downloads.
pub fn load_host_patch_tools(data_dir: &Path) -> Result<HostPatchTools, FusionError> {
    let path = data_dir.join("fusion-source-host.json");
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HostPatchTools::default())
        }
        Err(error) => return Err(fs_safety::io_error("Inspect host config", &path, error)),
        Ok(_) => {}
    }
    let file = fs_safety::open_read(&path)?;
    if file
        .metadata()
        .map_err(|error| fs_safety::io_error("Inspect host config", &path, error))?
        .len()
        > CONFIG_LIMIT
    {
        return Err(invalid("Host config exceeds 64 KiB."));
    }
    let mut bytes = Vec::new();
    file.take(CONFIG_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| fs_safety::io_error("Read host config", &path, error))?;
    if bytes.len() > CONFIG_LIMIT as usize {
        return Err(invalid("Host config exceeds 64 KiB."));
    }
    let config: HostConfig = serde_json::from_slice(&bytes)
        .map_err(|error| invalid(&format!("Invalid host patch-tool config: {error}")))?;
    if config.schema_version != 1 {
        return Err(invalid("Host config requires schema_version 1."));
    }
    // Reject ambiguous formats and malformed pins before approving anything.
    PatchToolRegistry::new(
        config
            .patch_tools
            .iter()
            .map(HostToolConfig::pinned)
            .collect(),
    )?;
    let mut result = HostPatchTools::default();
    let mut trust = HostPatchToolTrust::default();
    for entry in config.patch_tools {
        let tool = entry.pinned();
        if !entry.trusted {
            result.warnings.push(format!(
                "Patch tool {:?} disabled: trusted must be explicitly true.",
                tool.format
            ));
            continue;
        }
        match VerifiedPatchRegistration::inspect(&tool) {
            Ok(registration) => {
                trust.approve_checked(&tool, &registration);
                if !trust.is_trusted(
                    &HostPatchToolTrust::subject(),
                    &tool.executable.to_string_lossy(),
                    registration.blake3(),
                )? {
                    return Err(FusionError::new(
                        ErrorCode::UntrustedExecutable,
                        "Host patch-tool approval did not match checked bytes.",
                    ));
                }
                result.registry.register_verified(tool, registration)?;
            }
            Err(error) => result
                .warnings
                .push(format!("Patch tool {:?} disabled: {error}", tool.format)),
        }
    }
    Ok(result)
}

fn invalid(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidManifest, message)
}

#[cfg(test)]
mod tests {
    use super::super::cas::hash_file;
    use super::super::schema::DigestAlgorithm;
    use super::*;
    use serde_json::json;

    fn tool(path: &Path, trusted: Option<bool>, format: &str) -> serde_json::Value {
        let mut value = json!({"format":format,"executable":path,"sha256":hash_file(path, DigestAlgorithm::Sha256).unwrap(),"version":"approved-build-v1"});
        if let Some(trusted) = trusted {
            value["trusted"] = json!(trusted);
        }
        value
    }

    fn write_config(root: &Path, tools: Vec<serde_json::Value>) {
        std::fs::write(
            root.join("fusion-source-host.json"),
            json!({"schema_version":1,"patch_tools":tools}).to_string(),
        )
        .unwrap();
    }

    #[test]
    fn config_advertises_only_verified_approved_formats_and_clones_keep_guards() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("xdelta3.exe");
        let second = directory.path().join("bspatch.exe");
        std::fs::write(&first, b"fixture tool one").unwrap();
        std::fs::write(&second, b"fixture tool two").unwrap();
        write_config(
            directory.path(),
            vec![
                tool(&first, Some(true), "xdelta3"),
                tool(&second, None, "bsdiff"),
            ],
        );
        let loaded = load_host_patch_tools(directory.path()).unwrap();
        assert_eq!(
            loaded.registry.configured_formats(),
            vec![PatchFormat::Xdelta3]
        );
        assert_eq!(loaded.warnings.len(), 1);
        let clone = loaded.registry.clone();
        drop(loaded);
        #[cfg(windows)]
        {
            assert!(std::fs::write(&first, b"replaced").is_err());
            assert!(std::fs::rename(&first, directory.path().join("moved")).is_err());
        }
        drop(clone);
        std::fs::write(&first, b"updated").unwrap();
        let loaded = load_host_patch_tools(directory.path()).unwrap();
        assert!(loaded.registry.configured_formats().is_empty());
        assert_eq!(loaded.warnings.len(), 2);
    }

    #[test]
    fn config_rejects_unknown_fields_duplicates_invalid_paths_and_versions() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("tool.exe");
        std::fs::write(&executable, b"tool").unwrap();
        let good = tool(&executable, Some(true), "xdelta3");
        write_config(directory.path(), vec![good.clone(), good.clone()]);
        assert_eq!(
            load_host_patch_tools(directory.path()).unwrap_err().code,
            ErrorCode::DuplicateId
        );
        for (field, value) in [
            ("extra", json!(true)),
            ("executable", json!("relative.exe")),
            ("version", json!("")),
            ("sha256", json!("bad")),
            ("format", json!("unknown")),
        ] {
            let mut entry = good.clone();
            entry[field] = value;
            write_config(directory.path(), vec![entry]);
            assert!(
                load_host_patch_tools(directory.path()).is_err(),
                "field: {field}"
            );
        }
        std::fs::write(
            directory.path().join("fusion-source-host.json"),
            vec![b' '; CONFIG_LIMIT as usize + 1],
        )
        .unwrap();
        assert!(load_host_patch_tools(directory.path()).is_err());
    }

    #[test]
    fn missing_config_or_binary_and_game_trust_cannot_enable_host_tool() {
        let directory = tempfile::tempdir().unwrap();
        assert!(load_host_patch_tools(directory.path())
            .unwrap()
            .registry
            .configured_formats()
            .is_empty());
        let missing = directory.path().join("missing.exe");
        write_config(
            directory.path(),
            vec![
                json!({"format":"bsdiff","executable":missing,"sha256":"0".repeat(64),"version":"v1","trusted":true}),
            ],
        );
        let loaded = load_host_patch_tools(directory.path()).unwrap();
        assert!(loaded.registry.configured_formats().is_empty());
        assert_eq!(loaded.warnings.len(), 1);
        let executable = directory.path().join("tool.exe");
        std::fs::write(&executable, b"tool").unwrap();
        let pinned = PinnedPatchTool {
            executable: executable.clone(),
            sha256: hash_file(&executable, DigestAlgorithm::Sha256).unwrap(),
            version: "v1".into(),
            format: PatchFormat::Xdelta3,
        };
        let registration = VerifiedPatchRegistration::inspect(&pinned).unwrap();
        let mut trust = HostPatchToolTrust::default();
        trust.approve_checked(&pinned, &registration);
        let game = QualifiedGameId {
            source_id: "source".into(),
            game_id: "game".into(),
        };
        assert!(!trust
            .is_trusted(&game, &executable.to_string_lossy(), registration.blake3())
            .unwrap());
        assert!(trust
            .is_trusted(
                &HostPatchToolTrust::subject(),
                &executable.to_string_lossy(),
                registration.blake3()
            )
            .unwrap());
        assert!(!trust
            .is_trusted(
                &HostPatchToolTrust::subject(),
                &executable.to_string_lossy(),
                &"0".repeat(64)
            )
            .unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn approved_hash_cannot_register_tool_through_a_junction_parent() {
        use std::os::windows::process::CommandExt;
        let directory = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let executable = external.path().join("tool.exe");
        std::fs::write(&executable, b"approved bytes").unwrap();
        let junction = directory.path().join("tool-parent");
        let result = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(external.path())
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(result.status.success());
        let mut config = tool(&executable, Some(true), "xdelta3");
        config["executable"] = json!(junction.join("tool.exe"));
        write_config(directory.path(), vec![config]);
        let loaded = load_host_patch_tools(directory.path()).unwrap();
        assert!(loaded.registry.verified_formats().is_empty());
        assert_eq!(loaded.warnings.len(), 1);
        std::fs::remove_dir(junction).unwrap();
    }
}
