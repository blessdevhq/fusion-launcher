//! Abstraction over the underlying P2P download engine.
//!
//! `TorrentManager` (in `torrent.rs`) owns the download state machine, SQLite
//! persistence, progress events, startup restore, and save-dir safety cleanup.
//! It must not care *how* bytes arrive. This module defines the [`TorrentEngine`]
//! trait that captures the raw torrent operations the manager needs.
//!
//! The only implementation is [`crate::libtorrent_engine::LibtorrentEngine`],
//! which drives the libtorrent sidecar. libtorrent forces MSE/PE handshake
//! obfuscation, so it slips past DPI that resets plaintext BitTorrent handshakes
//! — the reason the previous in-process librqbit engine was replaced.

use async_trait::async_trait;

/// Engine-agnostic snapshot of a torrent's live transfer stats. `progress` is a
/// fraction in `0.0..=1.0`; the manager converts it to a percentage when needed.
#[derive(Debug, Clone, Default)]
pub struct EngineTorrentStats {
    pub progress: f64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub download_speed_bytes_per_sec: u64,
    pub upload_speed_bytes_per_sec: u64,
    pub peers_count: usize,
    pub finished: bool,
    pub error: Option<String>,
    /// Metadata-declared relative files, published before payload writes begin.
    pub inventory: Option<Vec<String>>,
}

/// Raw torrent operations the manager drives. Torrents are addressed by an
/// opaque numeric id the engine assigns on [`add_magnet`](TorrentEngine::add_magnet)
/// and the manager persists in `torrent_downloads.torrent_id`.
#[async_trait]
// Raw engine hooks include pause/resume for consumers that keep an engine id;
// the manager deliberately creates a new runtime generation on resume.
#[allow(dead_code)]
pub trait TorrentEngine: Send + Sync {
    /// Add a magnet and begin downloading into `save_dir`, returning the engine's
    /// id for the new torrent.
    async fn add_magnet(&self, magnet_uri: String, save_dir: String) -> Result<usize, String>;
    /// Permit payload writes only after the manager has reserved these exact files.
    async fn approve_inventory(&self, id: usize) -> Result<(), String>;
    /// Pause the torrent. A no-op if the engine no longer tracks `id`.
    async fn pause(&self, id: usize) -> Result<(), String>;
    /// Resume a paused torrent. A no-op if the engine no longer tracks `id`.
    async fn resume(&self, id: usize) -> Result<(), String>;
    /// Stop writers and await confirmed process exit, preserving all files.
    /// An error means quiescence is unconfirmed and cleanup must not proceed.
    async fn stop(&self, id: usize) -> Result<(), String>;
    /// Stop destination writers, including a failed add with no returned id.
    /// An error forbids cleanup and reuse of the destination.
    async fn stop_for_path(&self, save_dir: &str) -> Result<(), String>;
    /// Stop and remove engine bookkeeping. Payload cleanup belongs to the
    /// manager, which must separately prove ownership of any deleted files.
    async fn delete(&self, id: usize, delete_files: bool) -> Result<(), String>;
    /// Whether the engine still tracks a torrent with this id.
    fn exists(&self, id: usize) -> bool;
    /// Live stats for the torrent, or `None` if the engine no longer tracks it.
    fn stats(&self, id: usize) -> Option<EngineTorrentStats>;
}
