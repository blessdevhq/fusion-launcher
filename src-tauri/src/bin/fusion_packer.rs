//! Standalone authoring CLI. All repository logic lives in the shared library.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

use fusion_launcher_lib::fusion_source::packer::{
    ArtifactInspectionOptions, PackerAttestationRecipe, PackerCriticalFile, PackerEngine,
};
use fusion_launcher_lib::fusion_source::schema::{ArchiveFormat, CriticalFileRole};
use fusion_launcher_lib::fusion_source::{ErrorCode, FusionError};

const HELP: &str = r#"fusion-packer — Fusion Source 1.0 authoring

Usage:
  fusion-packer keygen --out <private.pem>
  fusion-packer build --recipe <recipe.json|recipe.toml> --out <dist_dir> [--signing-key <private.pem>]
  fusion-packer verify --source-dir <dist_dir>
  fusion-packer inspect-artifact --file <path> [--critical <rel_path>:<role>]...
      [--archive raw|zip|seven_zip] [--raw-path <rel_path>] [--auto-main-executable]
      [--release-id <id> --tag <tag> --asset-id <id> --asset-url <https_url>]

Critical roles: main_executable, injector_dll, core_library
Recipe input paths resolve relative to the recipe file. Output is JSON on stdout;
typed FusionError diagnostics are JSON on stderr. Exit codes: 0 success, 1 failure,
2 invalid command arguments. Attestations require all four release/asset flags.
Signing key: --signing-key takes priority over FUSION_PACKER_SIGNING_KEY.
"#;

#[derive(Debug)]
enum Command {
    Help,
    Keygen {
        out: PathBuf,
    },
    Build {
        recipe: PathBuf,
        out: PathBuf,
        signing_key: Option<PathBuf>,
    },
    Verify {
        source_dir: PathBuf,
    },
    Inspect {
        file: PathBuf,
        options: Box<ArtifactInspectionOptions>,
    },
}

fn argument_error(message: impl Into<String>) -> FusionError {
    FusionError::new(ErrorCode::InvalidManifest, message)
        .with_detail("context", "cli_arguments")
        .with_detail("help", "Run fusion-packer --help for usage.")
}

fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, FusionError> {
    let mut arguments = arguments.into_iter();
    let Some(command) = arguments.next() else {
        return Ok(Command::Help);
    };
    if command == "--help" || command == "-h" || command == "help" {
        return Ok(Command::Help);
    }
    let command = command
        .to_str()
        .ok_or_else(|| argument_error("Command must be Unicode."))?;
    let allowed: &[&str] = match command {
        "keygen" => &["--out"],
        "build" => &["--recipe", "--out", "--signing-key"],
        "verify" => &["--source-dir"],
        "inspect-artifact" => &[
            "--file",
            "--critical",
            "--archive",
            "--raw-path",
            "--auto-main-executable",
            "--release-id",
            "--tag",
            "--asset-id",
            "--asset-url",
        ],
        _ => return Err(argument_error(format!("Unknown command: {command}"))),
    };
    let mut values = BTreeMap::<String, Vec<OsString>>::new();
    while let Some(option) = arguments.next() {
        if option == "--help" || option == "-h" {
            return Ok(Command::Help);
        }
        let option = option
            .to_str()
            .ok_or_else(|| argument_error("Option names must be Unicode."))?;
        if !allowed.contains(&option) {
            return Err(argument_error(format!(
                "Unknown option for {command}: {option}"
            )));
        }
        if values.contains_key(option) && option != "--critical" {
            return Err(argument_error(format!("Option supplied twice: {option}")));
        }
        let value = if option == "--auto-main-executable" {
            OsString::from("true")
        } else {
            let value = arguments
                .next()
                .ok_or_else(|| argument_error(format!("Missing value for {option}")))?;
            if value.to_string_lossy().starts_with("--") {
                return Err(argument_error(format!("Missing value for {option}")));
            }
            value
        };
        values.entry(option.to_owned()).or_default().push(value);
    }
    let required_path = |key: &str| -> Result<PathBuf, FusionError> {
        let value = values
            .get(key)
            .and_then(|values| values.first())
            .ok_or_else(|| argument_error(format!("Required option: {key}")))?;
        if value.is_empty() {
            return Err(argument_error(format!("Empty path for {key}")));
        }
        Ok(PathBuf::from(value))
    };
    let optional_string = |key: &str| -> Result<Option<String>, FusionError> {
        values
            .get(key)
            .and_then(|values| values.first())
            .map(|value| {
                value
                    .to_str()
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .ok_or_else(|| argument_error(format!("{key} requires nonempty Unicode text.")))
            })
            .transpose()
    };
    match command {
        "keygen" => Ok(Command::Keygen {
            out: required_path("--out")?,
        }),
        "build" => Ok(Command::Build {
            recipe: required_path("--recipe")?,
            out: required_path("--out")?,
            signing_key: values
                .contains_key("--signing-key")
                .then(|| required_path("--signing-key"))
                .transpose()?,
        }),
        "verify" => Ok(Command::Verify {
            source_dir: required_path("--source-dir")?,
        }),
        "inspect-artifact" => {
            let archive = optional_string("--archive")?
                .map(|value| match value.as_str() {
                    "raw" => Ok(ArchiveFormat::Raw),
                    "zip" => Ok(ArchiveFormat::Zip),
                    "seven_zip" | "7z" => Ok(ArchiveFormat::SevenZip),
                    _ => Err(argument_error("--archive must be raw, zip or seven_zip.")),
                })
                .transpose()?;
            let critical_files = values.get("--critical").into_iter().flatten()
                .map(|value| {
                    let value = value.to_str().ok_or_else(|| argument_error("Critical paths must be Unicode."))?;
                    let (path, role) = value.rsplit_once(':').ok_or_else(|| {
                        argument_error("--critical expects <relative_path>:<role>.")
                    })?;
                    let role = match role {
                        "main_executable" | "MainExecutable" => CriticalFileRole::MainExecutable,
                        "injector_dll" | "InjectorDll" => CriticalFileRole::InjectorDll,
                        "core_library" | "CoreLibrary" => CriticalFileRole::CoreLibrary,
                        _ => return Err(argument_error("Unknown critical role. Use main_executable, injector_dll or core_library.")),
                    };
                    Ok(PackerCriticalFile { path: path.to_owned(), role })
                }).collect::<Result<Vec<_>, FusionError>>()?;
            let release_id = optional_string("--release-id")?;
            let release_tag = optional_string("--tag")?;
            let asset_id = optional_string("--asset-id")?;
            let asset_url = optional_string("--asset-url")?;
            let attestation = match (release_id, release_tag, asset_id, asset_url) {
                (None, None, None, None) => None,
                (Some(release_id), Some(tag), Some(asset_id), Some(asset_url)) => Some(PackerAttestationRecipe {
                    release_id, release_tag: Some(tag), asset_id, asset_url, tool_versions: BTreeMap::new(),
                }),
                _ => return Err(argument_error("Attestation requires --release-id, --tag, --asset-id and --asset-url together.")),
            };
            Ok(Command::Inspect {
                file: required_path("--file")?,
                options: Box::new(ArtifactInspectionOptions {
                    archive,
                    raw_path: optional_string("--raw-path")?,
                    critical_files,
                    auto_main_executable: values.contains_key("--auto-main-executable"),
                    attestation,
                }),
            })
        }
        _ => unreachable!("known command"),
    }
}

fn run(command: Command) -> Result<(), FusionError> {
    let engine = PackerEngine::new();
    let output = match command {
        Command::Help => {
            print!("{HELP}");
            return Ok(());
        }
        Command::Keygen { out } => serde_json::to_value(
            fusion_launcher_lib::fusion_source::signatures::keygen(&out)?,
        ),
        Command::Build {
            recipe,
            out,
            signing_key,
        } => {
            let signing_key = signing_key
                .or_else(|| std::env::var_os("FUSION_PACKER_SIGNING_KEY").map(PathBuf::from));
            serde_json::to_value(engine.build_file_with_signing_key(
                &recipe,
                &out,
                signing_key.as_deref(),
            )?)
        }
        Command::Verify { source_dir } => serde_json::to_value(engine.verify(&source_dir)?),
        Command::Inspect { file, options } => {
            serde_json::to_value(engine.inspect_artifact(&file, &options)?)
        }
    }
    .map_err(|error| FusionError::new(ErrorCode::InvalidJson, error.to_string()))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&output)
            .map_err(|error| FusionError::new(ErrorCode::InvalidJson, error.to_string()))?
    );
    Ok(())
}

fn diagnostic(error: &FusionError) {
    eprintln!(
        "{}",
        serde_json::to_string(error).unwrap_or_else(|_| error.to_string())
    );
}

fn main() -> ExitCode {
    match parse(std::env::args_os().skip(1)) {
        Err(error) => {
            diagnostic(&error);
            ExitCode::from(2)
        }
        Ok(command) => match run(command) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                diagnostic(&error);
                ExitCode::FAILURE
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(args: &[&str]) -> Result<Command, FusionError> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn accepts_repeated_critical_roles_and_complete_attestation() {
        let Command::Inspect { file, options } = command(&[
            "inspect-artifact",
            "--file",
            "эмулятор.zip",
            "--critical",
            "bin/run.exe:MainExecutable",
            "--critical",
            "inject.dll:injector_dll",
            "--release-id",
            "42",
            "--tag",
            "v1",
            "--asset-id",
            "99",
            "--asset-url",
            "https://example.org/runtime.zip",
        ])
        .unwrap() else {
            panic!("expected inspection");
        };
        assert_eq!(file, PathBuf::from("эмулятор.zip"));
        assert_eq!(options.critical_files.len(), 2);
        assert_eq!(
            options.attestation.unwrap().release_tag.as_deref(),
            Some("v1")
        );
    }

    #[test]
    fn rejects_unknown_duplicate_missing_and_partial_arguments() {
        for args in [
            vec!["build", "--recipe", "recipe.json"],
            vec!["verify", "--source-dir", "dist", "--source-dir", "other"],
            vec!["build", "--recipe", "--out", "dist"],
            vec!["verify", "--file", "dist"],
            vec!["inspect-artifact", "--file", "a.zip", "--tag", "v1"],
            vec![
                "inspect-artifact",
                "--file",
                "a.zip",
                "--critical",
                "a.exe:unknown",
            ],
        ] {
            assert!(command(&args).is_err(), "arguments accepted: {args:?}");
        }
    }

    #[test]
    fn accepts_keygen_and_explicit_native_signing_key_path() {
        assert!(matches!(
            command(&["keygen", "--out", "private.pem"]).unwrap(),
            Command::Keygen { .. }
        ));
        let Command::Build { signing_key, .. } = command(&[
            "build",
            "--recipe",
            "r.json",
            "--out",
            "dist",
            "--signing-key",
            "ключ.pem",
        ])
        .unwrap() else {
            panic!("expected build")
        };
        assert_eq!(signing_key, Some(PathBuf::from("ключ.pem")));
        assert!(command(&["keygen"]).is_err());
    }
}
