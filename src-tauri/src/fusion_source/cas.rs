//! Streaming, immutable content-addressed layer storage.
//!
//! A CAS identity is the BLAKE3 of a sorted file inventory (path, byte length,
//! file BLAKE3). It is deliberately independent of a downloaded archive digest.
//! Only private completed staging directories are published. Source permissions
//! are never changed, including when importing a local storefront installation.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::errors::{ErrorCode, FusionError};
use super::fs_safety::{self, io_error};
use super::schema::{DigestAlgorithm, DigestSpec};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CasFile {
    pub path: String,
    pub blake3: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CasObject {
    pub digest: String,
    pub root: PathBuf,
    pub files: Vec<CasFile>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    version: u32,
    digest: String,
    files: Vec<CasFile>,
}

#[derive(Debug, Clone)]
pub struct CasStore {
    root: PathBuf,
}

impl CasStore {
    pub fn open(library_root: &Path) -> Result<Self, FusionError> {
        let root = library_root.join(".fusion-source").join("cas");
        for child in ["objects", "staging", "locks"] {
            fs_safety::ensure_directory(&root.join(child))?;
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn import_directory(&self, source: &Path) -> Result<CasObject, FusionError> {
        let _source = fs_safety::guard(source)?;
        let _store = fs_safety::guard(&self.root)?;
        let canonical_source = source
            .canonicalize()
            .map_err(|error| io_error("Resolve import source", source, error))?;
        let canonical_store = self
            .root
            .canonicalize()
            .map_err(|error| io_error("Resolve CAS store", &self.root, error))?;
        if path_contains(&canonical_source, &canonical_store) {
            return Err(FusionError::new(
                ErrorCode::InvalidPath,
                "An import source must not contain the CAS store.",
            ));
        }
        let inputs = fs_safety::list_plain_files(source)?;
        self.import(
            inputs
                .iter()
                .map(|(relative, path)| (relative.as_str(), path.as_path())),
        )
    }

    pub fn import_file(&self, source: &Path, relative: &str) -> Result<CasObject, FusionError> {
        fs_safety::validate_relative_path(relative)?;
        self.import(std::iter::once((relative, source)))
    }

    fn import<'a>(
        &self,
        inputs: impl Iterator<Item = (&'a str, &'a Path)>,
    ) -> Result<CasObject, FusionError> {
        let _store = fs_safety::guard(&self.root)?;
        let stage = self
            .root
            .join("staging")
            .join(fs_safety::unique_name("object"));
        // create_dir, rather than create_dir_all, establishes ownership; never
        // adopt a pre-existing directory, even one containing a valid marker.
        let _staging_parent = fs_safety::guard(&self.root.join("staging"))?;
        fs::create_dir(&stage)
            .map_err(|error| io_error("Create private CAS staging", &stage, error))?;
        let mut owned_stage = OwnedStage(Some(stage.clone()));
        {
            let _stage = fs_safety::guard(&stage)?;
            let data = stage.join("data");
            let _data = fs_safety::ensure_directory(&data)?;
            for (relative, input) in inputs {
                let relative = fs_safety::validate_relative_path(relative)?;
                fs_safety::copy_file(input, &data.join(relative))?;
            }
            // All copy writers have closed before bytes and lengths are read.
            let files = inventory_directory(&data)?;
            let digest = inventory_digest(&files)?;
            let object = self.object_path(&digest)?;
            let _object_lock = self.lock(&digest)?;
            if object
                .try_exists()
                .map_err(|error| io_error("Inspect CAS object", &object, error))?
            {
                let existing = self.verify(&digest)?;
                return Ok(existing);
            }
            for file in &files {
                let path = data.join(fs_safety::validate_relative_path(&file.path)?);
                let handle = fs_safety::open_read(&path)?;
                if fs_safety::link_count(&handle)? != 1 {
                    return Err(FusionError::new(
                        ErrorCode::IntegrityViolation,
                        "A private CAS staging file gained an external hardlink.",
                    ));
                }
                let mut permissions = handle
                    .metadata()
                    .map_err(|error| io_error("Inspect CAS permissions", &path, error))?
                    .permissions();
                permissions.set_readonly(true);
                fs::set_permissions(&path, permissions)
                    .map_err(|error| io_error("Protect CAS file", &path, error))?;
            }
            fs_safety::atomic_write_json(
                &stage.join("inventory.json"),
                &Inventory {
                    version: 1,
                    digest: digest.clone(),
                    files: files.clone(),
                },
            )?;
            let inventory_path = stage.join("inventory.json");
            let inventory_handle = fs_safety::open_read(&inventory_path)?;
            if fs_safety::link_count(&inventory_handle)? != 1 {
                return Err(FusionError::new(
                    ErrorCode::IntegrityViolation,
                    "Private CAS inventory gained an external hardlink.",
                ));
            }
            let mut permissions = inventory_handle
                .metadata()
                .map_err(|error| io_error("Inspect inventory", &inventory_path, error))?
                .permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&inventory_path, permissions)
                .map_err(|error| io_error("Protect inventory", &inventory_path, error))?;
            drop(inventory_handle);
            // Read back the immutable staged data and metadata before publication.
            verify_directory(&data, &files)?;
            // Directory handles cannot deny DELETE while this directory is
            // renamed. The unique private stage and object lock remain owned.
        }
        let inventory: Inventory =
            serde_json::from_reader(fs_safety::open_read(&stage.join("inventory.json"))?)
                .map_err(|error| io_error("Read completed inventory", &stage, error))?;
        // Retain the per-object lock across the final no-replace publication too.
        let _object_lock = self.lock(&inventory.digest)?;
        let destination = self.object_path(&inventory.digest)?;
        if destination
            .try_exists()
            .map_err(|error| io_error("Inspect publication target", &destination, error))?
        {
            return self.verify(&inventory.digest);
        }
        let _parent = fs_safety::guard(&self.root.join("objects"))?;
        fs_safety::publish_directory(&stage, &destination)?;
        owned_stage.0 = None;
        self.verify(&inventory.digest)
    }

    fn object_path(&self, digest: &str) -> Result<PathBuf, FusionError> {
        validate_hash(digest)?;
        Ok(self.root.join("objects").join(digest))
    }

    /// Validates the inventory's version, address, sorted paths and digests, then
    /// checks the actual plain-file set, lengths and read-only attributes. This
    /// ordinary cache lookup reads metadata, not payload bytes: same-length byte
    /// corruption requires `verify` or a critical-file integrity check to detect.
    /// The returned root is derived from the store, never persisted absolute data.
    pub fn get(&self, digest: &str) -> Result<CasObject, FusionError> {
        let object = self.object_path(digest)?;
        let _object = fs_safety::guard(&object)?;
        let manifest = object.join("inventory.json");
        let file = fs_safety::open_read(&manifest)?;
        if !file
            .metadata()
            .map_err(|error| io_error("Inspect CAS inventory", &manifest, error))?
            .permissions()
            .readonly()
        {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "CAS inventory lost read-only protection.",
            ));
        }
        let inventory: Inventory = serde_json::from_reader(file)
            .map_err(|error| io_error("Parse CAS inventory", &manifest, error))?;
        if inventory.version != 1
            || inventory.digest != digest
            || inventory_digest(&inventory.files)? != digest
        {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "CAS inventory identity does not match its address.",
            ));
        }
        let root = object.join("data");
        verify_structure(&root, &inventory.files)?;
        Ok(CasObject {
            digest: digest.to_owned(),
            root,
            files: inventory.files,
        })
    }

    /// Performs `get`'s structural validation plus an explicit streaming BLAKE3
    /// audit of every payload file. Publication and import reuse always take this
    /// full audit path; ordinary layer-cache access can use the cheaper `get`.
    pub fn verify(&self, digest: &str) -> Result<CasObject, FusionError> {
        let object = self.get(digest)?;
        let _root = fs_safety::guard(&object.root)?;
        verify_directory(&object.root, &object.files)?;
        Ok(object)
    }

    fn lock(&self, digest: &str) -> Result<File, FusionError> {
        validate_hash(digest)?;
        let parent = self.root.join("locks");
        let _parent = fs_safety::guard(&parent)?;
        let path = parent.join(format!("{digest}.lock"));
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            fs_safety::check_metadata(&metadata, &path)?;
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
        let lock = options
            .open(&path)
            .map_err(|error| io_error("Open CAS publication lock", &path, error))?;
        fs_safety::check_metadata(
            &lock
                .metadata()
                .map_err(|error| io_error("Inspect CAS lock", &path, error))?,
            &path,
        )?;
        fs2::FileExt::lock_exclusive(&lock)
            .map_err(|error| io_error("Lock CAS object", &path, error))?;
        Ok(lock)
    }
}

struct OwnedStage(Option<PathBuf>);

impl Drop for OwnedStage {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            // Failure preserves an orphan for a future explicitly owned cleanup;
            // it never justifies following a junction or weakening permissions.
            let _ = fs_safety::safe_remove_tree(&path);
        }
    }
}

fn path_contains(parent: &Path, child: &Path) -> bool {
    #[cfg(windows)]
    {
        child.to_string_lossy().to_lowercase().starts_with(
            &(parent
                .to_string_lossy()
                .to_lowercase()
                .trim_end_matches('\\')
                .to_owned()
                + "\\"),
        ) || parent
            .to_string_lossy()
            .eq_ignore_ascii_case(&child.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        child.starts_with(parent)
    }
}

fn inventory_directory(root: &Path) -> Result<Vec<CasFile>, FusionError> {
    fs_safety::list_plain_files(root)?
        .into_iter()
        .map(|(path, absolute)| {
            let mut file = fs_safety::open_read(&absolute)?;
            let size = file
                .metadata()
                .map_err(|error| io_error("Inspect inventory file", &absolute, error))?
                .len();
            let blake3 = hash_reader(&mut file, DigestAlgorithm::Blake3)
                .map_err(|error| io_error("Hash inventory file", &absolute, error))?;
            Ok(CasFile { path, blake3, size })
        })
        .collect()
}

fn inventory_digest(files: &[CasFile]) -> Result<String, FusionError> {
    let mut previous: Option<&str> = None;
    let mut case_paths = std::collections::BTreeSet::new();
    for file in files {
        fs_safety::validate_relative_path(&file.path)?;
        validate_hash(&file.blake3)?;
        if previous.is_some_and(|previous| previous >= file.path.as_str())
            || !case_paths.insert(file.path.to_lowercase())
        {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "CAS inventory paths must be unique and sorted.",
            ));
        }
        previous = Some(&file.path);
    }
    let bytes = serde_json::to_vec(files).map_err(|error| {
        FusionError::new(
            ErrorCode::IntegrityViolation,
            format!("Cannot encode CAS inventory: {error}"),
        )
    })?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"fusion-source-cas-tree-v1\0");
    hasher.update(&bytes);
    Ok(hasher.finalize().to_hex().to_string())
}

fn verify_directory(root: &Path, expected: &[CasFile]) -> Result<(), FusionError> {
    let actual = inventory_directory(root)?;
    if actual != expected {
        return Err(FusionError::new(
            ErrorCode::IntegrityViolation,
            "CAS bytes or file inventory changed.",
        ));
    }
    Ok(())
}

fn verify_structure(root: &Path, expected: &[CasFile]) -> Result<(), FusionError> {
    let actual = fs_safety::list_plain_files(root)?;
    if actual.len() != expected.len() {
        return Err(FusionError::new(
            ErrorCode::IntegrityViolation,
            "CAS file set changed.",
        ));
    }
    for ((relative, absolute), expected) in actual.iter().zip(expected) {
        if relative != &expected.path {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "CAS file set changed.",
            ));
        }
        let file = fs_safety::open_read(absolute)?;
        let metadata = file
            .metadata()
            .map_err(|error| io_error("Inspect immutable CAS file", absolute, error))?;
        if metadata.len() != expected.size || !metadata.permissions().readonly() {
            return Err(FusionError::new(
                ErrorCode::IntegrityViolation,
                "CAS file length or read-only protection changed.",
            )
            .with_detail("file", relative));
        }
    }
    Ok(())
}

pub(crate) fn validate_hash(value: &str) -> Result<(), FusionError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(FusionError::new(
            ErrorCode::InvalidDigest,
            "Expected a lowercase 256-bit hexadecimal digest.",
        ));
    }
    Ok(())
}

fn hash_reader(file: &mut impl Read, algorithm: DigestAlgorithm) -> std::io::Result<String> {
    let mut buffer = [0u8; 128 * 1024];
    match algorithm {
        DigestAlgorithm::Blake3 => {
            let mut hasher = blake3::Hasher::new();
            loop {
                let size = file.read(&mut buffer)?;
                if size == 0 {
                    break;
                }
                hasher.update(&buffer[..size]);
            }
            Ok(hasher.finalize().to_hex().to_string())
        }
        DigestAlgorithm::Sha256 => {
            let mut hasher = Sha256::new();
            loop {
                let size = file.read(&mut buffer)?;
                if size == 0 {
                    break;
                }
                hasher.update(&buffer[..size]);
            }
            Ok(hex::encode(hasher.finalize()))
        }
    }
}

pub fn hash_file(path: &Path, algorithm: DigestAlgorithm) -> Result<String, FusionError> {
    let _parent =
        fs_safety::guard(path.parent().ok_or_else(|| {
            FusionError::new(ErrorCode::InvalidPath, "Hash input has no parent.")
        })?)?;
    let mut file = fs_safety::open_read(path)?;
    hash_reader(&mut file, algorithm).map_err(|error| io_error("Hash file", path, error))
}

pub fn verify_digest(path: &Path, expected: &DigestSpec) -> Result<(), FusionError> {
    validate_hash(&expected.value)?;
    let actual = hash_file(path, expected.algorithm)?;
    if actual != expected.value {
        return Err(FusionError::new(
            ErrorCode::IntegrityViolation,
            "File digest does not match the declared digest.",
        )
        .with_detail("expected", &expected.value)
        .with_detail("actual", actual));
    }
    Ok(())
}

/// Hashes the retained descriptor, needed when publication must use exactly the
/// bytes verified before a rename rather than reopening a mutable pathname.
pub fn verify_open_digest(file: &mut File, expected: &DigestSpec) -> Result<(), FusionError> {
    validate_hash(&expected.value)?;
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        FusionError::new(
            ErrorCode::IntegrityViolation,
            format!("Seek verified file: {error}"),
        )
    })?;
    let actual = hash_reader(file, expected.algorithm).map_err(|error| {
        FusionError::new(
            ErrorCode::IntegrityViolation,
            format!("Hash verified file: {error}"),
        )
    })?;
    if actual != expected.value {
        return Err(FusionError::new(
            ErrorCode::IntegrityViolation,
            "Retained file digest mismatch.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_streaming_digest_vectors_and_mismatch() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("abc");
        fs::write(&file, b"abc").unwrap();
        assert_eq!(
            hash_file(&file, DigestAlgorithm::Blake3).unwrap(),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
        assert_eq!(
            hash_file(&file, DigestAlgorithm::Sha256).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(verify_digest(
            &file,
            &DigestSpec {
                algorithm: DigestAlgorithm::Blake3,
                value: "0".repeat(64)
            }
        )
        .is_err());
        assert!(verify_digest(
            &file,
            &DigestSpec {
                algorithm: DigestAlgorithm::Sha256,
                value: "../escape".into()
            }
        )
        .is_err());
        fs::write(&file, vec![b'a'; 1_000_000]).unwrap();
        assert_eq!(
            hash_file(&file, DigestAlgorithm::Sha256).unwrap(),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn published_object_can_be_linked_without_losing_integrity_or_readonly() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source-file");
        fs::write(&source, b"game payload").unwrap();
        let store = CasStore::open(&directory.path().join("library")).unwrap();
        let object = store.import_file(&source, "game.bin").unwrap();
        let workspace_file = directory.path().join("linked-game.bin");
        fs::hard_link(object.root.join("game.bin"), &workspace_file).unwrap();
        assert_eq!(store.verify(&object.digest).unwrap(), object);
        fs_safety::unlink_file(&workspace_file).unwrap();
        assert!(object
            .root
            .join("game.bin")
            .metadata()
            .unwrap()
            .permissions()
            .readonly());
        assert_eq!(store.verify(&object.digest).unwrap(), object);
        fs_safety::safe_remove_tree(&directory.path().join("library")).unwrap();
    }

    #[test]
    fn imports_reuse_immutable_objects_and_never_chmod_original() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        fs::create_dir_all(source.join("sub")).unwrap();
        fs::write(source.join("z"), b"last").unwrap();
        fs::write(source.join("sub/a"), b"first").unwrap();
        let store = CasStore::open(&directory.path().join("library")).unwrap();
        let first = store.import_directory(&source).unwrap();
        let second = store.import_directory(&source).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.files[0].path, "sub/a");
        assert!(first
            .root
            .join("z")
            .metadata()
            .unwrap()
            .permissions()
            .readonly());
        assert!(!source
            .join("z")
            .metadata()
            .unwrap()
            .permissions()
            .readonly());
        assert_ne!(
            fs_safety::file_identity(&source.join("z")).unwrap(),
            fs_safety::file_identity(&first.root.join("z")).unwrap()
        );
        assert_eq!(fs::read_dir(store.root.join("staging")).unwrap().count(), 0);
        assert_eq!(store.verify(&first.digest).unwrap(), first);
        fs_safety::safe_remove_tree(&directory.path().join("library")).unwrap();
    }

    #[test]
    fn tree_identity_includes_paths_and_handles_empty_layers() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        fs::create_dir(&source).unwrap();
        let store = CasStore::open(&directory.path().join("library")).unwrap();
        let empty = store.import_directory(&source).unwrap();
        assert!(empty.files.is_empty());
        fs::write(source.join("original"), b"same").unwrap();
        let first = store.import_directory(&source).unwrap();
        fs::rename(source.join("original"), source.join("renamed")).unwrap();
        let second = store.import_directory(&source).unwrap();
        assert_ne!(first.digest, second.digest);
        assert_ne!(empty.digest, second.digest);
        assert!(store.get("../objects").is_err());
        assert!(store
            .import_file(&source.join("renamed"), "../escape")
            .is_err());
        fs_safety::safe_remove_tree(&directory.path().join("library")).unwrap();
    }

    #[test]
    fn rejects_source_containing_store_and_detects_unlisted_files() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), b"data").unwrap();
        let store = CasStore::open(&directory.path().join("library")).unwrap();
        assert!(store.import_directory(directory.path()).is_err());
        let object = store.import_directory(&source).unwrap();
        fs::write(object.root.join("unexpected"), b"mutation").unwrap();
        assert!(store.verify(&object.digest).is_err());
        fs_safety::safe_remove_tree(&directory.path().join("library")).unwrap();
    }

    #[test]
    fn lookup_checks_structure_and_attributes_while_audit_detects_same_size_corruption() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        fs::write(&source, b"good").unwrap();
        let library = directory.path().join("library");
        let store = CasStore::open(&library).unwrap();
        let object = store
            .import_file(&source, "assets/noncritical.dat")
            .unwrap();
        let payload = object.root.join("assets/noncritical.dat");
        assert_eq!(store.get(&object.digest).unwrap(), object);

        // Replacing an owned fixture file avoids ever clearing a shared inode's
        // read-only bit. Attribute loss is rejected without reading its content.
        fs_safety::atomic_write(&payload, b"evil").unwrap();
        assert_eq!(
            store.get(&object.digest).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        let protect = |path: &Path| {
            let mut permissions = path.metadata().unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(path, permissions).unwrap();
        };
        protect(&payload);
        // Both bytestrings have the same length. A cheap lookup deliberately
        // succeeds, whereas an explicit streaming content audit must reject it.
        assert_eq!(store.get(&object.digest).unwrap(), object);
        assert_eq!(
            store.verify(&object.digest).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );

        fs_safety::atomic_write(&payload, b"longer corruption").unwrap();
        protect(&payload);
        assert_eq!(
            store.get(&object.digest).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        fs_safety::atomic_write(&payload, b"good").unwrap();
        protect(&payload);
        assert_eq!(store.verify(&object.digest).unwrap(), object);

        let unlisted = object.root.join("unlisted.dat");
        fs::write(&unlisted, b"unexpected").unwrap();
        assert_eq!(
            store.get(&object.digest).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        fs_safety::unlink_file(&unlisted).unwrap();
        fs_safety::unlink_file(&payload).unwrap();
        assert_eq!(
            store.get(&object.digest).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        fs_safety::safe_remove_tree(&library).unwrap();
    }

    #[test]
    fn lookup_rejects_invalid_inventory_version_address_and_order() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("a"), b"first").unwrap();
        fs::write(source.join("b"), b"second").unwrap();
        let library = directory.path().join("library");
        let store = CasStore::open(&library).unwrap();
        let object = store.import_directory(&source).unwrap();
        let manifest = object.root.parent().unwrap().join("inventory.json");
        let write = |inventory: &Inventory| {
            fs_safety::atomic_write_json(&manifest, inventory).unwrap();
            let mut permissions = manifest.metadata().unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(&manifest, permissions).unwrap();
        };
        let mut inventory = Inventory {
            version: 9,
            digest: object.digest.clone(),
            files: object.files.clone(),
        };
        write(&inventory);
        assert_eq!(
            store.get(&object.digest).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        inventory.version = 1;
        inventory.digest = "0".repeat(64);
        write(&inventory);
        assert_eq!(
            store.get(&object.digest).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        inventory.digest = object.digest.clone();
        inventory.files.reverse();
        write(&inventory);
        assert_eq!(
            store.get(&object.digest).unwrap_err().code,
            ErrorCode::IntegrityViolation
        );
        inventory.files = object.files.clone();
        write(&inventory);
        assert_eq!(store.get(&object.digest).unwrap(), object);
        fs_safety::safe_remove_tree(&library).unwrap();
    }

    #[test]
    fn concurrent_publishers_reuse_one_complete_object() {
        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("file"), vec![5u8; 400_000]).unwrap();
        let store = CasStore::open(&directory.path().join("library")).unwrap();
        let handles = (0..4)
            .map(|_| {
                let store = store.clone();
                let source = source.clone();
                std::thread::spawn(move || store.import_directory(&source).unwrap())
            })
            .collect::<Vec<_>>();
        let objects = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert!(objects.iter().all(|object| object == &objects[0]));
        assert_eq!(fs::read_dir(store.root.join("objects")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(store.root.join("staging")).unwrap().count(), 0);
        fs_safety::safe_remove_tree(&directory.path().join("library")).unwrap();
    }
}
