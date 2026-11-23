//! Production Build: sign and audit one managed-directory release without exporting it.
use super::jobs::{JobContext, JobHandler, ResumePoint};
use super::{CreatorCheckpoint, CreatorJobStartRequest, CreatorRelease, SavedPrivateBindings};
use crate::fusion_source::cas::directory_digest;
use crate::fusion_source::creator_schema::ComponentOrigin;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::packer::{PackerEngine, PackerRecipe};
use crate::fusion_source::program_schema::{
    CreatorProgramProject, ProgramComponent, ProgramControlsProfile,
};
use crate::fusion_source::schema::{DigestAlgorithm, SourceIndex};
use crate::fusion_source::{fs_safety, signatures};
use base64::Engine as _;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const LOCAL_SOURCE_ROOT: &str = "https://offline.fusion.invalid/creator/";
const INDEX_ID: &str = "source-index.json";
const INVENTORY_ID: &str = "artifact-inventory.json";

pub(super) struct ManagedDirectoryBuild;

pub(super) fn supported_component(component: &ProgramComponent) -> bool {
    component.source_root.is_empty()
        && matches!(
            component.origin,
            ComponentOrigin::Managed {
                redistribution_allowed: true,
                ..
            }
        )
}

fn validate_request(request: &CreatorJobStartRequest) -> Result<(), FusionError> {
    if request.target_directory.is_some()
        || request.source_url.is_some()
        || request.release_id.is_some()
    {
        return Err(unavailable("Build creates a local release; it does not accept an output directory, Source URL, or existing release."));
    }
    Ok(())
}

fn unavailable(message: &'static str) -> FusionError {
    FusionError::new(ErrorCode::ActionUnavailable, message)
        .with_detail("command", "fusion_creator_build")
}

fn integrity() -> FusionError {
    FusionError::new(
        ErrorCode::IntegrityViolation,
        "Creator Build output does not match its checked inputs.",
    )
}

fn redact(error: FusionError) -> FusionError {
    FusionError::new(
        error.code,
        "Creator Build could not prepare or verify the managed release.",
    )
}

fn digest(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn implementation_digest() -> &'static str {
    static DIGEST: OnceLock<String> = OnceLock::new();
    DIGEST.get_or_init(|| {
        let mut hash = blake3::Hasher::new();
        for bytes in [
            include_bytes!("build.rs").as_slice(),
            include_bytes!("declarative/compiler.rs"),
            include_bytes!("declarative/runtime.rs"),
            include_bytes!("declarative/capabilities.rs"),
            include_bytes!("declarative/codecs.rs"),
            include_bytes!("projection.rs"),
            include_bytes!("../fusion_source/packer.rs"),
            include_bytes!("../fusion_source/plan.rs"),
            include_bytes!("../fusion_source/cas.rs"),
            include_bytes!("../fusion_source/signatures.rs"),
            include_bytes!("../../Cargo.lock"),
        ] {
            hash.update(&blake3::hash(bytes).as_bytes()[..]);
        }
        hash.finalize().to_hex().to_string()
    })
}

struct Inputs<'a> {
    key: &'a Path,
    paths: Vec<(&'a ProgramComponent, &'a Path)>,
}

impl<'a> Inputs<'a> {
    fn resolve(
        project: &'a CreatorProgramProject,
        bindings: Option<&'a SavedPrivateBindings>,
    ) -> Result<Self, FusionError> {
        if project
            .program_descriptor
            .components
            .iter()
            .any(|c| !supported_component(c))
        {
            return Err(unavailable("Build supports only redistributable managed directories with an empty source root."));
        }
        if project
            .program_descriptor
            .controls_profiles
            .iter()
            .any(|profile| !matches!(profile, ProgramControlsProfile::ApplicationDefaults { .. }))
        {
            return Err(unavailable("Build does not support this controls writer."));
        }
        let bindings = bindings.map(|saved| &saved.bindings);
        let key = bindings
            .and_then(|b| b.signing_key_ref.as_deref())
            .map(Path::new)
            .filter(|path| path.is_absolute())
            .ok_or_else(|| {
                FusionError::new(
                    ErrorCode::InvalidSourceSignature,
                    "Select a local Source signing key before building.",
                )
            })?;
        let paths = project
            .program_descriptor
            .components
            .iter()
            .map(|c| {
                let path = bindings
                    .and_then(|b| b.component_paths.get(&c.id))
                    .map(Path::new)
                    .filter(|path| path.is_absolute())
                    .ok_or_else(|| {
                        FusionError::new(
                            ErrorCode::MissingLayer,
                            "Select a local component directory before building.",
                        )
                        .with_detail("componentId", &c.id)
                    })?;
                Ok((c, path))
            })
            .collect::<Result<_, FusionError>>()?;
        Ok(Self { key, paths })
    }

    fn recipe(
        &self,
        project: &CreatorProgramProject,
        public_key: String,
    ) -> Result<PackerRecipe, FusionError> {
        let layers: Vec<_> = self
            .paths
            .iter()
            .map(|(component, path)| {
                serde_json::json!({
                    "id":component.layer_id, "kind":"runtime", "local_path":path, "archive":"zip"
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "source_info": { "id":project.source_id, "name":project.title, "public_key":public_key },
            "base_url":LOCAL_SOURCE_ROOT,
            "games":[{
                "id":project.game_id, "title":project.title, "version":project.game_version,
                "layers":layers, "program_descriptor":project.program_descriptor
            }]
        })).map_err(|_| integrity())
    }
}

fn public_identity(path: &Path) -> Result<(String, String), FusionError> {
    let key = signatures::read_signing_key(path)
        .map_err(redact)?
        .verifying_key();
    Ok((
        base64::engine::general_purpose::STANDARD.encode(key.as_bytes()),
        digest(key.as_bytes()),
    ))
}

impl JobHandler for ManagedDirectoryBuild {
    fn validate_request(&self, request: &CreatorJobStartRequest) -> Result<(), FusionError> {
        validate_request(request)
    }
    fn validate_write_roots(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
        workspace: &Path,
        release_cas: &Path,
    ) -> Result<(), FusionError> {
        let inputs = Inputs::resolve(project, bindings)?;
        super::jobs::validate_build_write_roots(
            inputs
                .paths
                .into_iter()
                .map(|(component, path)| (component.id.as_str(), path)),
            workspace,
            release_cas,
        )
    }
    fn input_hashes(
        &self,
        project: &CreatorProgramProject,
        bindings: Option<&SavedPrivateBindings>,
    ) -> Result<BTreeMap<String, String>, FusionError> {
        let inputs = Inputs::resolve(project, bindings)?;
        let (_, signer_digest) = public_identity(inputs.key)?;
        let mut hashes = BTreeMap::from([
            ("source_signing_identity".into(), signer_digest),
            (
                "executor_implementation".into(),
                implementation_digest().into(),
            ),
        ]);
        for (component, path) in inputs.paths {
            let actual = directory_digest(path)
                .map_err(|error| redact(error).with_detail("componentId", &component.id))?;
            hashes.insert(format!("component:{}", component.id), actual);
        }
        Ok(hashes)
    }

    fn validate_checkpoint(&self, _: &CreatorCheckpoint, _: &[u8]) -> Result<bool, FusionError> {
        // The runner validates release receipts itself; no partial checkpoints are reusable.
        Ok(false)
    }

    fn run(&self, context: JobContext, resume: Option<ResumePoint>) -> Result<(), FusionError> {
        validate_request(context.request())?;
        if resume.is_some() {
            return Err(unavailable("Build has no partial state to resume."));
        }
        context.progress("packing", 0, None)?;
        let project = context.project();
        let bindings = context.private_bindings()?;
        let inputs = Inputs::resolve(project, bindings.as_ref())?;
        let (public_key, signer_digest) = public_identity(inputs.key)?;
        if !context.matches_input("source_signing_identity", &signer_digest) {
            return Err(changed());
        }
        let recipe = inputs.recipe(project, public_key.clone())?;
        let scratch = PrivateStage::new(context.workspace(), "build")?;
        let publication = scratch.path.join("source");
        let packer = PackerEngine::new();
        let built = context.blocking_library(|_| {
            packer
                .build_with_signing_key(&recipe, &scratch.path, &publication, Some(inputs.key))
                .map_err(redact)
        })?;
        if built.artifacts.len() != inputs.paths.len()
            || built.manifests.len() != 1
            || !built.attestations.is_empty()
        {
            return Err(integrity());
        }
        for (component, _) in &inputs.paths {
            let mut artifacts = built.artifacts.iter().filter(|artifact| {
                artifact.layer_id == component.layer_id && artifact.game_id == project.game_id
            });
            let artifact = artifacts.next().ok_or_else(integrity)?;
            if artifacts.next().is_some()
                || !context
                    .matches_input(&format!("component:{}", component.id), &artifact.cas_digest)
            {
                return Err(changed());
            }
        }
        context.progress("auditing", 0, None)?;
        let verified = context.blocking_library(|_| packer.verify(&publication).map_err(redact))?;
        if !verified.signature_verified
            || verified.source_id != project.source_id
            || verified.manifests_verified != 1
            || verified.artifacts_verified != built.artifacts.len()
            || verified.attestations_verified != 0
        {
            return Err(integrity());
        }

        let stage = PrivateStage::new(context.workspace(), "release")?;
        let artifacts_dir = stage.path.join("artifacts");
        let _artifacts = fs_safety::ensure_directory(&artifacts_dir).map_err(redact)?;
        let raw_index = metadata_bytes(&built.source_index)?;
        let text = std::str::from_utf8(&raw_index).map_err(|_| integrity())?;
        let index: SourceIndex = serde_json::from_str(text).map_err(|_| integrity())?;
        let signed = signatures::parse_wire_index(text).map_err(redact)?;
        let [entry] = index.games.as_slice() else {
            return Err(integrity());
        };
        if index.source_info.public_key.as_deref() != Some(public_key.as_str())
            || index.source_info.id != project.source_id
            || entry.id != project.game_id
            || entry.version != project.game_version
            || !signatures::verify_wire_index(&signed, None, None).map_err(redact)?
        {
            return Err(integrity());
        }
        let manifest_stem = format!("manifests/{}-{}", entry.id, entry.manifest_blake3);
        let raw_manifest = metadata_bytes(&publication.join(format!("{manifest_stem}.json")))?;
        if digest(&raw_manifest) != entry.manifest_blake3 {
            return Err(integrity());
        }
        let inventory =
            metadata_bytes(&publication.join(format!("{manifest_stem}.artifacts.json")))?;
        let mut artifact_hashes = BTreeMap::new();
        for (id, bytes) in [(INDEX_ID, raw_index), (INVENTORY_ID, inventory)] {
            fs_safety::atomic_write(&artifacts_dir.join(id), &bytes).map_err(redact)?;
            artifact_hashes.insert(id.to_owned(), digest(&bytes));
        }
        fs_safety::atomic_write(&stage.path.join("manifest.json"), &raw_manifest)
            .map_err(redact)?;
        for artifact in &built.artifacts {
            context.check_cancelled()?;
            let hash = &artifact.transport_digest.value;
            if artifact.transport_digest.algorithm != DigestAlgorithm::Blake3
                || hash.len() != 64
                || !hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return Err(integrity());
            }
            let id = format!("{hash}.zip");
            if artifact.path != publication.join("artifacts").join(&id) {
                return Err(integrity());
            }
            if !artifact_hashes.contains_key(&id) {
                fs_safety::copy_file(&artifact.path, &artifacts_dir.join(&id)).map_err(redact)?;
                // checkpoint_release audits the copied bytes against this expected hash.
                artifact_hashes.insert(id, hash.clone());
            }
        }
        context
            .checkpoint_release(
                CreatorRelease {
                    release_id: fs_safety::unique_name("release"),
                    project_id: project.project_id.clone(),
                    project_revision: project.revision,
                    manifest_blake3: entry.manifest_blake3.clone(),
                    artifact_hashes,
                    report_ids: vec![],
                },
                &stage.name,
            )
            .map_err(redact)?;
        Ok(())
    }
}

fn changed() -> FusionError {
    FusionError::new(
        ErrorCode::SnapshotConflict,
        "Creator Build inputs changed during packing.",
    )
}

fn metadata_bytes(path: &Path) -> Result<Vec<u8>, FusionError> {
    const MAX: u64 = 8 * 1024 * 1024;
    let mut bytes = Vec::new();
    fs_safety::open_read(path)
        .map_err(redact)?
        .take(MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| super::jobs::io())?;
    if !(1..=MAX).contains(&(bytes.len() as u64)) {
        return Err(integrity());
    }
    Ok(bytes)
}

struct PrivateStage {
    name: String,
    path: PathBuf,
}
impl PrivateStage {
    fn new(workspace: &Path, prefix: &str) -> Result<Self, FusionError> {
        let name = fs_safety::unique_name(prefix);
        let path = workspace.join(&name);
        signatures::create_private_directory(&path).map_err(|_| super::jobs::io())?;
        Ok(Self { name, path })
    }
}
impl Drop for PrivateStage {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
