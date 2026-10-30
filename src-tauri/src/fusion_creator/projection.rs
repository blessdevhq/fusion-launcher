//! Pure filesystem-projection contract shared by Packer and installer (owner B).
//! No IO, implicit dependency precedence, device state or resolver trait.
use std::collections::BTreeMap;
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

/// Signature of B's future resolve_projection; no executable stub that could succeed.
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
