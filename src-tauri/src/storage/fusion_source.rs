//! Fusion-only SQLite persistence. None of these records enter the legacy catalog.
//! Source identities remain reserved after disconnect, so reconnecting an ID cannot
//! change the URL or public-key metadata of an installed or historical source.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::fusion_source::canonical::{
    CanonicalGameManifest, CanonicalSourceIndex, QualifiedGameId,
};
use crate::fusion_source::dto::{
    CatalogLocalState, FusionCatalogRecord, FusionCrashRecord, FusionInstallationRecord,
    FusionManifestRecord, FusionOperationRecord, FusionSourceRecord, FusionWorkspaceRecord,
    GameOptions,
};
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::evolution::SchemaUpcaster;
use crate::fusion_source::layers::LayerInput;
use crate::fusion_source::schema::{SourceGameEntry, SourceIndex};
use crate::fusion_source::telemetry::{
    LogSanitizationLimits, SanitizedCrashReport, TelemetryContext, TelemetrySanitizer,
};
use crate::fusion_source::workspace::WorkspaceSnapshot;

use super::RepositoryStore;

fn database_error(error: rusqlite::Error) -> FusionError {
    FusionError::new(ErrorCode::Io, "The Fusion database operation failed.")
        .with_detail("databaseError", error.to_string())
}

fn json_error(error: serde_json::Error) -> FusionError {
    FusionError::new(
        ErrorCode::InvalidManifest,
        "A stored Fusion record is invalid.",
    )
    .with_detail("recordError", error.to_string())
}

fn validate_game(game: &QualifiedGameId) -> Result<(), FusionError> {
    for id in [&game.source_id, &game.game_id] {
        if id.is_empty()
            || id.len() > 128
            || matches!(id.as_str(), "." | "..")
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(FusionError::new(
                ErrorCode::InvalidId,
                "A Fusion storage identifier is invalid.",
            ));
        }
    }
    Ok(())
}

impl RepositoryStore {
    pub fn fusion_local_source(
        &self,
        source_id: &str,
    ) -> Result<Option<crate::local_sources::LocalSourceBinding>, FusionError> {
        self.conn.query_row(
            "SELECT l.directory,l.prepare_eden FROM fusion_local_sources l JOIN fusion_sources s ON s.id=l.source_id WHERE l.source_id=?1 AND s.connected=1",
            params![source_id],
            |row| Ok(crate::local_sources::LocalSourceBinding { directory: row.get(0)?, prepare_eden: row.get(1)? }),
        ).optional().map_err(database_error)
    }

    /// Bind only host-selected directories, atomically with the verified index.
    /// Source manifests cannot create or replace this filesystem authority.
    pub fn fusion_connect_local_source(
        &self,
        index: &CanonicalSourceIndex,
        binding: &crate::local_sources::LocalSourceBinding,
    ) -> Result<FusionSourceRecord, FusionError> {
        validated_index(index)?;
        let directory = Path::new(&binding.directory);
        if !directory.is_absolute() {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "Select an absolute source directory.",
            ));
        }
        let _guard = crate::fusion_source::fs_safety::guard(directory)?;
        let transport = crate::fusion_source::offline::OfflineTransport::open(
            directory,
            &index.source_info.id,
        )?;
        let url = transport.index_url();
        let tx = self.conn.unchecked_transaction().map_err(database_error)?;
        // Other hosts sharing the database cannot move a live operation's input.
        let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM fusion_operations WHERE source_id=?1 AND status IN ('queued','running','cancelling'))", params![index.source_info.id], |row| row.get(0)).map_err(database_error)?;
        if active {
            return Err(FusionError::new(
                ErrorCode::OperationInProgress,
                "Wait for this source's active operation before replacing its directory.",
            ));
        }
        write_source(&tx, index, &url)?;
        tx.execute("INSERT INTO fusion_local_sources(source_id,directory,prepare_eden) VALUES (?1,?2,?3) ON CONFLICT(source_id) DO UPDATE SET directory=excluded.directory,prepare_eden=excluded.prepare_eden", params![index.source_info.id,binding.directory,binding.prepare_eden]).map_err(database_error)?;
        tx.commit().map_err(database_error)?;
        self.fusion_list_sources()?
            .into_iter()
            .find(|s| s.index.source_info.id == index.source_info.id)
            .ok_or_else(|| FusionError::new(ErrorCode::Io, "The connected source disappeared."))
    }

    pub fn fusion_bind_installation_root(
        &self,
        game: &QualifiedGameId,
        library_root: &Path,
        external_root: Option<&Path>,
    ) -> Result<(), FusionError> {
        validate_game(game)?;
        if !library_root.is_absolute() || external_root.is_some_and(|root| !root.is_absolute()) {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "Installation roots must be absolute host-approved paths.",
            ));
        }
        let root = library_root.to_string_lossy().into_owned();
        let external = external_root.map(|root| root.to_string_lossy().into_owned());
        let tx = self.conn.unchecked_transaction().map_err(database_error)?;
        ensure_installation(&tx, game)?;
        let existing=tx.query_row("SELECT source_id,game_id,options_json,active_snapshot_id,status,library_root,external_workspace_root,active_version,active_manifest_blake3 FROM fusion_installations WHERE source_id=?1 AND game_id=?2",params![game.source_id,game.game_id],installation_row).map_err(database_error)?;
        if (existing.active_snapshot_id.is_some() || existing.status == "staging")
            && existing.library_root.is_some()
            && (existing.library_root.as_deref() != Some(root.as_str())
                || existing.external_workspace_root != external)
        {
            return Err(FusionError::new(
                ErrorCode::SnapshotConflict,
                "An existing installation or interrupted build owns different workspace roots.",
            ));
        }
        tx.execute("UPDATE fusion_installations SET library_root=?3,external_workspace_root=?4,status=CASE WHEN active_snapshot_id IS NULL THEN 'staging' ELSE status END,updated_at=?5 WHERE source_id=?1 AND game_id=?2",params![game.source_id,game.game_id,root,external,Utc::now().to_rfc3339()]).map_err(database_error)?;
        tx.commit().map_err(database_error)
    }

    pub fn fusion_add_source(
        &self,
        index: &CanonicalSourceIndex,
        url: &str,
    ) -> Result<FusionSourceRecord, FusionError> {
        validated_index(index)?;
        let parsed = reqwest::Url::parse(url)
            .map_err(|_| FusionError::new(ErrorCode::InvalidUri, "The source URL is invalid."))?;
        if parsed.scheme() != "https"
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.fragment().is_some()
        {
            return Err(FusionError::new(
                ErrorCode::InvalidUri,
                "Sources require an HTTPS URL without credentials or fragments.",
            ));
        }
        let tx = self.conn.unchecked_transaction().map_err(database_error)?;
        write_source(&tx, index, url)?;
        tx.commit().map_err(database_error)?;
        self.fusion_list_sources()?
            .into_iter()
            .find(|source| source.index.source_info.id == index.source_info.id)
            .ok_or_else(|| FusionError::new(ErrorCode::Io, "The stored Fusion source disappeared."))
    }

    pub fn fusion_refresh_source(
        &self,
        index: &CanonicalSourceIndex,
        url: &str,
    ) -> Result<FusionSourceRecord, FusionError> {
        self.fusion_add_source(index, url)
    }

    pub fn fusion_list_sources(&self) -> Result<Vec<FusionSourceRecord>, FusionError> {
        let mut statement=self.conn.prepare("SELECT index_json,url,connected_at,refreshed_at FROM fusion_sources WHERE connected=1 ORDER BY id").map_err(database_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(database_error)?;
        rows.map(|row| {
            let (json, url, connected_at, refreshed_at) = row.map_err(database_error)?;
            let index = serde_json::from_str(&json).map_err(json_error)?;
            validated_index(&index)?;
            Ok(FusionSourceRecord {
                index,
                url,
                connected_at,
                refreshed_at,
            })
        })
        .collect()
    }

    pub fn fusion_pinned_source_identity(
        &self,
        source_id: &str,
        index_url: &str,
    ) -> Result<Option<crate::fusion_source::signatures::PinnedSourceIdentity>, FusionError> {
        let mut statement = self
            .conn
            .prepare(
                "SELECT id,url,public_key FROM fusion_sources WHERE id=?1 COLLATE NOCASE OR url=?2",
            )
            .map_err(database_error)?;
        let identities = statement
            .query_map(params![source_id, index_url], |row| {
                Ok(crate::fusion_source::signatures::PinnedSourceIdentity {
                    source_id: row.get(0)?,
                    url: row.get(1)?,
                    public_key: row.get(2)?,
                })
            })
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        if identities.len() > 1 {
            return Err(FusionError::new(
                ErrorCode::InvalidSourceSignature,
                "The source URL and ID belong to different reserved identities.",
            ));
        }
        Ok(identities.into_iter().next())
    }

    pub fn fusion_list_catalog(&self) -> Result<Vec<FusionCatalogRecord>, FusionError> {
        let mut statement=self.conn.prepare("SELECT e.source_id,s.name,e.entry_json FROM fusion_source_entries e JOIN fusion_sources s ON s.id=e.source_id WHERE s.connected=1 ORDER BY e.source_id,e.game_id").map_err(database_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(database_error)?;
        rows.map(|row| {
            let (source_id, source_name, entry_json) = row.map_err(database_error)?;
            let entry: SourceGameEntry = serde_json::from_str(&entry_json).map_err(json_error)?;
            Ok(FusionCatalogRecord {
                source_id,
                source_name,
                entry,
            })
        })
        .collect()
    }

    /// Read all catalog enrichment in bounded queries. Only metadata is checked;
    /// the launch guard still rehashes critical game files immediately before launch.
    pub fn fusion_catalog_local_state(&self) -> Result<CatalogLocalState, FusionError> {
        let mut state = CatalogLocalState {
            installations: self
                .fusion_list_installations()?
                .into_iter()
                .map(|record| (record.game.clone(), record))
                .collect(),
            ..CatalogLocalState::default()
        };
        let mut trust_statement = self.conn.prepare("SELECT asset_id,local_path,sha256 FROM trusted_executables WHERE sha256 LIKE 'blake3:%'").map_err(database_error)?;
        let trusts = trust_statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(database_error)?
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(database_error)?;
        let mut snapshots = self.conn.prepare("SELECT s.id,s.source_id,s.game_id,s.manifest_blake3,s.snapshot_json,s.snapshot_blake3 FROM fusion_workspace_snapshots s JOIN fusion_installations i ON i.active_snapshot_id=s.id AND i.source_id=s.source_id AND i.game_id=s.game_id").map_err(database_error)?;
        for row in snapshots
            .query_map([], workspace_row)
            .map_err(database_error)?
        {
            // A damaged installation remains visible but cannot be advertised as ready.
            let Ok(record) = decode_workspace(row.map_err(database_error)?) else {
                continue;
            };
            let snapshot: WorkspaceSnapshot =
                serde_json::from_str(&record.snapshot_json).map_err(json_error)?;
            if SchemaUpcaster::validate_canonical(&snapshot.manifest).is_err()
                || blake3::hash(&serde_json::to_vec(&snapshot.manifest).map_err(json_error)?)
                    .to_hex()
                    .as_str()
                    != snapshot.canonical_blake3
                || snapshot.plan.game != snapshot.game
            {
                continue;
            }
            let trusted = crate::fusion_source::service::critical_inventory(&snapshot)
                .into_iter()
                .filter_map(|(path, file)| {
                    let id = trust_id(&snapshot.game, &path, &file.blake3).ok()?;
                    trusts
                        .contains(&(
                            id,
                            path.clone(),
                            format!("blake3:{}", file.blake3.to_ascii_lowercase()),
                        ))
                        .then_some((path, file.blake3))
                })
                .collect();
            state
                .trusted_executables
                .insert(snapshot.game.clone(), trusted);
            state.snapshots.insert(snapshot.game.clone(), snapshot);
        }
        let mut manifests = self.conn.prepare("SELECT m.source_id,m.game_id,m.manifest_blake3,m.raw_bytes,m.canonical_json,m.canonical_blake3 FROM fusion_manifest_snapshots m JOIN fusion_source_entries e ON e.source_id=m.source_id AND e.game_id=m.game_id WHERE lower(json_extract(e.entry_json,'$.manifest_blake3'))=m.manifest_blake3").map_err(database_error)?;
        let rows = manifests
            .query_map([], |row| {
                Ok((
                    QualifiedGameId {
                        source_id: row.get(0)?,
                        game_id: row.get(1)?,
                    },
                    row.get::<_, String>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(database_error)?;
        for row in rows {
            let (game, digest, raw, canonical_json, canonical_digest) =
                row.map_err(database_error)?;
            if blake3::hash(&raw).to_hex().as_str() != digest
                || blake3::hash(canonical_json.as_bytes()).to_hex().as_str() != canonical_digest
            {
                continue;
            }
            let Ok(canonical) = serde_json::from_str::<CanonicalGameManifest>(&canonical_json)
            else {
                continue;
            };
            if canonical.source_id == game.source_id
                && canonical.id == game.game_id
                && SchemaUpcaster::validate_canonical(&canonical).is_ok()
            {
                state.manifests.insert(game, canonical);
            }
        }
        let mut operations = self.conn.prepare("SELECT source_id,game_id FROM fusion_operations WHERE status IN ('queued','running','cancelling')").map_err(database_error)?;
        state.active_operations = operations
            .query_map([], |row| {
                Ok(QualifiedGameId {
                    source_id: row.get(0)?,
                    game_id: row.get(1)?,
                })
            })
            .map_err(database_error)?
            .collect::<Result<_, _>>()
            .map_err(database_error)?;
        Ok(state)
    }

    pub fn fusion_remove_source(&self, source_id: &str) -> Result<(), FusionError> {
        validate_game(&QualifiedGameId {
            source_id: source_id.into(),
            game_id: "validation".into(),
        })?;
        let tx = self.conn.unchecked_transaction().map_err(database_error)?;
        let busy:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM fusion_operations WHERE source_id=?1 AND status IN ('queued','running','cancelling'))",params![source_id],|row|row.get(0)).map_err(database_error)?;
        if busy {
            return Err(FusionError::new(
                ErrorCode::OperationInProgress,
                "A source operation is still active.",
            ));
        }
        let installed:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM fusion_installations WHERE source_id=?1 AND (active_snapshot_id IS NOT NULL OR status='staging'))",params![source_id],|row|row.get(0)).map_err(database_error)?;
        if installed {
            return Err(FusionError::new(
                ErrorCode::SnapshotConflict,
                "Remove the source's installed games before disconnecting it.",
            ));
        }
        tx.execute(
            "DELETE FROM fusion_source_entries WHERE source_id=?1",
            params![source_id],
        )
        .map_err(database_error)?;
        tx.execute(
            "UPDATE fusion_sources SET connected=0 WHERE id=?1",
            params![source_id],
        )
        .map_err(database_error)?;
        tx.commit().map_err(database_error)
    }

    pub fn fusion_put_manifest(&self, record: &FusionManifestRecord) -> Result<(), FusionError> {
        validate_game(&record.game)?;
        validate_digest(&record.digest)?;
        if record.raw_bytes.len() > crate::net::JSON_BODY_LIMIT {
            return Err(FusionError::new(
                ErrorCode::TransferLimitExceeded,
                "The manifest exceeds the metadata storage budget.",
            ));
        }
        if blake3::hash(&record.raw_bytes).to_hex().as_str() != record.digest.to_ascii_lowercase() {
            return Err(FusionError::new(
                ErrorCode::ManifestDigestMismatch,
                "Manifest bytes do not match the verified source digest.",
            ));
        }
        let raw = std::str::from_utf8(&record.raw_bytes).map_err(|_| {
            FusionError::new(ErrorCode::InvalidJson, "Manifest bytes are not UTF-8 JSON.")
        })?;
        let wire = SchemaUpcaster::parse_game_manifest(raw)?;
        SchemaUpcaster::validate_canonical(&record.canonical)?;
        if wire.id != record.game.game_id
            || record.canonical.source_id != record.game.source_id
            || record.canonical.id != record.game.game_id
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "The verified manifest identity changed.",
            ));
        }
        let canonical_json = serde_json::to_string(&record.canonical).map_err(json_error)?;
        let canonical_blake3 = blake3::hash(canonical_json.as_bytes()).to_hex().to_string();
        let digest = record.digest.to_ascii_lowercase();
        let tx = self.conn.unchecked_transaction().map_err(database_error)?;
        let existing:Option<(Vec<u8>,String)>=tx.query_row("SELECT raw_bytes,canonical_json FROM fusion_manifest_snapshots WHERE source_id=?1 AND game_id=?2 AND manifest_blake3=?3",params![record.game.source_id,record.game.game_id,digest],|row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(database_error)?;
        if let Some((bytes, _canonical)) = existing {
            if bytes != record.raw_bytes {
                return Err(FusionError::new(
                    ErrorCode::SnapshotConflict,
                    "An immutable manifest snapshot already has different source bytes.",
                ));
            }
            // The source bytes stay immutable. A child's derived canonical form
            // may change when an extends parent is refreshed; each installed
            // workspace separately records its exact resolved canonical form.
            tx.execute("UPDATE fusion_manifest_snapshots SET canonical_json=?4,canonical_blake3=?5,verified_at=?6 WHERE source_id=?1 AND game_id=?2 AND manifest_blake3=?3",params![record.game.source_id,record.game.game_id,digest,canonical_json,canonical_blake3,Utc::now().to_rfc3339()]).map_err(database_error)?;
        } else {
            tx.execute("INSERT INTO fusion_manifest_snapshots(source_id,game_id,manifest_blake3,raw_bytes,canonical_json,verified_at,canonical_blake3) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![record.game.source_id,record.game.game_id,digest,record.raw_bytes,canonical_json,Utc::now().to_rfc3339(),canonical_blake3]).map_err(database_error)?;
        }
        tx.commit().map_err(database_error)
    }

    pub fn fusion_get_manifest(
        &self,
        game: &QualifiedGameId,
        digest: &str,
    ) -> Result<Option<FusionManifestRecord>, FusionError> {
        validate_game(game)?;
        validate_digest(digest)?;
        let stored:Option<(Vec<u8>,String,String)>=self.conn.query_row("SELECT raw_bytes,canonical_json,canonical_blake3 FROM fusion_manifest_snapshots WHERE source_id=?1 AND game_id=?2 AND manifest_blake3=?3",params![game.source_id,game.game_id,digest.to_ascii_lowercase()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(database_error)?;
        stored
            .map(|(raw_bytes, canonical, canonical_blake3)| {
                if blake3::hash(&raw_bytes).to_hex().as_str() != digest.to_ascii_lowercase() {
                    return Err(FusionError::new(
                        ErrorCode::ManifestDigestMismatch,
                        "The cached source manifest is corrupt.",
                    ));
                }
                if blake3::hash(canonical.as_bytes()).to_hex().as_str() != canonical_blake3 {
                    return Err(FusionError::new(
                        ErrorCode::IntegrityViolation,
                        "The derived cached manifest is corrupt.",
                    ));
                }
                let canonical: CanonicalGameManifest =
                    serde_json::from_str(&canonical).map_err(json_error)?;
                SchemaUpcaster::validate_canonical(&canonical)?;
                if canonical.source_id != game.source_id || canonical.id != game.game_id {
                    return Err(FusionError::new(
                        ErrorCode::InvalidManifest,
                        "The cached manifest identity changed.",
                    ));
                }
                Ok(FusionManifestRecord {
                    game: game.clone(),
                    digest: digest.to_ascii_lowercase(),
                    raw_bytes,
                    canonical,
                })
            })
            .transpose()
    }

    pub fn fusion_get_installation(
        &self,
        game: &QualifiedGameId,
    ) -> Result<Option<FusionInstallationRecord>, FusionError> {
        validate_game(game)?;
        self.conn.query_row("SELECT source_id,game_id,options_json,active_snapshot_id,status,library_root,external_workspace_root,active_version,active_manifest_blake3 FROM fusion_installations WHERE source_id=?1 AND game_id=?2",params![game.source_id,game.game_id],installation_row).optional().map_err(database_error)?.map(decode_installation).transpose()
    }

    pub fn fusion_set_options(
        &self,
        game: &QualifiedGameId,
        options: &GameOptions,
    ) -> Result<(), FusionError> {
        validate_game(game)?;
        let json = serde_json::to_string(options).map_err(json_error)?;
        let tx = self.conn.unchecked_transaction().map_err(database_error)?;
        if active_operation(&tx, game)? {
            return Err(FusionError::new(
                ErrorCode::OperationInProgress,
                "Options cannot change during a game operation.",
            ));
        }
        ensure_installation(&tx, game)?;
        tx.execute("UPDATE fusion_installations SET options_json=?3,updated_at=?4 WHERE source_id=?1 AND game_id=?2",params![game.source_id,game.game_id,json,Utc::now().to_rfc3339()]).map_err(database_error)?;
        tx.commit().map_err(database_error)
    }

    pub fn fusion_begin_operation(
        &self,
        game: &QualifiedGameId,
        kind: &str,
    ) -> Result<FusionOperationRecord, FusionError> {
        validate_game(game)?;
        if !["install", "update", "rollback", "launch", "remove"].contains(&kind) {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "The Fusion operation kind is invalid.",
            ));
        }
        let tx = self.conn.unchecked_transaction().map_err(database_error)?;
        if active_operation(&tx, game)? {
            return Err(FusionError::new(
                ErrorCode::OperationInProgress,
                "Another operation owns this Fusion game.",
            ));
        }
        let record = FusionOperationRecord {
            id: unique_id("operation"),
            game: game.clone(),
            kind: kind.into(),
            phase: "queued".into(),
            sequence: 0,
            status: "queued".into(),
        };
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO fusion_operations(id,source_id,game_id,kind,phase,sequence,status,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,0,'queued',?6,?6)",params![record.id,game.source_id,game.game_id,kind,record.phase,now]).map_err(database_error)?;
        tx.commit().map_err(database_error)?;
        Ok(record)
    }

    pub fn fusion_progress_operation(
        &self,
        id: &str,
        sequence: u64,
        phase: &str,
    ) -> Result<bool, FusionError> {
        let sequence = i64::try_from(sequence).map_err(|_| {
            FusionError::new(
                ErrorCode::InvalidManifest,
                "The operation sequence exceeds SQLite's range.",
            )
        })?;
        if phase.is_empty() || phase.len() > 128 {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "The operation phase is invalid.",
            ));
        }
        self.conn.execute("UPDATE fusion_operations SET sequence=?2,phase=?3,status='running',updated_at=?4 WHERE id=?1 AND sequence<?2 AND status IN ('queued','running')",params![id,sequence,phase,Utc::now().to_rfc3339()]).map(|changed|changed==1).map_err(database_error)
    }

    pub fn fusion_finish_operation(&self, id: &str, status: &str) -> Result<bool, FusionError> {
        if !["completed", "cancelled", "failed", "interrupted"].contains(&status) {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "The final operation status is invalid.",
            ));
        }
        self.conn.execute("UPDATE fusion_operations SET status=?2,phase=?2,sequence=sequence+1,updated_at=?3 WHERE id=?1 AND status IN ('queued','running','cancelling')",params![id,status,Utc::now().to_rfc3339()]).map(|changed|changed==1).map_err(database_error)
    }

    pub fn fusion_interrupt_operations(&self) -> Result<usize, FusionError> {
        self.conn.execute("UPDATE fusion_operations SET status='interrupted',phase='interrupted',sequence=sequence+1,updated_at=?1 WHERE status IN ('queued','running','cancelling')",params![Utc::now().to_rfc3339()]).map_err(database_error)
    }

    pub fn fusion_active_operation(
        &self,
        game: &QualifiedGameId,
    ) -> Result<Option<FusionOperationRecord>, FusionError> {
        validate_game(game)?;
        self.conn.query_row("SELECT id,kind,phase,sequence,status FROM fusion_operations WHERE source_id=?1 AND game_id=?2 AND status IN ('queued','running','cancelling')",params![game.source_id,game.game_id],|row|Ok(FusionOperationRecord{id:row.get(0)?,game:game.clone(),kind:row.get(1)?,phase:row.get(2)?,sequence:row.get(3)?,status:row.get(4)?})).optional().map_err(database_error)
    }
}

fn validate_digest(digest: &str) -> Result<(), FusionError> {
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(FusionError::new(
            ErrorCode::InvalidDigest,
            "A stored BLAKE3 digest must contain 64 hexadecimal characters.",
        ));
    }
    Ok(())
}

fn unique_id(kind: &str) -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let material = format!(
        "{}:{}:{}:{}",
        kind,
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    );
    format!(
        "{}-{}",
        kind,
        &blake3::hash(material.as_bytes()).to_hex()[..24]
    )
}

fn validated_index(index: &CanonicalSourceIndex) -> Result<(), FusionError> {
    let wire = SourceIndex {
        schema_version: index.schema_version.clone(),
        source_info: index.source_info.clone(),
        games: index.games.clone(),
        signature: None,
    };
    let json = serde_json::to_string(&wire).map_err(json_error)?;
    SchemaUpcaster::upcast_source_index(&json).map(|_| ())
}

fn active_operation(conn: &Connection, game: &QualifiedGameId) -> Result<bool, FusionError> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM fusion_operations WHERE source_id = ?1 AND game_id = ?2 AND status IN ('queued','running','cancelling'))", params![game.source_id, game.game_id], |row| row.get(0)).map_err(database_error)
}

fn ensure_installation(tx: &Transaction<'_>, game: &QualifiedGameId) -> Result<(), FusionError> {
    tx.execute("INSERT OR IGNORE INTO fusion_installations(source_id,game_id,options_json,status,updated_at) VALUES (?1,?2,'{}','not_installed',?3)", params![game.source_id,game.game_id,Utc::now().to_rfc3339()]).map_err(database_error)?;
    Ok(())
}

fn reserve_source(
    tx: &Transaction<'_>,
    index: &CanonicalSourceIndex,
    url: &str,
) -> Result<(), FusionError> {
    let existing: Option<(String, String, Option<String>)> = tx
        .query_row(
            "SELECT id,url,public_key FROM fusion_sources WHERE id=?1 COLLATE NOCASE",
            params![index.source_info.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(database_error)?;
    if let Some((old_id, old_url, key)) = existing {
        if key != index.source_info.public_key {
            return Err(FusionError::new(
                ErrorCode::InvalidSourceSignature,
                "The source public key differs from its reserved identity.",
            ));
        }
        if old_id != index.source_info.id || old_url != url {
            return Err(FusionError::new(
                ErrorCode::DuplicateId,
                "The source ID is already reserved with another spelling, URL or public key.",
            )
            .with_detail("sourceId", &index.source_info.id));
        }
    }
    Ok(())
}

fn write_source(
    tx: &Transaction<'_>,
    index: &CanonicalSourceIndex,
    url: &str,
) -> Result<(), FusionError> {
    reserve_source(tx, index, url)?;
    reserve_game_names(tx, index)?;
    let now = Utc::now().to_rfc3339();
    let index_json = serde_json::to_string(index).map_err(json_error)?;
    tx.execute("INSERT INTO fusion_sources(id,url,public_key,index_json,connected,connected_at,refreshed_at,name) VALUES (?1,?2,?3,?4,1,?5,?5,?6) ON CONFLICT(id) DO UPDATE SET index_json=excluded.index_json, name=excluded.name, connected=1, refreshed_at=excluded.refreshed_at",params![index.source_info.id,url,index.source_info.public_key,index_json,now,index.source_info.name]).map_err(database_error)?;
    tx.execute(
        "DELETE FROM fusion_source_entries WHERE source_id=?1",
        params![index.source_info.id],
    )
    .map_err(database_error)?;
    for entry in &index.games {
        let json = serde_json::to_string(entry).map_err(json_error)?;
        tx.execute(
            "INSERT INTO fusion_source_entries(source_id,game_id,entry_json) VALUES (?1,?2,?3)",
            params![index.source_info.id, entry.id, json],
        )
        .map_err(database_error)?;
    }
    Ok(())
}

/// A refreshed index cannot change only the case of a previously owned game.
/// The history is read in one batch, including removed catalog entries whose
/// installations or snapshots still own Windows paths.
fn reserve_game_names(
    tx: &Transaction<'_>,
    index: &CanonicalSourceIndex,
) -> Result<(), FusionError> {
    let mut statement=tx.prepare("SELECT game_id FROM fusion_source_entries WHERE source_id=?1 UNION SELECT game_id FROM fusion_installations WHERE source_id=?1 UNION SELECT game_id FROM fusion_manifest_snapshots WHERE source_id=?1 UNION SELECT game_id FROM fusion_workspace_snapshots WHERE source_id=?1 UNION SELECT game_id FROM fusion_operations WHERE source_id=?1").map_err(database_error)?;
    let historical = statement
        .query_map(params![index.source_info.id], |row| row.get::<_, String>(0))
        .map_err(database_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(database_error)?;
    let mut names = BTreeMap::new();
    for id in historical {
        names.entry(id.to_ascii_lowercase()).or_insert(id);
    }
    for entry in &index.games {
        if names
            .get(&entry.id.to_ascii_lowercase())
            .is_some_and(|old_id| old_id != &entry.id)
        {
            return Err(FusionError::new(
                ErrorCode::DuplicateId,
                "A game ID is already reserved with another case-sensitive spelling.",
            )
            .with_detail("sourceId", &index.source_info.id)
            .with_detail("gameId", &entry.id));
        }
    }
    Ok(())
}

struct StoredInstallation {
    game: QualifiedGameId,
    options: String,
    active_snapshot_id: Option<String>,
    status: String,
    library_root: Option<String>,
    external_workspace_root: Option<String>,
    active_version: Option<String>,
    active_manifest_blake3: Option<String>,
}
fn installation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredInstallation> {
    Ok(StoredInstallation {
        game: QualifiedGameId {
            source_id: row.get(0)?,
            game_id: row.get(1)?,
        },
        options: row.get(2)?,
        active_snapshot_id: row.get(3)?,
        status: row.get(4)?,
        library_root: row.get(5)?,
        external_workspace_root: row.get(6)?,
        active_version: row.get(7)?,
        active_manifest_blake3: row.get(8)?,
    })
}
fn decode_installation(row: StoredInstallation) -> Result<FusionInstallationRecord, FusionError> {
    validate_game(&row.game)?;
    Ok(FusionInstallationRecord {
        game: row.game,
        options: serde_json::from_str(&row.options).map_err(json_error)?,
        active_snapshot_id: row.active_snapshot_id,
        status: row.status,
        library_root: row.library_root,
        external_workspace_root: row.external_workspace_root,
        active_version: row.active_version,
        active_manifest_blake3: row.active_manifest_blake3,
    })
}

impl RepositoryStore {
    pub fn fusion_list_installations(&self) -> Result<Vec<FusionInstallationRecord>, FusionError> {
        let mut statement=self.conn.prepare("SELECT source_id,game_id,options_json,active_snapshot_id,status,library_root,external_workspace_root,active_version,active_manifest_blake3 FROM fusion_installations ORDER BY source_id,game_id").map_err(database_error)?;
        let rows = statement
            .query_map([], installation_row)
            .map_err(database_error)?;
        rows.map(|row| decode_installation(row.map_err(database_error)?))
            .collect()
    }

    pub fn fusion_activate_snapshot(
        &self,
        snapshot: &WorkspaceSnapshot,
        library_root: &Path,
        external_root: Option<&Path>,
    ) -> Result<(), FusionError> {
        validate_game(&snapshot.game)?;
        SchemaUpcaster::validate_canonical(&snapshot.manifest)?;
        validate_digest(&snapshot.canonical_blake3)?;
        let canonical_json = serde_json::to_vec(&snapshot.manifest).map_err(json_error)?;
        if snapshot.game.source_id != snapshot.manifest.source_id
            || snapshot.game.game_id != snapshot.manifest.id
            || snapshot.plan.game != snapshot.game
            || blake3::hash(&canonical_json).to_hex().as_str() != snapshot.canonical_blake3
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "The workspace snapshot's canonical identity is invalid.",
            ));
        }
        if !library_root.is_absolute() || external_root.is_some_and(|root| !root.is_absolute()) {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "Installation roots must be absolute host-approved paths.",
            ));
        }
        if let Some(digest) = &snapshot.source_manifest_blake3 {
            validate_digest(digest)?;
        }
        let snapshot_json = serde_json::to_string(snapshot).map_err(json_error)?;
        let snapshot_blake3 = blake3::hash(snapshot_json.as_bytes()).to_hex().to_string();
        let tx = self.conn.unchecked_transaction().map_err(database_error)?;
        ensure_installation(&tx, &snapshot.game)?;
        let stored: Option<String> = tx
            .query_row(
                "SELECT snapshot_blake3 FROM fusion_workspace_snapshots WHERE id=?1",
                params![snapshot.id],
                |row| row.get(0),
            )
            .optional()
            .map_err(database_error)?;
        if let Some(existing) = stored {
            if existing != snapshot_blake3 {
                return Err(FusionError::new(
                    ErrorCode::SnapshotConflict,
                    "A workspace snapshot ID already identifies different content.",
                ));
            }
        } else {
            tx.execute("INSERT INTO fusion_workspace_snapshots(id,source_id,game_id,manifest_blake3,snapshot_json,snapshot_blake3,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![snapshot.id,snapshot.game.source_id,snapshot.game.game_id,snapshot.source_manifest_blake3,snapshot_json,snapshot_blake3,Utc::now().to_rfc3339()]).map_err(database_error)?;
            for (layer_id, input) in &snapshot.inputs {
                if let LayerInput::Cas { digest, .. } = input {
                    validate_digest(digest)?;
                    tx.execute("INSERT OR IGNORE INTO fusion_cas_objects(digest,size_bytes,ref_count) VALUES (?1,0,0)",params![digest]).map_err(database_error)?;
                    tx.execute("INSERT INTO fusion_layer_refs(snapshot_id,layer_id,cas_digest) VALUES (?1,?2,?3)",params![snapshot.id,layer_id,digest]).map_err(database_error)?;
                    tx.execute(
                        "UPDATE fusion_cas_objects SET ref_count=ref_count+1 WHERE digest=?1",
                        params![digest],
                    )
                    .map_err(database_error)?;
                }
            }
            if let Some(save) = &snapshot.save_backup {
                let json = serde_json::to_string(save).map_err(json_error)?;
                tx.execute("INSERT OR IGNORE INTO fusion_save_snapshots(id,source_id,game_id,snapshot_json,created_at) VALUES (?1,?2,?3,?4,?5)",params![save.id,snapshot.game.source_id,snapshot.game.game_id,json,Utc::now().to_rfc3339()]).map_err(database_error)?;
            }
        }
        tx.execute("UPDATE fusion_installations SET active_snapshot_id=?3,status='ready',library_root=?4,external_workspace_root=?5,active_version=?6,active_manifest_blake3=?7,updated_at=?8 WHERE source_id=?1 AND game_id=?2",params![snapshot.game.source_id,snapshot.game.game_id,snapshot.id,library_root.to_string_lossy(),external_root.map(|root|root.to_string_lossy().into_owned()),snapshot.manifest.version,snapshot.source_manifest_blake3,Utc::now().to_rfc3339()]).map_err(database_error)?;
        tx.commit().map_err(database_error)
    }

    pub fn fusion_get_active_snapshot(
        &self,
        game: &QualifiedGameId,
    ) -> Result<Option<FusionWorkspaceRecord>, FusionError> {
        validate_game(game)?;
        let row=self.conn.query_row("SELECT s.id,s.source_id,s.game_id,s.manifest_blake3,s.snapshot_json,s.snapshot_blake3 FROM fusion_workspace_snapshots s JOIN fusion_installations i ON i.active_snapshot_id=s.id AND i.source_id=s.source_id AND i.game_id=s.game_id WHERE i.source_id=?1 AND i.game_id=?2",params![game.source_id,game.game_id],workspace_row).optional().map_err(database_error)?;
        row.map(decode_workspace).transpose()
    }

    pub fn fusion_list_snapshots(
        &self,
        game: &QualifiedGameId,
    ) -> Result<Vec<FusionWorkspaceRecord>, FusionError> {
        validate_game(game)?;
        let mut statement=self.conn.prepare("SELECT id,source_id,game_id,manifest_blake3,snapshot_json,snapshot_blake3 FROM fusion_workspace_snapshots WHERE source_id=?1 AND game_id=?2 ORDER BY created_at,id").map_err(database_error)?;
        let rows = statement
            .query_map(params![game.source_id, game.game_id], workspace_row)
            .map_err(database_error)?;
        rows.map(|row| decode_workspace(row.map_err(database_error)?))
            .collect()
    }

    pub fn fusion_remove_installation(&self, game: &QualifiedGameId) -> Result<(), FusionError> {
        validate_game(game)?;
        // Ownership history, CAS references and saves survive uninstall. Only a
        // separately verified filesystem cleanup may retire historical records.
        self.conn.execute("UPDATE fusion_installations SET active_snapshot_id=NULL,status='not_installed',active_version=NULL,active_manifest_blake3=NULL,library_root=NULL,external_workspace_root=NULL,updated_at=?3 WHERE source_id=?1 AND game_id=?2",params![game.source_id,game.game_id,Utc::now().to_rfc3339()]).map_err(database_error)?;
        Ok(())
    }

    pub fn fusion_is_trusted(
        &self,
        game: &QualifiedGameId,
        path: &str,
        digest: &str,
    ) -> Result<bool, FusionError> {
        let id = trust_id(game, path, digest)?;
        self.conn.query_row("SELECT EXISTS(SELECT 1 FROM trusted_executables WHERE asset_id=?1 AND local_path=?2 AND sha256=?3)",params![id,path,format!("blake3:{}",digest.to_ascii_lowercase())],|row|row.get(0)).map_err(database_error)
    }

    pub fn fusion_trust_executable(
        &self,
        game: &QualifiedGameId,
        path: &str,
        digest: &str,
    ) -> Result<(), FusionError> {
        let id = trust_id(game, path, digest)?;
        self.conn.execute("INSERT OR REPLACE INTO trusted_executables(asset_id,local_path,sha256,trusted_at) VALUES (?1,?2,?3,?4)",params![id,path,format!("blake3:{}",digest.to_ascii_lowercase()),Utc::now().to_rfc3339()]).map_err(database_error)?;
        Ok(())
    }
}

struct StoredWorkspace {
    id: String,
    game: QualifiedGameId,
    manifest_blake3: Option<String>,
    json: String,
    digest: String,
}
fn workspace_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredWorkspace> {
    Ok(StoredWorkspace {
        id: row.get(0)?,
        game: QualifiedGameId {
            source_id: row.get(1)?,
            game_id: row.get(2)?,
        },
        manifest_blake3: row.get(3)?,
        json: row.get(4)?,
        digest: row.get(5)?,
    })
}
fn decode_workspace(row: StoredWorkspace) -> Result<FusionWorkspaceRecord, FusionError> {
    if blake3::hash(row.json.as_bytes()).to_hex().as_str() != row.digest {
        return Err(FusionError::new(
            ErrorCode::IntegrityViolation,
            "The persisted workspace record is corrupt.",
        ));
    }
    let snapshot: WorkspaceSnapshot = serde_json::from_str(&row.json).map_err(json_error)?;
    if snapshot.id != row.id
        || snapshot.game != row.game
        || snapshot.source_manifest_blake3 != row.manifest_blake3
    {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "A persisted snapshot identity changed.",
        ));
    }
    Ok(FusionWorkspaceRecord {
        id: row.id,
        game: row.game,
        manifest_blake3: row.manifest_blake3.unwrap_or(snapshot.canonical_blake3),
        snapshot_json: row.json,
    })
}
fn trust_id(game: &QualifiedGameId, path: &str, digest: &str) -> Result<String, FusionError> {
    validate_game(game)?;
    validate_digest(digest)?;
    crate::fusion_source::evolution::validate_relative_path(
        path,
        false,
        false,
        "trusted executable",
    )?;
    Ok(format!(
        "fusion-source:{}::{}:blake3:{}:{}",
        game.source_id,
        game.game_id,
        digest.to_ascii_lowercase(),
        blake3::hash(path.as_bytes()).to_hex()
    ))
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredCrashPreview {
    schema_version: String,
    game: QualifiedGameId,
    layer_hashes: BTreeMap<String, String>,
    runtime: Option<StoredRuntimePreview>,
    enabled_addons: Vec<String>,
    exit_code: i32,
    clean_log_tail: String,
    crash_signature_hash: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StoredRuntimePreview {
    channel: crate::fusion_source::schema::RuntimeChannel,
    version: String,
    executable_blake3: String,
}

fn safe_crash_preview(
    json: &str,
    game: &QualifiedGameId,
    signature: &str,
) -> Result<serde_json::Value, FusionError> {
    if json.len() > 1024 * 1024 {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "The stored crash report exceeds its size bound.",
        ));
    }
    let mut preview: StoredCrashPreview = serde_json::from_str(json).map_err(|_| {
        FusionError::new(
            ErrorCode::InvalidManifest,
            "The stored sanitized crash report is invalid.",
        )
    })?;
    if preview.schema_version != "1.0"
        || &preview.game != game
        || preview.crash_signature_hash != signature
        || preview.exit_code == 0
    {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "The sanitized crash report identity is invalid.",
        ));
    }
    validate_game(game)?;
    validate_digest(signature)?;
    if preview.layer_hashes.len() > 10_000 || preview.enabled_addons.len() > 10_000 {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "The crash report inventory is too large.",
        ));
    }
    for (id, hash) in &preview.layer_hashes {
        validate_game(&QualifiedGameId {
            source_id: game.source_id.clone(),
            game_id: id.clone(),
        })?;
        validate_digest(hash)?;
    }
    for id in &preview.enabled_addons {
        validate_game(&QualifiedGameId {
            source_id: game.source_id.clone(),
            game_id: id.clone(),
        })?;
    }
    if let Some(runtime) = &preview.runtime {
        validate_digest(&runtime.executable_blake3)?;
        if runtime.version.is_empty()
            || runtime.version.len() > 128
            || !runtime
                .version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-+".contains(&byte))
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "The sanitized runtime identity is invalid.",
            ));
        }
    }
    // The typed producer is the primary privacy boundary. Read-side validation
    // and another bounded sanitizer prevent malformed/tampered local DB JSON
    // from becoming an arbitrary IPC payload or restoring private log fields.
    let context = TelemetryContext {
        os_username: std::env::var("USERNAME").ok(),
        appdata: std::env::var_os("APPDATA").map(Into::into),
        local_appdata: std::env::var_os("LOCALAPPDATA").map(Into::into),
        user_profile: std::env::var_os("USERPROFILE").map(Into::into),
        ..Default::default()
    };
    let sanitizer = TelemetrySanitizer::new(context, LogSanitizationLimits::default())?;
    preview.clean_log_tail = sanitizer
        .sanitize_logs(&preview.clean_log_tail)
        .as_str()
        .to_owned();
    serde_json::to_value(preview).map_err(json_error)
}

impl RepositoryStore {
    pub fn fusion_put_crash(
        &self,
        report: &SanitizedCrashReport,
    ) -> Result<FusionCrashRecord, FusionError> {
        let json = serde_json::to_string(report).map_err(json_error)?;
        let signature = report.crash_signature_hash();
        let game = report.game();
        let _ = safe_crash_preview(&json, game, signature)?;
        let id = unique_id("crash");
        let now = Utc::now().to_rfc3339();
        self.conn.execute("INSERT INTO fusion_crash_reports(id,source_id,game_id,signature,report_json,created_at,updated_at) VALUES (?1,?2,?3,?4,?5,?6,?6) ON CONFLICT(source_id,game_id,signature) DO UPDATE SET occurrences=occurrences+1,updated_at=excluded.updated_at",params![id,game.source_id,game.game_id,signature,json,now]).map_err(database_error)?;
        let id:String=self.conn.query_row("SELECT id FROM fusion_crash_reports WHERE source_id=?1 AND game_id=?2 AND signature=?3",params![game.source_id,game.game_id,signature],|row|row.get(0)).map_err(database_error)?;
        self.fusion_get_crash(&id)?.ok_or_else(|| {
            FusionError::new(
                ErrorCode::Io,
                "The saved sanitized crash report disappeared.",
            )
        })
    }

    pub fn fusion_get_crash(&self, id: &str) -> Result<Option<FusionCrashRecord>, FusionError> {
        let stored=self.conn.query_row("SELECT source_id,game_id,signature,report_json,status,created_at FROM fusion_crash_reports WHERE id=?1",params![id],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?))).optional().map_err(database_error)?;
        stored
            .map(
                |(source_id, game_id, signature, json, status, created_at)| {
                    let game = QualifiedGameId { source_id, game_id };
                    let report = safe_crash_preview(&json, &game, &signature)?;
                    Ok(FusionCrashRecord {
                        id: id.into(),
                        game,
                        report,
                        status,
                        created_at,
                    })
                },
            )
            .transpose()
    }

    pub fn fusion_list_crashes(
        &self,
        game: &QualifiedGameId,
    ) -> Result<Vec<FusionCrashRecord>, FusionError> {
        validate_game(game)?;
        let mut statement=self.conn.prepare("SELECT id FROM fusion_crash_reports WHERE source_id=?1 AND game_id=?2 AND status='pending' ORDER BY created_at DESC LIMIT 100").map_err(database_error)?;
        let ids = statement
            .query_map(params![game.source_id, game.game_id], |row| {
                row.get::<_, String>(0)
            })
            .map_err(database_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(database_error)?;
        ids.into_iter()
            .map(|id| {
                self.fusion_get_crash(&id)?.ok_or_else(|| {
                    FusionError::new(ErrorCode::Io, "A stored crash report disappeared.")
                })
            })
            .collect()
    }

    pub fn fusion_set_crash_status(&self, id: &str, status: &str) -> Result<(), FusionError> {
        if !["sent", "discarded"].contains(&status) {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "The crash report status is invalid.",
            ));
        }
        self.conn.execute("UPDATE fusion_crash_reports SET status=?2,updated_at=?3 WHERE id=?1 AND status='pending'",params![id,status,Utc::now().to_rfc3339()]).map_err(database_error)?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct SqliteFusionStorage {
    store: Arc<Mutex<RepositoryStore>>,
}
impl SqliteFusionStorage {
    pub fn local_source(
        &self,
        source_id: &str,
    ) -> Result<Option<crate::local_sources::LocalSourceBinding>, FusionError> {
        self.with_store(|store| store.fusion_local_source(source_id))
    }
    pub fn connect_local_source(
        &self,
        index: &CanonicalSourceIndex,
        binding: &crate::local_sources::LocalSourceBinding,
    ) -> Result<FusionSourceRecord, FusionError> {
        self.with_store(|store| store.fusion_connect_local_source(index, binding))
    }
    pub fn new(store: Arc<Mutex<RepositoryStore>>) -> Self {
        Self { store }
    }
    fn with_store<T>(
        &self,
        operation: impl FnOnce(&RepositoryStore) -> Result<T, FusionError>,
    ) -> Result<T, FusionError> {
        let store = self.store.lock().map_err(|_| {
            FusionError::new(ErrorCode::Io, "The Fusion database lock is unavailable.")
        })?;
        operation(&store)
    }
    /// Called once by app setup after a restart, never when opening a service
    /// for another library root. Active filesystem snapshots remain unchanged.
    pub fn interrupt_operations(&self) -> Result<usize, FusionError> {
        self.with_store(RepositoryStore::fusion_interrupt_operations)
    }
}
impl crate::fusion_source::service::FusionStorage for SqliteFusionStorage {
    fn pinned_source_identity(
        &self,
        source_id: &str,
        index_url: &str,
    ) -> Result<Option<crate::fusion_source::signatures::PinnedSourceIdentity>, FusionError> {
        self.with_store(|store| store.fusion_pinned_source_identity(source_id, index_url))
    }
    fn bind_installation_root(
        &self,
        game: &QualifiedGameId,
        library_root: &Path,
        external_root: Option<&Path>,
    ) -> Result<(), FusionError> {
        self.with_store(|store| {
            store.fusion_bind_installation_root(game, library_root, external_root)
        })
    }
    fn add_source(
        &self,
        index: &CanonicalSourceIndex,
        url: &str,
    ) -> Result<FusionSourceRecord, FusionError> {
        self.with_store(|store| store.fusion_add_source(index, url))
    }
    fn refresh_source(
        &self,
        index: &CanonicalSourceIndex,
        url: &str,
    ) -> Result<FusionSourceRecord, FusionError> {
        self.with_store(|store| store.fusion_refresh_source(index, url))
    }
    fn list_sources(&self) -> Result<Vec<FusionSourceRecord>, FusionError> {
        self.with_store(RepositoryStore::fusion_list_sources)
    }
    fn remove_source(&self, id: &str) -> Result<(), FusionError> {
        self.with_store(|store| store.fusion_remove_source(id))
    }
    fn list_catalog(&self) -> Result<Vec<FusionCatalogRecord>, FusionError> {
        self.with_store(RepositoryStore::fusion_list_catalog)
    }
    fn catalog_local_state(&self) -> Result<CatalogLocalState, FusionError> {
        self.with_store(RepositoryStore::fusion_catalog_local_state)
    }
    fn put_manifest(&self, record: &FusionManifestRecord) -> Result<(), FusionError> {
        self.with_store(|store| store.fusion_put_manifest(record))
    }
    fn get_manifest(
        &self,
        game: &QualifiedGameId,
        digest: &str,
    ) -> Result<Option<FusionManifestRecord>, FusionError> {
        self.with_store(|store| store.fusion_get_manifest(game, digest))
    }
    fn get_installation(
        &self,
        game: &QualifiedGameId,
    ) -> Result<Option<FusionInstallationRecord>, FusionError> {
        self.with_store(|store| store.fusion_get_installation(game))
    }
    fn list_installations(&self) -> Result<Vec<FusionInstallationRecord>, FusionError> {
        self.with_store(RepositoryStore::fusion_list_installations)
    }
    fn set_options(
        &self,
        game: &QualifiedGameId,
        options: &GameOptions,
    ) -> Result<(), FusionError> {
        self.with_store(|store| store.fusion_set_options(game, options))
    }
    fn begin_operation(
        &self,
        game: &QualifiedGameId,
        kind: &str,
    ) -> Result<FusionOperationRecord, FusionError> {
        self.with_store(|store| store.fusion_begin_operation(game, kind))
    }
    fn progress_operation(
        &self,
        id: &str,
        sequence: u64,
        phase: &str,
    ) -> Result<bool, FusionError> {
        self.with_store(|store| store.fusion_progress_operation(id, sequence, phase))
    }
    fn finish_operation(&self, id: &str, status: &str) -> Result<bool, FusionError> {
        self.with_store(|store| store.fusion_finish_operation(id, status))
    }
    fn activate_snapshot(
        &self,
        snapshot: &WorkspaceSnapshot,
        library_root: &Path,
        external_root: Option<&Path>,
    ) -> Result<(), FusionError> {
        self.with_store(|store| {
            store.fusion_activate_snapshot(snapshot, library_root, external_root)
        })
    }
    fn get_active_snapshot(
        &self,
        game: &QualifiedGameId,
    ) -> Result<Option<FusionWorkspaceRecord>, FusionError> {
        self.with_store(|store| store.fusion_get_active_snapshot(game))
    }
    fn list_snapshots(
        &self,
        game: &QualifiedGameId,
    ) -> Result<Vec<FusionWorkspaceRecord>, FusionError> {
        self.with_store(|store| store.fusion_list_snapshots(game))
    }
    fn remove_installation(&self, game: &QualifiedGameId) -> Result<(), FusionError> {
        self.with_store(|store| store.fusion_remove_installation(game))
    }
    fn is_trusted(
        &self,
        game: &QualifiedGameId,
        path: &str,
        digest: &str,
    ) -> Result<bool, FusionError> {
        self.with_store(|store| store.fusion_is_trusted(game, path, digest))
    }
    fn trust_executable(
        &self,
        game: &QualifiedGameId,
        path: &str,
        digest: &str,
    ) -> Result<(), FusionError> {
        self.with_store(|store| store.fusion_trust_executable(game, path, digest))
    }
    fn put_crash(&self, report: &SanitizedCrashReport) -> Result<FusionCrashRecord, FusionError> {
        self.with_store(|store| store.fusion_put_crash(report))
    }
    fn get_crash(&self, id: &str) -> Result<Option<FusionCrashRecord>, FusionError> {
        self.with_store(|store| store.fusion_get_crash(id))
    }
    fn list_crashes(&self, game: &QualifiedGameId) -> Result<Vec<FusionCrashRecord>, FusionError> {
        self.with_store(|store| store.fusion_list_crashes(game))
    }
    fn set_crash_status(&self, id: &str, status: &str) -> Result<(), FusionError> {
        self.with_store(|store| store.fusion_set_crash_status(id, status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fusion_source::capabilities::CapabilityRegistry;
    use crate::fusion_source::plan::{PlanBuilder, PlanOptions};
    use crate::fusion_source::telemetry::{CrashOutcome, CrashReportBuilder};
    use serde_json::json;
    use tempfile::tempdir;

    fn game() -> QualifiedGameId {
        QualifiedGameId {
            source_id: "test-source".into(),
            game_id: "native-game".into(),
        }
    }
    fn raw_manifest() -> String {
        json!({"schema_version":"1.0","id":"native-game","title":"Storage fixture","version":"1.0","layers":[{"id":"base","kind":"base","source":{"kind":"user_provided"},"critical_files":[{"path":"main.exe","blake3":"a".repeat(64),"role":"main_executable"}]}],"launch":{"id":"main","executable":"main.exe","role":"primary"}}).to_string()
    }
    fn index() -> CanonicalSourceIndex {
        SchemaUpcaster::upcast_source_index(&json!({"schema_version":"1.0","source_info":{"id":"test-source","name":"Storage source"},"games":[{"id":"native-game","title":"Storage fixture","version":"1.0","manifest_url":"https://example.com/game.json","manifest_blake3":blake3::hash(raw_manifest().as_bytes()).to_hex().to_string()}]}).to_string()).unwrap()
    }
    fn snapshot(id: &str) -> WorkspaceSnapshot {
        let raw = raw_manifest();
        let manifest =
            SchemaUpcaster::upcast_game_manifest("test-source", &raw, &BTreeMap::new()).unwrap();
        let plan = PlanBuilder::build(
            &manifest,
            &PlanOptions::default(),
            &CapabilityRegistry::current_client().with("proc.supervisor.v1"),
        )
        .unwrap();
        let canonical_blake3 = blake3::hash(&serde_json::to_vec(&manifest).unwrap())
            .to_hex()
            .to_string();
        serde_json::from_value(json!({"id":id,"game":game(),"workspaceRoot":std::env::temp_dir().join("fusion-storage-fixture").join(id).join("workspace"),"manifest":manifest,"canonicalBlake3":canonical_blake3,"sourceManifestBlake3":blake3::hash(raw.as_bytes()).to_hex().to_string(),"inputs":{"base":{"kind":"cas","digest":"b".repeat(64),"runtime_lock":null}},"plan":plan,"persistence":{"bindings":[],"envOverrides":{}},"saveBackup":null,"patches":[],"delta":{"added":["base"],"changed":[],"removed":[],"reused":[]},"containerIdentity":{"volume":1,"index":1},"workspaceIdentity":{"volume":1,"index":2},"files":{}})).unwrap()
    }
    fn make_v3(path: &Path) {
        let store = RepositoryStore::open(path).unwrap();
        store.conn.execute("INSERT INTO trusted_executables(asset_id,local_path,sha256,trusted_at) VALUES ('legacy-emu','legacy.exe',?1,'old')",params!["c".repeat(64)]).unwrap();
        store.conn.execute("INSERT INTO app_config(key,value,updated_at) VALUES ('sentinel','preserved','old')",[]).unwrap();
        store.conn.execute_batch("PRAGMA foreign_keys=OFF; DROP TABLE creator_private_bindings; DROP TABLE creator_program_projects; DROP TABLE fusion_local_sources; DROP TABLE fusion_layer_refs; DROP TABLE fusion_workspace_snapshots; DROP TABLE fusion_installations; DROP TABLE fusion_cas_objects; DROP TABLE fusion_manifest_snapshots; DROP TABLE fusion_source_entries; DROP TABLE fusion_sources; DROP TABLE fusion_operations; DROP TABLE fusion_save_snapshots; DROP TABLE fusion_crash_reports; PRAGMA user_version=3;").unwrap();
    }

    #[test]
    fn v3_migration_preserves_legacy_and_creates_only_empty_fusion_tables() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        make_v3(&path);
        let store = RepositoryStore::open(&path).unwrap();
        assert_eq!(
            store
                .conn
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            6
        );
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT value FROM app_config WHERE key='sentinel'",
                    [],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "preserved"
        );
        assert_eq!(
            store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM trusted_executables WHERE asset_id='legacy-emu'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert!(store.fusion_list_sources().unwrap().is_empty());
        assert!(store.fusion_list_installations().unwrap().is_empty());
        for table in ["creator_program_projects", "creator_private_bindings"] {
            let query = format!("SELECT count(*) FROM {table}");
            let count: i64 = store.conn.query_row(&query, [], |row| row.get(0)).unwrap();
            assert_eq!(count, 0, "migration must not fabricate Creator records");
        }
    }

    #[test]
    fn failed_v4_migration_rolls_back_every_new_table_and_version() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("db.sqlite");
        make_v3(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE fusion_source_entries (broken TEXT);")
                .unwrap();
        }
        assert!(RepositoryStore::open(&path).is_err());
        let conn = Connection::open(&path).unwrap();
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            3
        );
        assert_eq!(
            conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='fusion_sources'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            conn.query_row(
                "SELECT value FROM app_config WHERE key='sentinel'",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            "preserved"
        );
    }

    #[test]
    fn add_refresh_are_atomic_metadata_only_and_source_identity_cannot_change() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("db.sqlite")).unwrap();
        let original = index();
        store
            .fusion_add_source(&original, "https://example.com/index.json")
            .unwrap();
        assert_eq!(store.fusion_list_catalog().unwrap().len(), 1);
        assert!(store.fusion_list_installations().unwrap().is_empty());
        assert_eq!(
            store
                .fusion_add_source(&original, "https://other.example/index.json")
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        let mut changed = original.clone();
        changed.source_info.public_key =
            Some("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into());
        assert_eq!(
            store
                .fusion_refresh_source(&changed, "https://example.com/index.json")
                .unwrap_err()
                .code,
            ErrorCode::InvalidSourceSignature
        );
        let mut changed = original.clone();
        changed.games[0].version = "2.0".into();
        store.conn.execute_batch("CREATE TRIGGER fail_catalog_refresh BEFORE INSERT ON fusion_source_entries BEGIN SELECT RAISE(ABORT,'injected refresh failure'); END;").unwrap();
        assert!(store
            .fusion_refresh_source(&changed, "https://example.com/index.json")
            .is_err());
        assert_eq!(store.fusion_list_catalog().unwrap()[0].entry.version, "1.0");
        store
            .conn
            .execute_batch("DROP TRIGGER fail_catalog_refresh;")
            .unwrap();
        store.fusion_remove_source("test-source").unwrap();
        assert!(store.fusion_list_catalog().unwrap().is_empty());
        assert_eq!(
            store
                .fusion_add_source(&original, "https://other.example/index.json")
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        store
            .fusion_add_source(&original, "https://example.com/index.json")
            .unwrap();
    }

    #[test]
    fn source_ids_reserve_windows_case_namespace_even_after_disconnect() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("db.sqlite")).unwrap();
        let mut first = index();
        first.source_info.id = "Foo".into();
        store
            .fusion_add_source(&first, "https://example.com/index.json")
            .unwrap();
        let mut alias = first.clone();
        alias.source_info.id = "foo".into();
        assert_eq!(
            store
                .fusion_add_source(&alias, "https://example.com/index.json")
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        assert_eq!(
            store.fusion_list_sources().unwrap()[0].index.source_info.id,
            "Foo"
        );
        store.fusion_remove_source("Foo").unwrap();
        assert_eq!(
            store
                .fusion_add_source(&alias, "https://example.com/index.json")
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        store
            .fusion_add_source(&first, "https://example.com/index.json")
            .unwrap();
    }

    #[test]
    fn refresh_cannot_recase_existing_or_historical_game_ids() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("db.sqlite")).unwrap();
        let mut first = index();
        first.games[0].id = "Game".into();
        store
            .fusion_add_source(&first, "https://example.com/index.json")
            .unwrap();
        let mut alias = first.clone();
        alias.games[0].id = "game".into();
        assert_eq!(
            store
                .fusion_refresh_source(&alias, "https://example.com/index.json")
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        assert_eq!(store.fusion_list_catalog().unwrap()[0].entry.id, "Game");
        let owned = QualifiedGameId {
            source_id: first.source_info.id.clone(),
            game_id: "Game".into(),
        };
        store
            .fusion_set_options(&owned, &GameOptions::default())
            .unwrap();
        let mut empty = first.clone();
        empty.games.clear();
        store
            .fusion_refresh_source(&empty, "https://example.com/index.json")
            .unwrap();
        assert_eq!(
            store
                .fusion_refresh_source(&alias, "https://example.com/index.json")
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
        assert!(store.fusion_list_catalog().unwrap().is_empty());
        store.fusion_remove_source(&first.source_info.id).unwrap();
        assert_eq!(
            store
                .fusion_add_source(&alias, "https://example.com/index.json")
                .unwrap_err()
                .code,
            ErrorCode::DuplicateId
        );
    }

    #[test]
    fn raw_manifest_bytes_are_reverified_and_immutable() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("db.sqlite")).unwrap();
        let raw_bytes = raw_manifest().into_bytes();
        let canonical = SchemaUpcaster::upcast_game_manifest(
            "test-source",
            std::str::from_utf8(&raw_bytes).unwrap(),
            &BTreeMap::new(),
        )
        .unwrap();
        let record = FusionManifestRecord {
            game: game(),
            digest: blake3::hash(&raw_bytes).to_hex().to_string(),
            raw_bytes,
            canonical,
        };
        store.fusion_put_manifest(&record).unwrap();
        store.fusion_put_manifest(&record).unwrap();
        let mut changed = record.clone();
        changed.raw_bytes.push(b' ');
        assert_eq!(
            store.fusion_put_manifest(&changed).unwrap_err().code,
            ErrorCode::ManifestDigestMismatch
        );
        store
            .conn
            .execute(
                "UPDATE fusion_manifest_snapshots SET canonical_json=?1",
                params!["{}"],
            )
            .unwrap();
        assert_eq!(
            store
                .fusion_get_manifest(&game(), &record.digest)
                .unwrap_err()
                .code,
            ErrorCode::IntegrityViolation
        );
        store.fusion_put_manifest(&record).unwrap();
        store
            .conn
            .execute(
                "UPDATE fusion_manifest_snapshots SET raw_bytes=?1",
                params![b"corrupted".to_vec()],
            )
            .unwrap();
        assert_eq!(
            store
                .fusion_get_manifest(&game(), &record.digest)
                .unwrap_err()
                .code,
            ErrorCode::ManifestDigestMismatch
        );
    }

    #[test]
    fn activation_refs_and_active_pointer_commit_together_and_uninstall_keeps_history() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("db.sqlite")).unwrap();
        store
            .fusion_add_source(&index(), "https://example.com/index.json")
            .unwrap();
        let first = snapshot("snapshot-first");
        store
            .fusion_activate_snapshot(&first, dir.path(), None)
            .unwrap();
        store
            .fusion_activate_snapshot(&first, dir.path(), None)
            .unwrap();
        assert_eq!(
            store
                .conn
                .query_row("SELECT ref_count FROM fusion_cas_objects", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            store.fusion_remove_source("test-source").unwrap_err().code,
            ErrorCode::SnapshotConflict
        );
        store.conn.execute_batch("CREATE TRIGGER fail_layer_ref BEFORE INSERT ON fusion_layer_refs BEGIN SELECT RAISE(ABORT,'injected reference failure'); END;").unwrap();
        assert!(store
            .fusion_activate_snapshot(&snapshot("snapshot-second"), dir.path(), None)
            .is_err());
        assert_eq!(
            store
                .fusion_get_active_snapshot(&game())
                .unwrap()
                .unwrap()
                .id,
            first.id
        );
        assert_eq!(store.fusion_list_snapshots(&game()).unwrap().len(), 1);
        assert_eq!(
            store
                .conn
                .query_row("SELECT ref_count FROM fusion_cas_objects", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        store.fusion_remove_installation(&game()).unwrap();
        assert!(store.fusion_get_active_snapshot(&game()).unwrap().is_none());
        assert_eq!(store.fusion_list_snapshots(&game()).unwrap().len(), 1);
        store.fusion_remove_source("test-source").unwrap();
        assert_eq!(store.fusion_list_snapshots(&game()).unwrap().len(), 1);
    }

    #[test]
    fn operation_ids_sequences_and_restart_recovery_reject_stale_writes() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("db.sqlite")).unwrap();
        let old = store.fusion_begin_operation(&game(), "install").unwrap();
        assert_eq!(
            store
                .fusion_begin_operation(&game(), "update")
                .unwrap_err()
                .code,
            ErrorCode::OperationInProgress
        );
        assert!(store
            .fusion_progress_operation(&old.id, 2, "fetch")
            .unwrap());
        assert!(!store
            .fusion_progress_operation(&old.id, 1, "stale")
            .unwrap());
        assert_eq!(
            store.fusion_remove_source("test-source").unwrap_err().code,
            ErrorCode::OperationInProgress
        );
        assert_eq!(store.fusion_interrupt_operations().unwrap(), 1);
        let new = store.fusion_begin_operation(&game(), "update").unwrap();
        assert_ne!(new.id, old.id);
        assert!(!store
            .fusion_progress_operation(&old.id, 99, "stale")
            .unwrap());
        assert!(!store.fusion_finish_operation(&old.id, "completed").unwrap());
        assert_eq!(
            store.fusion_active_operation(&game()).unwrap().unwrap().id,
            new.id
        );
    }

    #[test]
    fn exact_hash_trust_is_namespaced_by_game_path_algorithm_and_hash() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("db.sqlite")).unwrap();
        store
            .fusion_trust_executable(&game(), "main.exe", &"a".repeat(64))
            .unwrap();
        assert!(store
            .fusion_is_trusted(&game(), "main.exe", &"a".repeat(64))
            .unwrap());
        assert!(!store
            .fusion_is_trusted(&game(), "main.exe", &"b".repeat(64))
            .unwrap());
        assert!(!store
            .fusion_is_trusted(&game(), "other.exe", &"a".repeat(64))
            .unwrap());
        let mut other = game();
        other.source_id = "legacy".into();
        assert!(!store
            .fusion_is_trusted(&other, "main.exe", &"a".repeat(64))
            .unwrap());
        assert_eq!(
            store
                .fusion_trust_executable(&game(), "../escape.exe", &"a".repeat(64))
                .unwrap_err()
                .code,
            ErrorCode::InvalidPath
        );
        assert_eq!(
            store
                .conn
                .query_row("SELECT sha256 FROM trusted_executables", [], |row| row
                    .get::<_, String>(
                    0
                ))
                .unwrap(),
            format!("blake3:{}", "a".repeat(64))
        );
    }

    #[test]
    fn crash_storage_accepts_only_sanitized_values_deduplicates_and_validates_reads() {
        let dir = tempdir().unwrap();
        let store = RepositoryStore::open(&dir.path().join("db.sqlite")).unwrap();
        let snapshot = snapshot("crash-snapshot");
        let sanitizer = TelemetrySanitizer::new(
            TelemetryContext {
                os_username: Some("SecretUser".into()),
                ..Default::default()
            },
            LogSanitizationLimits::default(),
        )
        .unwrap();
        let report=CrashReportBuilder::build(&snapshot,CrashOutcome{primary_exit_code:1,user_cancelled:false},r"crash C:\Users\SecretUser\private\payload.exe token=supersecret user@example.com 192.168.1.42",&sanitizer).unwrap().unwrap();
        let first = store.fusion_put_crash(&report).unwrap();
        let second = store.fusion_put_crash(&report).unwrap();
        assert_eq!(first.id, second.id);
        let json = serde_json::to_string(&first.report).unwrap();
        assert!(!json.contains("SecretUser"));
        assert!(!json.contains("supersecret"));
        assert!(!json.contains("user@example.com"));
        assert!(!json.contains("192.168.1.42"));
        assert_eq!(
            store
                .conn
                .query_row("SELECT occurrences FROM fusion_crash_reports", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            2
        );
        assert_eq!(store.fusion_list_crashes(&game()).unwrap().len(), 1);
        store
            .fusion_set_crash_status(&first.id, "discarded")
            .unwrap();
        assert!(store.fusion_list_crashes(&game()).unwrap().is_empty());
        store
            .conn
            .execute(
                "UPDATE fusion_crash_reports SET report_json=?1",
                params!["{\"rawLog\":\"private path\"}"],
            )
            .unwrap();
        assert!(store.fusion_get_crash(&first.id).is_err());
    }
}
