//! Private save generations. The caller owns the inactive candidate and stops all
//! writers under its workspace lease before capture. Binary rollback never picks
//! an older generation. Failed captures leave only unreferenced staging data.
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety::{self, io_error, DirectoryGuard};
use crate::fusion_source::program_schema::{ProgramPersistence, ProgramPersistenceKind};

pub struct ProgramPersistenceStore {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Node {
    directory: bool,
    bytes: u64,
    hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedEntry {
    declaration: ProgramPersistence,
    present: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    version: u32,
    entries: Vec<RecordedEntry>,
    nodes: BTreeMap<String, Node>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Latest {
    version: u32,
    generation: String,
    inventory_blake3: String,
}

impl ProgramPersistenceStore {
    pub fn open(root: &Path) -> Result<Self, FusionError> {
        let _root = fs_safety::ensure_directory(root)?;
        let _generations = fs_safety::ensure_directory(&root.join("generations"))?;
        Ok(Self {
            root: root.to_owned(),
        })
    }

    pub fn rehydrate(
        &self,
        candidate: &Path,
        entries: &[ProgramPersistence],
    ) -> Result<(), FusionError> {
        validate_entries(entries)?;
        self.check_separate(candidate)?;
        let _root = fs_safety::guard(&self.root)?;
        let _lock = self.lock()?;
        let _candidate = fs_safety::guard(candidate)?;
        // Validate every declared target, including missing-path ancestors, before
        // replacing any leaf. Keep directory guards until validation completes.
        let (_, guards) = inspect(candidate, entries)?;
        drop(guards);
        let pointer = self.root.join("latest.json");
        if metadata(&pointer)?.is_none() {
            return Ok(());
        }
        let latest: Latest = read_json(&pointer)?;
        fs_safety::validate_relative_path(&latest.generation)?;
        if latest.version != 1
            || latest.generation.contains('/')
            || !latest.generation.starts_with("generation-")
        {
            return Err(corrupt("Invalid save generation pointer."));
        }
        let generation = self.root.join("generations").join(&latest.generation);
        let _generation = fs_safety::guard(&generation)?;
        let bytes = read_bytes(&generation.join("inventory.json"))?;
        if blake3::hash(&bytes).to_hex().as_str() != latest.inventory_blake3 {
            return Err(corrupt("Save inventory hash mismatch."));
        }
        let inventory: Inventory = serde_json::from_slice(&bytes)
            .map_err(|e| io_error("Decode save inventory", &generation, e))?;
        let recorded: Vec<_> = inventory
            .entries
            .iter()
            .map(|e| e.declaration.clone())
            .collect();
        validate_entries(&recorded)?;
        let _saved_guards = verify_inventory(&generation.join("data"), &inventory)?;
        // Reject descriptor kind changes before touching any candidate path.
        for entry in entries {
            if inventory.entries.iter().any(|old| {
                old.declaration.path.eq_ignore_ascii_case(&entry.path) && old.declaration != *entry
            }) {
                return Err(invalid(
                    "Persistence declaration changed for a recorded save.",
                ));
            }
        }
        for entry in entries {
            let Some(saved) = inventory
                .entries
                .iter()
                .find(|old| old.declaration == *entry)
            else {
                continue;
            };
            let target = candidate.join(&entry.path);
            if let Some(m) = metadata(&target)? {
                if m.is_dir() {
                    fs_safety::safe_remove_tree(&target)?;
                } else {
                    fs_safety::unlink_file(&target)?;
                }
            }
            if saved.present {
                copy_entry(&generation.join("data"), candidate, entry, &inventory.nodes)?;
            }
        }
        Ok(())
    }

    pub fn capture(
        &self,
        candidate: &Path,
        entries: &[ProgramPersistence],
    ) -> Result<(), FusionError> {
        validate_entries(entries)?;
        self.check_separate(candidate)?;
        let _root = fs_safety::guard(&self.root)?;
        let _lock = self.lock()?;
        let _candidate = fs_safety::guard(candidate)?;
        let (inventory, _source_guards) = inspect(candidate, entries)?;
        let generations = self.root.join("generations");
        let _generations = fs_safety::guard(&generations)?;
        let id = fs_safety::unique_name("generation");
        let staging = generations.join(format!(".incomplete-{id}"));
        let data = staging.join("data");
        let _ = fs_safety::ensure_directory(&data)?;
        for recorded in &inventory.entries {
            if recorded.present {
                copy_entry(candidate, &data, &recorded.declaration, &inventory.nodes)?;
            }
        }
        // Re-read physical copies, including empty directories and tombstones,
        // before the single commit point. Never mutate a published generation.
        let guards = verify_inventory(&data, &inventory)?;
        drop(guards);
        let bytes = serde_json::to_vec(&inventory)
            .map_err(|e| io_error("Encode save inventory", &staging, e))?;
        fs_safety::atomic_write(&staging.join("inventory.json"), &bytes)?;
        if read_bytes(&staging.join("inventory.json"))? != bytes {
            return Err(corrupt("Save metadata failed read-back verification."));
        }
        let published = generations.join(&id);
        fs::rename(&staging, &published)
            .map_err(|e| io_error("Publish save generation", &published, e))?;
        let latest = Latest {
            version: 1,
            generation: id,
            inventory_blake3: blake3::hash(&bytes).to_hex().to_string(),
        };
        let pointer = self.root.join("latest.json");
        fs_safety::atomic_write_json(&pointer, &latest)?;
        if read_json::<Latest>(&pointer)? != latest {
            return Err(corrupt("Save pointer failed read-back verification."));
        }
        Ok(())
    }

    fn check_separate(&self, candidate: &Path) -> Result<(), FusionError> {
        // Resolve Windows extended/short-name aliases only after rejecting every
        // reparse ancestor. String-only checks can mistake the store for a private
        // candidate when the same directory has a different path spelling.
        let _root = fs_safety::guard(&self.root)?;
        let _candidate = fs_safety::guard(candidate)?;
        let root = fs::canonicalize(&self.root)
            .map_err(|e| io_error("Resolve save store", &self.root, e))?
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase();
        let candidate = fs::canonicalize(candidate)
            .map_err(|e| io_error("Resolve save candidate", candidate, e))?
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_lowercase();
        if root == candidate
            || root.starts_with(&(candidate.clone() + "/"))
            || candidate.starts_with(&(root + "/"))
        {
            return Err(invalid("Save store and candidate must be disjoint."));
        }
        Ok(())
    }

    fn lock(&self) -> Result<File, FusionError> {
        let path = self.root.join(".lock");
        if metadata(&path)?.is_some_and(|m| !m.is_file()) {
            return Err(invalid("Save lock must be a file."));
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3).custom_flags(0x00200000);
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(0x20000);
        }
        let file = options
            .open(&path)
            .map_err(|e| io_error("Open save lock", &path, e))?;
        let m = file
            .metadata()
            .map_err(|e| io_error("Inspect save lock", &path, e))?;
        fs_safety::check_metadata(&m, &path)?;
        if !m.is_file() || fs_safety::link_count(&file)? != 1 {
            return Err(invalid("Save lock must be private."));
        }
        file.lock_exclusive()
            .map_err(|e| io_error("Lock saves", &path, e))?;
        Ok(file)
    }
}

fn invalid(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidPath, message)
}
fn corrupt(message: &str) -> FusionError {
    FusionError::new(ErrorCode::IntegrityViolation, message)
}

fn validate_entries(entries: &[ProgramPersistence]) -> Result<(), FusionError> {
    if entries.len() > 64 {
        return Err(invalid("Too many persistence declarations."));
    }
    let mut parents = BTreeMap::<String, String>::new();
    for (i, entry) in entries.iter().enumerate() {
        fs_safety::validate_relative_path(&entry.path)?;
        let key = entry.path.to_lowercase();
        for prior in &entries[..i] {
            let old = prior.path.to_lowercase();
            if key == old
                || key.starts_with(&(old.clone() + "/"))
                || old.starts_with(&(key.clone() + "/"))
            {
                return Err(invalid("Persistence paths overlap or collide."));
            }
        }
        for end in entry
            .path
            .match_indices('/')
            .map(|(i, _)| i)
            .chain(std::iter::once(entry.path.len()))
        {
            let spelling = &entry.path[..end];
            if parents
                .insert(spelling.to_lowercase(), spelling.into())
                .is_some_and(|old| old != spelling)
            {
                return Err(invalid("Persistence ancestor casing conflicts."));
            }
        }
    }
    Ok(())
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>, FusionError> {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            fs_safety::check_metadata(&m, path)?;
            Ok(Some(m))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io_error("Inspect save path", path, e)),
    }
}

fn inspect(
    root: &Path,
    entries: &[ProgramPersistence],
) -> Result<(Inventory, Vec<DirectoryGuard>), FusionError> {
    let mut guards = vec![fs_safety::guard(root)?];
    let mut nodes = BTreeMap::new();
    let mut recorded = Vec::new();
    for entry in entries {
        let mut parent = root.to_owned();
        for component in entry
            .path
            .split('/')
            .take(entry.path.split('/').count() - 1)
        {
            parent.push(component);
            if let Some(m) = metadata(&parent)? {
                if !m.is_dir() {
                    return Err(invalid("Save ancestor is not a directory."));
                }
                guards.push(fs_safety::guard(&parent)?);
            } else {
                break;
            }
        }
        let path = root.join(&entry.path);
        let m = metadata(&path)?;
        if let Some(m) = &m {
            if m.is_dir() != (entry.kind == ProgramPersistenceKind::Directory) {
                return Err(invalid("Persistence path kind mismatch."));
            }
            scan(root, &path, &mut nodes, &mut guards)?;
        }
        recorded.push(RecordedEntry {
            declaration: entry.clone(),
            present: m.is_some(),
        });
    }
    Ok((
        Inventory {
            version: 1,
            entries: recorded,
            nodes,
        },
        guards,
    ))
}

fn scan(
    root: &Path,
    path: &Path,
    nodes: &mut BTreeMap<String, Node>,
    guards: &mut Vec<DirectoryGuard>,
) -> Result<(), FusionError> {
    let m = metadata(path)?.ok_or_else(|| corrupt("Save disappeared during inspection."))?;
    let relative = path
        .strip_prefix(root)
        .map_err(|e| io_error("Resolve save path", path, e))?;
    let relative = relative
        .to_str()
        .ok_or_else(|| invalid("Save paths must be Unicode."))?
        .replace('\\', "/");
    fs_safety::validate_relative_path(&relative)?;
    if nodes
        .keys()
        .any(|key| key.to_lowercase() == relative.to_lowercase())
    {
        return Err(invalid("Case-colliding save paths."));
    }
    let node = if m.is_dir() {
        guards.push(fs_safety::guard(path)?);
        Node {
            directory: true,
            bytes: 0,
            hash: None,
        }
    } else {
        let mut file = fs_safety::open_read(path)?;
        let mut hash = blake3::Hasher::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let n = file
                .read(&mut buffer)
                .map_err(|e| io_error("Hash save", path, e))?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
            bytes = bytes
                .checked_add(n as u64)
                .ok_or_else(|| corrupt("Save size overflow."))?;
        }
        Node {
            directory: false,
            bytes,
            hash: Some(hash.finalize().to_hex().to_string()),
        }
    };
    nodes.insert(relative, node);
    if m.is_dir() {
        for entry in fs::read_dir(path).map_err(|e| io_error("List saves", path, e))? {
            scan(
                root,
                &entry
                    .map_err(|e| io_error("Read save entry", path, e))?
                    .path(),
                nodes,
                guards,
            )?;
        }
    }
    Ok(())
}

fn copy_entry(
    source: &Path,
    destination: &Path,
    entry: &ProgramPersistence,
    nodes: &BTreeMap<String, Node>,
) -> Result<(), FusionError> {
    for (relative, node) in nodes {
        if relative != &entry.path && !relative.starts_with(&(entry.path.clone() + "/")) {
            continue;
        }
        let target = destination.join(relative);
        if node.directory {
            let _ = fs_safety::ensure_directory(&target)?;
        } else {
            fs_safety::copy_file(&source.join(relative), &target)?;
        }
    }
    Ok(())
}

fn verify_inventory(
    root: &Path,
    inventory: &Inventory,
) -> Result<Vec<DirectoryGuard>, FusionError> {
    let entries: Vec<_> = inventory
        .entries
        .iter()
        .map(|e| e.declaration.clone())
        .collect();
    let (actual, mut guards) = inspect(root, &entries)?;
    if inventory.version != 1 || actual != *inventory {
        return Err(corrupt("Save generation inventory mismatch."));
    }
    let mut expected = inventory.nodes.clone();
    for relative in inventory.nodes.keys() {
        for (end, _) in relative.match_indices('/') {
            expected.entry(relative[..end].into()).or_insert(Node {
                directory: true,
                bytes: 0,
                hash: None,
            });
        }
    }
    let mut all = BTreeMap::new();
    for entry in fs::read_dir(root).map_err(|e| io_error("List generation", root, e))? {
        scan(
            root,
            &entry
                .map_err(|e| io_error("Read generation", root, e))?
                .path(),
            &mut all,
            &mut guards,
        )?;
    }
    if all != expected {
        return Err(corrupt("Save generation contains unrecorded paths."));
    }
    for (relative, node) in &all {
        if !node.directory
            && fs_safety::link_count(&fs_safety::open_read(&root.join(relative))?)? != 1
        {
            return Err(corrupt("Save generation contains a shared physical file."));
        }
    }
    Ok(guards)
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, FusionError> {
    let mut file = fs_safety::open_read(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| io_error("Read save metadata", path, e))?;
    Ok(bytes)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, FusionError> {
    serde_json::from_slice(&read_bytes(path)?)
        .map_err(|e| io_error("Decode save metadata", path, e))
}
#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
