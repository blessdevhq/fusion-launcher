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
}

/// Raw torrent operations the manager drives. Torrents are addressed by an
/// opaque numeric id the engine assigns on [`add_magnet`](TorrentEngine::add_magnet)
/// and the manager persists in `torrent_downloads.torrent_id`.
#[async_trait]
pub trait TorrentEngine: Send + Sync {
    /// Add a magnet and begin downloading into `save_dir`, returning the engine's
    /// id for the new torrent.
    async fn add_magnet(&self, magnet_uri: String, save_dir: String) -> Result<usize, String>;
    /// Pause the torrent. A no-op if the engine no longer tracks `id`.
    async fn pause(&self, id: usize) -> Result<(), String>;
    /// Resume a paused torrent. A no-op if the engine no longer tracks `id`.
    async fn resume(&self, id: usize) -> Result<(), String>;
    /// Remove the torrent; when `delete_files` is set, also delete partial data.
    async fn delete(&self, id: usize, delete_files: bool) -> Result<(), String>;
    /// Whether the engine still tracks a torrent with this id.
    fn exists(&self, id: usize) -> bool;
    /// Live stats for the torrent, or `None` if the engine no longer tracks it.
    fn stats(&self, id: usize) -> Option<EngineTorrentStats>;
}
