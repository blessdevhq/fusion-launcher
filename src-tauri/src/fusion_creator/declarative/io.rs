//! Generic component normalization and private candidate IO.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use crate::fusion_source::cas::{CasObject, CasStore};
use crate::fusion_source::creator_schema::ComponentOrigin;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety::{self, io_error, DirectoryGuard};
use crate::fusion_source::program_schema::ProgramComponent;

#[cfg(test)]
#[path = "io_tests.rs"]
mod tests;

pub fn normalize_zip(
    cas: &CasStore,
    archive: &Path,
    staging_parent: &Path,
    component: &ProgramComponent,
) -> Result<CasObject, FusionError> {
    use crate::fusion_source::schema::{ArchiveFormat, DigestAlgorithm, DigestSpec};
    use std::io::{Seek, SeekFrom};
    let _parent = fs_safety::guard(archive.parent().ok_or_else(|| invalid("Archive has no parent."))?)?;
    let mut input = fs_safety::open_read(archive)?;
    if let ComponentOrigin::External { transport_blake3, .. } = &component.origin {
        crate::fusion_source::cas::verify_open_digest(&mut input, &DigestSpec {
            algorithm: DigestAlgorithm::Blake3, value: transport_blake3.clone(),
        })?;
    }
    input.seek(SeekFrom::Start(0)).map_err(|e| io_error("Rewind verified ZIP", archive, e))?;
    let mut zip = zip::ZipArchive::new(input).map_err(|_| invalid("Invalid ZIP archive."))?;
    if zip.len() > 100_000 { return Err(invalid("Too many ZIP entries.")); }
    let mut paths = PathTree::default();
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(|_| invalid("Invalid ZIP member."))?;
        if entry.is_symlink() || entry.unix_mode().is_some_and(|mode| {
            let kind=mode & 0o170000;
            kind!=0 && kind!=0o100000 && kind!=0o040000
        }) { return Err(invalid("Archive links and special files are forbidden.")); }
        let name = if entry.is_dir() { entry.name().strip_suffix('/').unwrap_or(entry.name()) } else { entry.name() };
        paths.add(name, entry.is_dir())?;
    }
    let _staging_parent = fs_safety::ensure_directory(staging_parent)?;
    let staging = staging_parent.join(fs_safety::unique_name("program-extract"));
    fs::create_dir(&staging).map_err(|e| io_error("Create private ZIP staging", &staging, e))?;
    let result = (|| {
        crate::fusion_source::transforms::ArchiveTransformer::default().extract(archive, ArchiveFormat::Zip, &staging)?;
        normalize_directory(cas, &staging, component)
    })();
    // The ZipArchive retains its deny-write descriptor through extraction/import.
    drop(zip);
    fs_safety::safe_remove_tree(&staging)?;
    result
}

/// Validate the entire input, including unselected siblings and empty directories,
/// before selecting the exact descriptor root. CAS performs the streaming copy.
pub fn normalize_directory(
    cas: &CasStore,
    input: &Path,
    component: &ProgramComponent,
) -> Result<CasObject, FusionError> {
    crate::fusion_source::evolution::validate_relative_path(
        &component.source_root,
        false,
        true,
        "component.source_root",
    )?;
    let tree = inspect_tree(input)?;
    if !component.source_root.is_empty()
        && tree.paths.paths.get(&component.source_root.to_lowercase())
            != Some(&(component.source_root.clone(), true))
    {
        return Err(invalid(
            "The exact component source_root directory does not exist.",
        ));
    }
    let expected = match &component.origin {
        ComponentOrigin::External {
            inventory_blake3, ..
        }
        | ComponentOrigin::UserProvided {
            inventory_blake3, ..
        } => Some(inventory_blake3),
        // Managed artifact_ref is an opaque reference, not a tree digest.
        ComponentOrigin::Managed { .. } => None,
    };
    if let Some(expected) = expected {
        crate::fusion_source::cas::validate_hash(expected)?;
    }
    let object = cas.import_directory(&input.join(&component.source_root))?;
    if expected.is_some_and(|expected| *expected != object.digest) {
        return Err(FusionError::new(
            ErrorCode::IntegrityViolation,
            "Normalized inventory differs from the pinned origin inventory.",
        ));
    }
    Ok(object)
}

fn invalid(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidPath, message)
}

/// Includes implicit parents so directory casing cannot depend on extraction order.
#[derive(Default)]
struct PathTree {
    paths: BTreeMap<String, (String, bool)>,
    explicit: BTreeSet<String>,
}
impl PathTree {
    fn add(&mut self, path: &str, directory: bool) -> Result<(), FusionError> {
        fs_safety::validate_relative_path(path)?;
        let key = path.to_lowercase();
        if !self.explicit.insert(key) {
            return Err(invalid("Duplicate or case-colliding input paths."));
        }
        for end in path
            .match_indices('/')
            .map(|(i, _)| i)
            .chain(std::iter::once(path.len()))
        {
            let spelling = &path[..end];
            let is_directory = end != path.len() || directory;
            let key = spelling.to_lowercase();
            if let Some((previous, previous_directory)) = self.paths.get(&key) {
                if previous != spelling || *previous_directory != is_directory {
                    return Err(invalid(
                        "Input contains path casing or file/parent conflicts.",
                    ));
                }
            } else {
                self.paths.insert(key, (spelling.to_owned(), is_directory));
            }
        }
        Ok(())
    }
}

struct InspectedTree {
    paths: PathTree,
    _guards: Vec<DirectoryGuard>,
}
fn inspect_tree(root: &Path) -> Result<InspectedTree, FusionError> {
    fn walk(root: &Path, current: &Path, tree: &mut InspectedTree) -> Result<(), FusionError> {
        tree._guards.push(fs_safety::guard(current)?);
        for entry in fs::read_dir(current).map_err(|e| io_error("List input tree", current, e))? {
            let path = entry
                .map_err(|e| io_error("Read input entry", current, e))?
                .path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|e| io_error("Inspect input", &path, e))?;
            fs_safety::check_metadata(&metadata, &path)?;
            let relative = path
                .strip_prefix(root)
                .map_err(|e| io_error("Resolve input path", &path, e))?;
            let relative = relative
                .to_str()
                .ok_or_else(|| invalid("Input paths must be Unicode."))?
                .replace('\\', "/");
            tree.paths.add(&relative, metadata.is_dir())?;
            if metadata.is_dir() {
                walk(root, &path, tree)?;
            }
        }
        Ok(())
    }
    let mut tree = InspectedTree {
        paths: PathTree::default(),
        _guards: Vec::new(),
    };
    walk(root, root, &mut tree)?;
    Ok(tree)
}
