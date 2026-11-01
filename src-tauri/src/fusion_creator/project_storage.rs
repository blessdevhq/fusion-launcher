//! Typed CreatorProgramProject 1.1 storage using the single A carrier/validator.
//! SQLite is authoritative: there is no portable-file/DB dual write and no IPC wiring.
#![allow(dead_code)]

use super::CreatorPrivateBindings;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::program_schema::{
    CreatorProgramProject, MAX_PROGRAM_BYTES as MAX_BYTES, MAX_REVISION,
};
use crate::storage::RepositoryStore;
use rusqlite::{params, OptionalExtension, TransactionBehavior};

pub(crate) fn save_project_bytes(
    store: &mut RepositoryStore,
    bytes: &[u8],
    expected_revision: Option<u64>,
) -> Result<CreatorProgramProject, FusionError> {
    save_project_with_bindings(store, bytes, expected_revision, None)
}

pub(crate) struct PrivateBindingsSave<'a> {
    pub revision: u64,
    pub expected_revision: Option<u64>,
    pub bindings: &'a CreatorPrivateBindings,
}

#[derive(Debug)]
pub(crate) struct SavedPrivateBindings {
    pub revision: u64,
    pub bindings: CreatorPrivateBindings,
}

pub(crate) fn save_project_with_bindings(
    store: &mut RepositoryStore,
    bytes: &[u8],
    expected_revision: Option<u64>,
    bindings: Option<&PrivateBindingsSave<'_>>,
) -> Result<CreatorProgramProject, FusionError> {
    let json = std::str::from_utf8(bytes).map_err(|_| {
        FusionError::new(
            ErrorCode::InvalidManifest,
            "Creator project must be UTF-8 JSON.",
        )
    })?;
    let project = CreatorProgramProject::read_json(json)?;
    let private_bytes = bindings
        .map(|update| {
            if update.bindings.project_id != project.project_id
                || update.bindings.component_paths.keys().any(|id| {
                    !project
                        .program_descriptor
                        .components
                        .iter()
                        .any(|component| &component.id == id)
                })
            {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Private bindings do not match the project components.",
                ));
            }
            let bytes = serde_json::to_vec(update.bindings).map_err(|_| {
                FusionError::new(ErrorCode::InvalidManifest, "Invalid private bindings.")
            })?;
            check_payload(update.revision, &bytes)?;
            Ok(bytes)
        })
        .transpose()?;
    let private = bindings
        .zip(private_bytes.as_deref())
        .map(|(update, bytes)| PrivateBindingUpdate {
            revision: update.revision,
            expected_revision: update.expected_revision,
            bytes,
        });
    store_verified(
        store,
        &VerifiedDocument {
            project_id: &project.project_id,
            revision: project.revision,
            bytes,
        },
        expected_revision,
        private.as_ref(),
    )?;
    Ok(project)
}

fn invalid_stored() -> FusionError {
    FusionError::new(
        ErrorCode::IntegrityViolation,
        "Stored Creator metadata is inconsistent.",
    )
}

fn validate_lookup_id(project_id: &str) -> Result<(), FusionError> {
    if !(1..=128).contains(&project_id.chars().count()) {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Invalid Creator project identifier.",
        ));
    }
    Ok(())
}

pub(crate) fn open_project(
    store: &RepositoryStore,
    project_id: &str,
) -> Result<Option<CreatorProgramProject>, FusionError> {
    validate_lookup_id(project_id)?;
    load_document(store, project_id)?
        .map(|row| {
            let json = std::str::from_utf8(&row.bytes).map_err(|_| invalid_stored())?;
            let project = CreatorProgramProject::read_json(json).map_err(|_| invalid_stored())?;
            if project.project_id != project_id || project.revision != row.revision {
                return Err(invalid_stored());
            }
            Ok(project)
        })
        .transpose()
}

/// Explicit bounded pages; never joins or exports private binding rows.
pub(crate) fn list_projects(
    store: &RepositoryStore,
    limit: u32,
    offset: u32,
) -> Result<Vec<CreatorProgramProject>, FusionError> {
    if !(1..=32).contains(&limit) {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Creator page size must be between 1 and 32.",
        ));
    }
    let ids = {
        let mut statement = store.conn.prepare(
            "SELECT project_id FROM creator_program_projects ORDER BY project_id LIMIT ?1 OFFSET ?2",
        ).map_err(sql_error)?;
        let rows = statement
            .query_map(params![limit, offset], |row| row.get::<_, String>(0))
            .map_err(sql_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)?
    };
    ids.into_iter()
        .map(|id| open_project(store, &id)?.ok_or_else(invalid_stored))
        .collect()
}

pub(crate) fn open_private_bindings(
    store: &RepositoryStore,
    project_id: &str,
) -> Result<Option<SavedPrivateBindings>, FusionError> {
    validate_lookup_id(project_id)?;
    load_private_bindings(store, project_id)?
        .map(|row| {
            let json = std::str::from_utf8(&row.bytes).map_err(|_| invalid_stored())?;
            let value = crate::fusion_source::program_schema::parse_unique_json(json, MAX_BYTES)
                .map_err(|_| invalid_stored())?;
            let bindings: CreatorPrivateBindings =
                serde_json::from_value(value).map_err(|_| invalid_stored())?;
            if bindings.project_id != project_id {
                return Err(invalid_stored());
            }
            Ok(SavedPrivateBindings {
                revision: row.revision,
                bindings,
            })
        })
        .transpose()
}

// These private byte envelopes are storage inputs, not duplicate descriptor DTOs.
// The production constructor above is reachable only after A's strict reader.
struct VerifiedDocument<'a> {
    project_id: &'a str,
    revision: u64,
    bytes: &'a [u8],
}

struct PrivateBindingUpdate<'a> {
    revision: u64,
    expected_revision: Option<u64>,
    bytes: &'a [u8],
}

#[derive(Debug, PartialEq, Eq)]
struct StoredBytes {
    revision: u64,
    bytes: Vec<u8>,
}

fn sql_error(_: rusqlite::Error) -> FusionError {
    // Never disclose SQL payloads, paths, trigger messages or private values.
    FusionError::new(ErrorCode::Io, "Creator storage operation failed.")
}

fn conflict() -> FusionError {
    FusionError::new(ErrorCode::SnapshotConflict, "Creator revision conflict.")
}

fn check_payload(revision: u64, bytes: &[u8]) -> Result<(), FusionError> {
    if revision > MAX_REVISION || bytes.len() > MAX_BYTES {
        return Err(FusionError::new(
            ErrorCode::InvalidManifest,
            "Creator storage revision or document size is out of bounds.",
        ));
    }
    Ok(())
}

// None means insert-only (not revision zero). New documents may start at any
// frozen valid revision, including zero. Updates must advance exactly once.
fn check_revision(
    actual: Option<u64>,
    expected: Option<u64>,
    next: u64,
) -> Result<(), FusionError> {
    if actual != expected || expected.is_some_and(|old| old >= MAX_REVISION || next != old + 1) {
        return Err(conflict());
    }
    Ok(())
}

fn store_verified(
    store: &mut RepositoryStore,
    document: &VerifiedDocument<'_>,
    expected_revision: Option<u64>,
    bindings: Option<&PrivateBindingUpdate<'_>>,
) -> Result<(), FusionError> {
    check_payload(document.revision, document.bytes)?;
    if let Some(bindings) = bindings {
        check_payload(bindings.revision, bindings.bytes)?;
    }
    let digest = blake3::hash(document.bytes).to_hex().to_string();
    // Acquire the writer lock before reading either expected revision.
    let tx = store
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sql_error)?;
    let actual: Option<u64> = tx
        .query_row(
            "SELECT revision FROM creator_program_projects WHERE project_id=?1",
            [document.project_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    check_revision(actual, expected_revision, document.revision)?;
    if let Some(bindings) = bindings {
        let actual_private: Option<u64> = tx
            .query_row(
                "SELECT revision FROM creator_private_bindings WHERE project_id=?1",
                [document.project_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        check_revision(
            actual_private,
            bindings.expected_revision,
            bindings.revision,
        )?;
    }
    // Parameterized keys never become filesystem paths or SQL syntax.
    tx.execute(
        "INSERT INTO creator_program_projects(project_id,revision,document,document_blake3)
         VALUES(?1,?2,?3,?4) ON CONFLICT(project_id) DO UPDATE SET
         revision=excluded.revision,document=excluded.document,document_blake3=excluded.document_blake3",
        params![document.project_id, document.revision, document.bytes, digest],
    )
    .map_err(sql_error)?;
    if let Some(bindings) = bindings {
        tx.execute(
            "INSERT INTO creator_private_bindings(project_id,revision,bindings_json)
             VALUES(?1,?2,?3) ON CONFLICT(project_id) DO UPDATE SET
             revision=excluded.revision,bindings_json=excluded.bindings_json",
            params![document.project_id, bindings.revision, bindings.bytes],
        )
        .map_err(sql_error)?;
    }
    // Any early error drops/rolls back the transaction, including the first write
    // if the private write fails. Report success only after COMMIT succeeds.
    tx.commit().map_err(sql_error)
}

fn load_document(
    store: &RepositoryStore,
    project_id: &str,
) -> Result<Option<StoredBytes>, FusionError> {
    let row: Option<(u64, Vec<u8>, String)> = store
        .conn
        .query_row(
            "SELECT revision,document,document_blake3 FROM creator_program_projects WHERE project_id=?1",
            [project_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(sql_error)?;
    row.map(|(revision, bytes, digest)| {
        check_payload(revision, &bytes)?;
        if blake3::hash(&bytes).to_hex().as_str() != digest {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "Creator stored document digest mismatch.",
            ));
        }
        Ok(StoredBytes { revision, bytes })
    })
    .transpose()
}

// Separate explicit private read: portable reads never join/select bindings.
fn load_private_bindings(
    store: &RepositoryStore,
    project_id: &str,
) -> Result<Option<StoredBytes>, FusionError> {
    let row: Option<StoredBytes> = store
        .conn
        .query_row(
            "SELECT revision,bindings_json FROM creator_private_bindings WHERE project_id=?1",
            [project_id],
            |r| {
                Ok(StoredBytes {
                    revision: r.get(0)?,
                    bytes: r.get(1)?,
                })
            },
        )
        .optional()
        .map_err(sql_error)?;
    if let Some(row) = &row {
        check_payload(row.revision, &row.bytes)?;
    }
    Ok(row)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod typed_tests;
