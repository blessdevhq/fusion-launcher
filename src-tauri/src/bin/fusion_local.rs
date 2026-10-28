//! Offline CLI host for the same Fusion Source engine used by the desktop app.
#![cfg_attr(windows, windows_subsystem = "windows")]
use fusion_launcher_lib::fusion_source::dto::GameOptions;
use fusion_launcher_lib::fusion_source::{offline::OfflineLibrary, ErrorCode, FusionError};
use std::path::PathBuf;

fn error(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidManifest, message)
}

async fn run() -> Result<serde_json::Value, FusionError> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct LauncherConfig {
            source_dir: String,
            library: String,
            game: String,
            #[serde(default)]
            eden: bool,
        }
        let executable = std::env::current_exe().map_err(|_| error("Cannot locate launcher"))?;
        let config = executable
            .parent()
            .ok_or_else(|| error("No launcher directory"))?
            .join("fusion-local-launch.json");
        if config.exists() {
            use std::io::Read;
            let file = fusion_launcher_lib::fusion_source::fs_safety::open_read(&config)?;
            let config: LauncherConfig = serde_json::from_reader(file.take(64 * 1024))
                .map_err(|_| error("Invalid local launcher configuration"))?;
            args = vec![
                if config.eden {
                    "launch-eden".into()
                } else {
                    "launch".into()
                },
                "--source-dir".into(),
                config.source_dir,
                "--library".into(),
                config.library,
                "--game".into(),
                config.game,
            ];
        }
    }
    if args.is_empty() || args == ["--help"] {
        println!("fusion-local <install|status|options|update|trust|launch|launch-eden|rollback> --source-dir <packer-output> --library <private-root> --game <id>\ninstall: [--workspace-root <root>]\noptions: --options-file <GameOptions.json> (stages selection; update applies it)\ntrust: --path <workspace-path> --blake3 <approved-digest> OR --trust-file <approvals.json>\nNo network access. update uses the selected source revision and preserves saves; rollback restores a snapshot, not old saves. launch waits for the supervised process tree to exit. Trust is explicit for each exact digest; source JSON cannot grant it.");
        return Ok(serde_json::json!({"help":true}));
    }
    if ![
        "install",
        "status",
        "options",
        "update",
        "trust",
        "launch",
        "launch-eden",
        "rollback",
    ]
    .contains(&args[0].as_str())
    {
        return Err(error("Unknown command"));
    }
    let mut values = std::collections::BTreeMap::new();
    let mut rest = args[1..].chunks_exact(2);
    for pair in &mut rest {
        if ![
            "--source-dir",
            "--library",
            "--game",
            "--path",
            "--blake3",
            "--workspace-root",
            "--trust-file",
            "--options-file",
        ]
        .contains(&pair[0].as_str())
            || values.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err(error("Unknown or repeated option"));
        }
    }
    if !rest.remainder().is_empty() {
        return Err(error("Option requires a value"));
    }
    let required = |key| {
        values
            .get(key)
            .copied()
            .ok_or_else(|| error("Missing required option"))
    };
    if args[0] != "trust"
        && (values.contains_key("--path")
            || values.contains_key("--blake3")
            || values.contains_key("--trust-file"))
    {
        return Err(error("Trust options require the trust command"));
    }
    if values.contains_key("--workspace-root") && args[0] != "install" {
        return Err(error("Workspace root is selected during install"));
    }
    if values.contains_key("--options-file") && args[0] != "options" {
        return Err(error("Options file requires the options command"));
    }
    // Validate local selection before opening (and refreshing) the library.
    let selection = if args[0] == "options" {
        use std::io::Read;
        let file = std::path::absolute(required("--options-file")?)
            .map_err(|_| error("Invalid options file path"))?;
        let file = fusion_launcher_lib::fusion_source::fs_safety::open_read(&file)?;
        Some(
            serde_json::from_reader::<_, GameOptions>(file.take(64 * 1024))
                .map_err(|_| error("Invalid GameOptions JSON"))?,
        )
    } else {
        None
    };
    let source = PathBuf::from(required("--source-dir")?);
    let library = PathBuf::from(required("--library")?);
    let host = OfflineLibrary::open(&source, &library, required("--game")?).await?;
    match args[0].as_str() {
        "install" => Ok(serde_json::to_value(
            host.install_at(values.get("--workspace-root").map(|v| (*v).to_owned()))
                .await?,
        )
        .expect("serializable progress")),
        "status" => Ok(serde_json::to_value(host.state().await?).expect("serializable state")),
        "options" => Ok(serde_json::to_value(
            host.set_options(selection.expect("validated options"))
                .await?,
        )
        .expect("serializable state")),
        "update" => Ok(serde_json::to_value(host.update().await?).expect("serializable progress")),
        "trust" => {
            if let Some(file) = values.get("--trust-file") {
                if values.contains_key("--path") || values.contains_key("--blake3") {
                    return Err(error("Choose explicit pair or trust file"));
                }
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Approval {
                    path: String,
                    blake3: String,
                }
                use std::io::Read;
                let handle = fusion_launcher_lib::fusion_source::fs_safety::open_read(
                    &std::path::absolute(file).map_err(|_| error("Invalid trust file"))?,
                )?;
                let files: Vec<Approval> = serde_json::from_reader(handle.take(64 * 1024))
                    .map_err(|_| error("Invalid trust list"))?;
                host.trust_many(files.into_iter().map(|f| (f.path, f.blake3)).collect())
                    .await?;
            } else {
                host.trust(required("--path")?, required("--blake3")?)
                    .await?;
            }
            Ok(serde_json::json!({"trusted":true}))
        }
        "launch" => Ok(serde_json::to_value(host.launch().await?).expect("serializable progress")),
        "launch-eden" => {
            Ok(serde_json::to_value(host.launch_eden().await?).expect("serializable progress"))
        }
        "rollback" => {
            Ok(serde_json::to_value(host.rollback().await?).expect("serializable progress"))
        }
        _ => unreachable!(),
    }
}

fn main() -> std::process::ExitCode {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Tokio runtime");
    match runtime.block_on(run()) {
        Ok(value) => {
            println!("{value}");
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!(
                "{}",
                serde_json::to_string(&error).expect("serializable error")
            );
            std::process::ExitCode::FAILURE
        }
    }
}
