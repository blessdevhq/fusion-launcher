// Kept outside src/bin: Tauri versions that scan that directory can otherwise
// bundle a stale fallback executable despite this target's required-features.
use std::env;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct ProgressLine<'a> {
    progress: f64,
    speed_bytes: u64,
    peers: usize,
    state: &'a str,
    downloaded_bytes: u64,
    total_bytes: u64,
}

#[derive(Deserialize)]
struct StdinCommand {
    action: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: fusion-torrent <magnet_link> <save_path>");
        std::process::exit(2);
    }

    let magnet = &args[1];
    let save_path = PathBuf::from(&args[2]);

    if !magnet.starts_with("magnet:") {
        eprintln!("error: first argument must be a magnet: URI");
        std::process::exit(2);
    }

    tokio::fs::create_dir_all(&save_path).await?;

    let session = Session::new_with_opts(
        save_path.clone(),
        SessionOptions {
            fastresume: true,
            ..Default::default()
        },
    )
    .await?;

    let response = session
        .add_torrent(
            AddTorrent::from_url(magnet),
            Some(AddTorrentOptions {
                overwrite: true,
                ..Default::default()
            }),
        )
        .await?;

    let handle = match response {
        AddTorrentResponse::Added(_, handle) => handle,
        AddTorrentResponse::AlreadyManaged(_, handle) => handle,
        _ => return Err("Unexpected AddTorrentResponse".into()),
    };

    let is_paused = Arc::new(AtomicBool::new(false));
    let stop_requested = Arc::new(AtomicBool::new(false));

    // Stdin command listener
    let is_paused_stdin = Arc::clone(&is_paused);
    let stop_requested_stdin = Arc::clone(&stop_requested);
    let session_stdin = Arc::clone(&session);
    let handle_stdin = handle.clone();
    let rt_handle = tokio::runtime::Handle::current();

    tokio::task::spawn_blocking(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let Ok(text) = line else { break };
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(cmd) = serde_json::from_str::<StdinCommand>(trimmed) {
                match cmd.action.as_str() {
                    "pause" => {
                        let _ = rt_handle.block_on(session_stdin.pause(&handle_stdin));
                        is_paused_stdin.store(true, Ordering::SeqCst);
                        eprintln!("info: torrent paused via stdin");
                    }
                    "resume" => {
                        let _ = rt_handle.block_on(session_stdin.unpause(&handle_stdin));
                        is_paused_stdin.store(false, Ordering::SeqCst);
                        eprintln!("info: torrent resumed via stdin");
                    }
                    "stop" => {
                        stop_requested_stdin.store(true, Ordering::SeqCst);
                        eprintln!("info: torrent stop requested via stdin");
                        break;
                    }
                    _ => {}
                }
            }
        }
    });

    let mut last_downloaded = 0u64;

    while !stop_requested.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let stats = handle.stats();
        let total = stats.total_bytes;
        let downloaded = stats.progress_bytes;
        let speed = downloaded.saturating_sub(last_downloaded);
        last_downloaded = downloaded;

        let paused = is_paused.load(Ordering::SeqCst);
        let progress = if total > 0 {
            ((downloaded as f64 / total as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            0.0
        };

        let state = if paused {
            "paused"
        } else if progress >= 100.0 || stats.finished {
            "finished"
        } else if total == 0 {
            "checking"
        } else {
            "downloading"
        };

        let live_peers = stats
            .live
            .as_ref()
            .map(|l| l.snapshot.peer_stats.live as usize)
            .unwrap_or(0);

        let payload = ProgressLine {
            progress: (progress * 100.0).round() / 100.0,
            speed_bytes: if paused { 0 } else { speed },
            peers: live_peers,
            state,
            downloaded_bytes: downloaded,
            total_bytes: total,
        };

        let json = serde_json::to_string(&payload)?;
        println!("{json}");
        let _ = io::stdout().flush();
    }

    Ok(())
}
