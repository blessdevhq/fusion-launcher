#![allow(unused_imports)]

use std::collections::HashMap;
use std::path::Path;

use chrono::{Duration, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::rom_hasher::RomHashes;
use crate::schema::{
    AssetInstallation, AssetView, CatalogGameView, DownloadRecord, EmulatorConfig, GameArtwork,
    GameMetadata, ProfileEmulatorConfig, ProfileSystemFileImport, RepositorySchema,
    RepositorySummary, ScrapeCandidate, ScrapeStateView, ScrapedGamePayload, TorrentDownloadRecord,
    TrustedExecutable,
};
use crate::security::global_id;
use crate::setup_profiles;

pub struct RepositoryStore {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct MetadataCacheEntry {
    pub payload: ScrapedGamePayload,
    pub match_kind: String,
}

#[derive(Debug, Clone)]
pub struct ScrapeOverrideRecord {
    pub provider_game_id: Option<String>,
    pub payload: Option<ScrapedGamePayload>,
    pub locked: bool,
    pub updated_at: String,
}

pub fn metadata_name_key(platform: &str, title: &str) -> String {
    let normalized_title = title
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "name:{}:{}",
        platform.trim().to_lowercase(),
        normalized_title
    )
}

pub fn steamgriddb_cache_key(game_id: &str) -> String {
    format!("sgdb:{}", game_id.trim())
}

mod catalog;
mod config;
mod downloads;
mod emulators;
mod metadata;
mod migrations;
mod repositories;
mod rows;

#[cfg(test)]
mod tests;
