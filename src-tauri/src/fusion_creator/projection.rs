//! Pure filesystem-projection contract shared by Packer and installer (owner B).
//! No IO, implicit dependency precedence, device state or resolver trait.
#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::fusion_source::canonical::CanonicalGameManifest;
use crate::fusion_source::cas::CasFile;
use crate::fusion_source::creator_schema::{AdapterReference, ProjectionRule};
use crate::fusion_source::errors::FusionError;

#[derive(Debug, Clone)]
pub struct ProjectionLayer {
    pub layer_id: String,
    pub mount_path: String,
    /// Caller has verified inventory hash and each file. Paths are layer-relative.
    pub verified_inventory: Vec<CasFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionSemantics {
    Legacy10,
    Explicit11,
}

#[derive(Debug)]
pub struct ProjectionRequest<'a> {
    pub semantics: ProjectionSemantics,
    pub selected_layers: &'a [ProjectionLayer],
    pub rules: &'a [ProjectionRule],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionFile {
    pub path: String,
    pub winning_layer: String,
    pub source_relative_path: String,
    pub blake3: String,
    pub size: u64,
    /// Identical path/hash providers are compatible; no redundant author decision.
    pub equivalent_layers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProjection {
    /// Keys are validated slash-separated Windows case-folded workspace paths.
    /// Different hashes require a selected winningLayer; dependencies grant no priority.
    pub files: BTreeMap<String, ProjectionFile>,
}

/// Validate a cumulative path tree without fabricating inventories for patch
/// outputs, whose bytes and sizes are not available before applying the patch.
pub(crate) fn validate_projection_paths<'a>(
    paths: impl IntoIterator<Item = &'a str>,
) -> Result<(), FusionError> {
    use crate::fusion_source::{errors::ErrorCode, evolution::validate_relative_path};
    let mut files = BTreeSet::new();
    for path in paths {
        validate_relative_path(path, false, false, "projection.path")?;
        files.insert(path.to_lowercase());
    }
    for path in &files {
        for (index, _) in path.match_indices('/') {
            if files.contains(&path[..index]) {
                return Err(FusionError::new(
                    ErrorCode::InvalidManifest,
                    "Projection contains a file/parent conflict.",
                )
                .with_detail("path", path));
            }
        }
    }
    Ok(())
}

/// Resolve a verified inventory without IO. Legacy order is explicit; 1.1 order
/// never establishes precedence and every decision must resolve a real conflict.
pub fn resolve_projection(
    request: ProjectionRequest<'_>,
) -> Result<ResolvedProjection, FusionError> {
    use crate::fusion_source::{
        cas::validate_hash, errors::ErrorCode, evolution::validate_relative_path,
    };
    let invalid = |message: &str| FusionError::new(ErrorCode::InvalidManifest, message);
    let mut providers: BTreeMap<String, Vec<ProjectionFile>> = BTreeMap::new();
    let mut layer_ids = std::collections::BTreeSet::new();
    let mut spellings = BTreeMap::new();
    for layer in request.selected_layers {
        if layer.layer_id.is_empty() || !layer_ids.insert(&layer.layer_id) {
            return Err(invalid("Projection layer IDs must be nonempty and unique."));
        }
        validate_relative_path(&layer.mount_path, false, true, "projection.mount_path")?;
        let mut seen = std::collections::BTreeSet::new();
        for file in &layer.verified_inventory {
            validate_relative_path(&file.path, false, false, "projection.inventory.path")?;
            validate_hash(&file.blake3)?;
            let path = crate::fusion_source::evolution::mounted_path(&layer.mount_path, &file.path);
            let key = path.to_lowercase();
            if !seen.insert(key.clone()) {
                return Err(invalid(
                    "An inventory contains duplicate case-insensitive paths.",
                ));
            }
            // Windows aliases cannot depend on the order directories are created.
            for end in path
                .match_indices('/')
                .map(|(index, _)| index)
                .chain(std::iter::once(path.len()))
            {
                let spelling = &path[..end];
                if spellings
                    .insert(spelling.to_lowercase(), spelling.to_owned())
                    .is_some_and(|previous| previous != spelling)
                    && request.semantics == ProjectionSemantics::Explicit11
                {
                    return Err(
                        invalid("Projection contains inconsistent Windows path casing.")
                            .with_detail("path", &path),
                    );
                }
            }
            providers.entry(key).or_default().push(ProjectionFile {
                path,
                winning_layer: layer.layer_id.clone(),
                source_relative_path: file.path.clone(),
                blake3: file.blake3.to_ascii_lowercase(),
                size: file.size,
                equivalent_layers: Vec::new(),
            });
        }
    }
    validate_projection_paths(providers.keys().map(String::as_str))?;
    if request.semantics == ProjectionSemantics::Legacy10 && !request.rules.is_empty() {
        return Err(invalid(
            "Legacy projections cannot contain 1.1 winner rules.",
        ));
    }
    let mut rules = BTreeMap::new();
    for rule in request.rules {
        validate_relative_path(&rule.path, false, false, "projection.rule.path")?;
        if rules
            .insert(rule.path.to_lowercase(), rule.winning_layer.as_str())
            .is_some()
        {
            return Err(invalid("Duplicate projection winner rule."));
        }
    }
    let mut files = BTreeMap::new();
    for (key, mut candidates) in providers {
        let mut sizes = BTreeMap::new();
        for candidate in &candidates {
            if sizes
                .insert(candidate.blake3.clone(), candidate.size)
                .is_some_and(|size| size != candidate.size)
            {
                return Err(invalid("The same digest has inconsistent inventory sizes."));
            }
        }
        let winner = if request.semantics == ProjectionSemantics::Legacy10 {
            candidates.last().expect("nonempty providers")
        } else if sizes.len() == 1 {
            if rules.contains_key(&key) {
                return Err(invalid(
                    "A winner rule must resolve different bytes, not an unambiguous path.",
                ));
            }
            candidates.sort_by(|a, b| a.winning_layer.cmp(&b.winning_layer));
            &candidates[0]
        } else {
            let id = rules.remove(&key).ok_or_else(|| {
                invalid("Different projected bytes require an explicit winner.")
                    .with_detail("path", &key)
            })?;
            candidates
                .iter()
                .find(|candidate| candidate.winning_layer == id)
                .ok_or_else(|| {
                    invalid("The winning layer does not supply this selected path.")
                        .with_detail("path", &key)
                })?
        };
        let mut winner = winner.clone();
        winner.equivalent_layers = candidates
            .iter()
            .filter(|candidate| candidate.blake3 == winner.blake3)
            .map(|candidate| candidate.winning_layer.clone())
            .collect();
        winner.equivalent_layers.sort();
        files.insert(key, winner);
    }
    if !rules.is_empty() {
        return Err(invalid(
            "A winner rule does not refer to a selected conflicting path.",
        ));
    }
    Ok(ResolvedProjection { files })
}

/// Shared pure resolver signature.
pub type ResolveProjection =
    for<'a> fn(ProjectionRequest<'a>) -> Result<ResolvedProjection, FusionError>;

#[derive(Debug)]
pub struct NormalizeDirectoryRequest {
    pub input_directory: PathBuf,
    pub private_output_directory: PathBuf,
    pub adapter: AdapterReference,
}
#[derive(Debug)]
pub struct NormalizedDirectory {
    pub directory: PathBuf,
    pub inventory_blake3: String,
    pub files: Vec<CasFile>,
}
/// Same deterministic, streaming normalizer for author and player. No second installer.
pub type NormalizeDirectory =
    fn(&NormalizeDirectoryRequest) -> Result<NormalizedDirectory, FusionError>;

#[derive(Debug)]
pub struct PrepareCandidateRequest<'a> {
    pub manifest: &'a CanonicalGameManifest,
    pub build_profile_id: &'a str,
    pub controls_profile_id: &'a str,
    pub inactive_workspace: &'a std::path::Path,
    pub projection: &'a ResolvedProjection,
}
#[derive(Debug)]
pub struct PreparedCandidate {
    pub inactive_workspace: PathBuf,
    pub configuration_hashes: BTreeMap<String, String>,
}
/// Preparation cannot alter active config, shared saves or private bindings before activation.
pub type PrepareCandidate =
    for<'a> fn(&PrepareCandidateRequest<'a>) -> Result<PreparedCandidate, FusionError>;
