//! Pure semantic compilation of the frozen declarative descriptor contract.
use std::collections::{BTreeMap, BTreeSet};

use crate::fusion_creator::projection::{
    resolve_projection, ProjectionLayer, ProjectionRequest, ProjectionSemantics, ResolvedProjection,
};
use crate::fusion_source::creator_schema::ProjectionRule;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::program_schema::*;

#[cfg(test)]
#[path = "compiler_tests.rs"]
mod tests;

#[derive(Debug, Clone)]
pub struct CompiledProgram {
    pub descriptor: ProgramDescriptor,
    pub descriptor_blake3: String,
    pub projections: BTreeMap<String, ResolvedProjection>,
}

impl CompiledProgram {
    /// Validate an exact finite pair; capability availability is a host decision.
    pub fn selected(
        &self,
        build: &str,
        controls: &str,
    ) -> Result<(&ProgramBuildProfile, &ProgramControlsProfile), FusionError> {
        let profile = self
            .descriptor
            .build_profiles
            .iter()
            .find(|p| p.id == build)
            .ok_or_else(|| invalid("Unknown build profile."))?;
        if !profile.controls_profile_ids.iter().any(|id| id == controls)
            || !self.projections.contains_key(build)
        {
            return Err(invalid(
                "The requested build/controls pair is not compiled and allowed.",
            ));
        }
        let control = self
            .descriptor
            .controls_profiles
            .iter()
            .find(|p| p.id() == controls)
            .ok_or_else(|| invalid("Unknown controls profile."))?;
        Ok((profile, control))
    }
}

pub fn compile_descriptor(
    descriptor: &ProgramDescriptor,
    inventories: &[ProjectionLayer],
) -> Result<CompiledProgram, FusionError> {
    validate_descriptor_metadata(descriptor)?;
    // Filtering before binding would hide missing, duplicate or foreign inputs.
    let mut inputs = BTreeMap::new();
    for layer in inventories {
        let component = descriptor
            .components
            .iter()
            .find(|c| c.layer_id == layer.layer_id)
            .ok_or_else(|| invalid("An inventory references an undeclared component layer."))?;
        if layer.mount_path != component.mount_path
            || inputs.insert(layer.layer_id.as_str(), layer).is_some()
        {
            return Err(invalid(
                "Inventory layer/mount identity differs from the descriptor.",
            ));
        }
        // Unselected components must also contain safe inventories.
        for file in &layer.verified_inventory {
            safe_path(&file.path)?;
        }
        resolve_projection(ProjectionRequest {
            semantics: ProjectionSemantics::Explicit11,
            selected_layers: std::slice::from_ref(layer),
            rules: &[],
        })?;
    }
    if inputs.len() != descriptor.components.len() {
        return Err(invalid(
            "Every descriptor component requires a verified inventory.",
        ));
    }
    let rules: Vec<_> = descriptor
        .projection_rules
        .iter()
        .map(|rule| ProjectionRule {
            path: rule.path.clone(),
            winning_layer: rule.winning_layer.clone(),
        })
        .collect();
    let mut projections = BTreeMap::new();
    for profile in &descriptor.build_profiles {
        let layers: Vec<_> = inventories
            .iter()
            .filter(|layer| profile.layer_ids.contains(&layer.layer_id))
            .cloned()
            .collect();
        let projection = resolve_projection(ProjectionRequest {
            semantics: ProjectionSemantics::Explicit11,
            selected_layers: &layers,
            rules: &rules,
        })?;
        validate_projected_layout(descriptor, &projection)?;
        projections.insert(profile.id.clone(), projection);
    }
    let value = serde_json::to_value(descriptor)
        .map_err(|_| invalid("Descriptor cannot be serialized."))?;
    let bytes =
        serde_json::to_vec(&value).map_err(|_| invalid("Descriptor cannot be serialized."))?;
    Ok(CompiledProgram {
        descriptor: descriptor.clone(),
        descriptor_blake3: blake3::hash(&bytes).to_hex().to_string(),
        projections,
    })
}

fn invalid(message: &str) -> FusionError {
    FusionError::new(ErrorCode::InvalidManifest, message)
}

/// Validate the descriptor before acquisition. No inventory or IO is consulted;
/// required-file/provider/cwd existence is established only by compilation.
pub fn validate_descriptor_metadata(descriptor: &ProgramDescriptor) -> Result<(), FusionError> {
    descriptor.validate()?;
    validate_layout_paths(descriptor, None)?;
    unique_ids(
        descriptor
            .entrypoint
            .environment_paths
            .iter()
            .map(|v| v.name.as_str()),
    )?;
    for variable in &descriptor.entrypoint.environment_paths {
        if protected_environment(&variable.name) {
            return Err(
                invalid("Protected or loader environment variables cannot be overridden.")
                    .with_detail("variable", &variable.name),
            );
        }
    }
    let executable = &descriptor.entrypoint.executable;
    if !executable.to_ascii_lowercase().ends_with(".exe") {
        return Err(invalid(
            "The entrypoint must be an executable, not a script or wrapper.",
        ));
    }
    let mut required = BTreeSet::new();
    let mut main = 0;
    for file in &descriptor.required_files {
        if !required.insert(file.path.to_lowercase()) {
            return Err(invalid("Required file paths must be unique."));
        }
        if file.role == ProgramFileRole::MainExecutable {
            main += 1;
            if file.path != *executable {
                return Err(invalid(
                    "The main executable role must identify the entrypoint.",
                ));
            }
        }
    }
    if main != 1 {
        return Err(invalid(
            "Exactly one required file must identify the main executable.",
        ));
    }
    let components = unique_ids(descriptor.components.iter().map(|c| c.id.as_str()))?;
    let layers = unique_ids(descriptor.components.iter().map(|c| c.layer_id.as_str()))?;
    let configs = unique_ids(descriptor.configs.iter().map(|c| c.id.as_str()))?;
    let controls = unique_ids(
        descriptor
            .controls_profiles
            .iter()
            .map(ProgramControlsProfile::id),
    )?;
    unique_ids(descriptor.build_profiles.iter().map(|p| p.id.as_str()))?;
    let _ = components;
    let mut keys = BTreeSet::new();
    for config in &descriptor.configs {
        unique_ids(config.keys.iter().map(|key| key.id.as_str()))?;
        validate_config(config)?;
        keys.extend(
            config
                .keys
                .iter()
                .map(|key| (config.id.as_str(), key.id.as_str())),
        );
    }
    for profile in &descriptor.build_profiles {
        if profile
            .layer_ids
            .iter()
            .any(|id| !layers.contains(id.as_str()))
            || profile
                .controls_profile_ids
                .iter()
                .any(|id| !controls.contains(id.as_str()))
            || profile
                .settings
                .iter()
                .any(|s| !keys.contains(&(s.config_id.as_str(), s.key_id.as_str())))
        {
            return Err(invalid(
                "A profile references an undeclared layer, controls profile or config key.",
            ));
        }
        let mut assigned = BTreeSet::new();
        for setting in &profile.settings {
            if !assigned.insert((setting.config_id.as_str(), setting.key_id.as_str())) {
                return Err(invalid(
                    "A profile assigns the same config key more than once.",
                ));
            }
            let key = descriptor
                .configs
                .iter()
                .find(|c| c.id == setting.config_id)
                .and_then(|c| c.keys.iter().find(|k| k.id == setting.key_id))
                .ok_or_else(|| invalid("Unknown profile config key."))?;
            if !scalar_has_type(&setting.value, key.scalar_type)
                || !key
                    .allowed_values
                    .iter()
                    .any(|v| scalar_equal(v, &setting.value))
            {
                return Err(invalid(
                    "A profile scalar is outside its declared type/domain.",
                ));
            }
        }
        if assigned != keys {
            return Err(invalid(
                "Every profile must assign every declared config key exactly once.",
            ));
        }
    }
    if descriptor
        .projection_rules
        .iter()
        .any(|r| !layers.contains(r.winning_layer.as_str()))
    {
        return Err(invalid(
            "A projection winner references an undeclared layer.",
        ));
    }
    for profile in &descriptor.controls_profiles {
        if let ProgramControlsProfile::EdenParamPackageV1 { config_id, .. } = profile {
            if !configs.contains(config_id.as_str()) {
                return Err(invalid(
                    "A controls writer references an undeclared config.",
                ));
            }
        }
    }
    Ok(())
}

fn unique_ids<'a>(
    ids: impl IntoIterator<Item = &'a str>,
) -> Result<BTreeSet<&'a str>, FusionError> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(invalid("Descriptor identifiers must be unique.").with_detail("id", id));
        }
    }
    Ok(seen)
}

fn validate_projected_layout(
    descriptor: &ProgramDescriptor,
    projection: &ResolvedProjection,
) -> Result<(), FusionError> {
    validate_layout_paths(descriptor, Some(projection))?;
    for file in &descriptor.required_files {
        if !projection.files.contains_key(&file.path.to_lowercase()) {
            return Err(
                invalid("A required file is absent from a build profile projection.")
                    .with_detail("path", &file.path),
            );
        }
    }
    let cwd = descriptor.entrypoint.working_directory.to_lowercase();
    if projection.files.keys().any(|file| at_or_below(&cwd, file)) {
        return Err(invalid(
            "The working directory conflicts with a projected file.",
        ));
    }
    let exists = cwd.is_empty()
        || projection.files.keys().any(|file| at_or_below(file, &cwd))
        || descriptor
            .directories
            .iter()
            .any(|dir| at_or_below(&dir.to_lowercase(), &cwd))
        || descriptor.configs.iter().any(|config| {
            config
                .path
                .rsplit_once('/')
                .is_some_and(|(parent, _)| at_or_below(&parent.to_lowercase(), &cwd))
        })
        || descriptor.persistence.iter().any(|p| {
            p.kind == ProgramPersistenceKind::Directory && at_or_below(&p.path.to_lowercase(), &cwd)
        });
    if !exists {
        return Err(invalid(
            "The working directory is not established by the build profile.",
        ));
    }
    Ok(())
}

fn at_or_below(path: &str, root: &str) -> bool {
    root.is_empty()
        || path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn overlaps(a: &str, b: &str) -> bool {
    at_or_below(a, b) || at_or_below(b, a)
}

/// Supplement the shared lexical validator with Windows console aliases and
/// the superscript device digits accepted by Win32, even on a non-Windows host.
fn safe_path(path: &str) -> Result<(), FusionError> {
    crate::fusion_source::evolution::validate_relative_path(path, false, true, "program.path")?;
    for part in path.split('/') {
        let stem = part.split('.').next().unwrap_or_default().to_uppercase();
        if matches!(stem.as_str(), "CONIN$" | "CONOUT$")
            || ["COM", "LPT"].iter().any(|prefix| {
                stem.strip_prefix(prefix)
                    .is_some_and(|digit| matches!(digit, "¹" | "²" | "³"))
            })
        {
            return Err(invalid("A program path contains a Windows device alias."));
        }
    }
    Ok(())
}

#[derive(Default)]
struct PathTree(BTreeMap<String, (String, bool)>);

impl PathTree {
    fn insert(&mut self, path: &str, file: bool) -> Result<(), FusionError> {
        safe_path(path)?;
        if path.is_empty() {
            return Ok(());
        }
        for end in path
            .match_indices('/')
            .map(|(i, _)| i)
            .chain(std::iter::once(path.len()))
        {
            let spelling = &path[..end];
            let is_file = file && end == path.len();
            let key = spelling.to_lowercase();
            if let Some((previous, previous_file)) = self.0.get(&key) {
                if previous != spelling || *previous_file != is_file {
                    return Err(invalid(
                        "Program paths contain a case alias or file/directory conflict.",
                    )
                    .with_detail("path", path));
                }
            } else {
                self.0.insert(key, (spelling.to_owned(), is_file));
            }
        }
        Ok(())
    }
}

fn validate_layout_paths(
    d: &ProgramDescriptor,
    projection: Option<&ResolvedProjection>,
) -> Result<(), FusionError> {
    let mut tree = PathTree::default();
    for component in &d.components {
        safe_path(&component.source_root)?;
        tree.insert(&component.mount_path, false)?;
    }
    for file in &d.required_files {
        tree.insert(&file.path, true)?;
    }
    for rule in &d.projection_rules {
        tree.insert(&rule.path, true)?;
    }
    tree.insert(&d.entrypoint.executable, true)?;
    tree.insert(&d.entrypoint.working_directory, false)?;
    for dir in &d.directories {
        tree.insert(dir, false)?;
    }
    for env in &d.entrypoint.environment_paths {
        tree.insert(&env.path, false)?;
    }
    // Workspace argument paths may name either files or directories. Check their
    // lexical spelling without guessing a type or requiring optional files.
    for arg in &d.entrypoint.arguments {
        if let ProgramArgument::WorkspacePath { path, .. } = arg {
            safe_path(path)?;
            let mut spellings = tree.clone_spellings();
            for end in path
                .match_indices('/')
                .map(|(i, _)| i)
                .chain(std::iter::once(path.len()))
            {
                let spelling = &path[..end];
                if spellings
                    .insert(spelling.to_lowercase(), spelling.to_owned())
                    .is_some_and(|old| old != spelling)
                {
                    return Err(invalid(
                        "Workspace argument paths contain a Windows case alias.",
                    ));
                }
            }
        }
    }
    let configs: Vec<_> = d.configs.iter().map(|c| c.path.to_lowercase()).collect();
    for (index, config) in d.configs.iter().enumerate() {
        tree.insert(&config.path, true)?;
        if configs[..index]
            .iter()
            .any(|p| overlaps(p, &configs[index]))
        {
            return Err(invalid("Config paths must not overlap."));
        }
    }
    let persistence: Vec<_> = d
        .persistence
        .iter()
        .map(|p| p.path.to_lowercase())
        .collect();
    for (index, p) in d.persistence.iter().enumerate() {
        tree.insert(&p.path, p.kind == ProgramPersistenceKind::File)?;
        if persistence[..index]
            .iter()
            .any(|other| overlaps(other, &persistence[index]))
        {
            return Err(invalid("Persistence roots must not overlap."));
        }
        for config in &configs {
            if overlaps(config, &persistence[index])
                && !(config == &persistence[index] && p.kind == ProgramPersistenceKind::File)
            {
                return Err(invalid("Only exact file persistence may overlap a config."));
            }
        }
    }
    if let Some(projection) = projection {
        for file in projection.files.values() {
            tree.insert(&file.path, true)?;
            let key = file.path.to_lowercase();
            if configs
                .iter()
                .chain(persistence.iter())
                .any(|path| overlaps(&key, path))
            {
                return Err(invalid(
                    "Mutable config/persistence overlaps immutable projected files.",
                )
                .with_detail("path", &file.path));
            }
        }
    }
    Ok(())
}

impl PathTree {
    fn clone_spellings(&self) -> BTreeMap<String, String> {
        self.0
            .iter()
            .map(|(key, (spelling, _))| (key.clone(), spelling.clone()))
            .collect()
    }
}

fn protected_environment(name: &str) -> bool {
    matches!(
        name,
        "PATH"
            | "PATHEXT"
            | "COMSPEC"
            | "SYSTEMROOT"
            | "WINDIR"
            | "APPDATA"
            | "LOCALAPPDATA"
            | "PROGRAMDATA"
            | "USERPROFILE"
            | "HOME"
            | "HOMEDRIVE"
            | "HOMEPATH"
            | "TEMP"
            | "TMP"
            | "TMPDIR"
            | "SYSTEMDRIVE"
            | "PROGRAMFILES"
            | "PROGRAMFILES_X86"
            | "COMMONPROGRAMFILES"
            | "__COMPAT_LAYER"
            | "JAVA_TOOL_OPTIONS"
            | "JDK_JAVA_OPTIONS"
            | "CLASSPATH"
            | "SSLKEYLOGFILE"
            | "RUSTC_WRAPPER"
            | "RUSTC_WORKSPACE_WRAPPER"
    ) || [
        "LD_", "DYLD_", "QT_", "QML_", "QML2_", "PYTHON", "NODE_", "DOTNET_", "COR_", "CORECLR_",
        "COMPLUS_", "JAVA_", "JDK_", "GCONV_", "GIO_", "GTK_", "PERL", "RUBY", "OPENSSL_",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

fn validate_config(config: &ProgramConfig) -> Result<(), FusionError> {
    match (&config.format, &config.seed) {
        (ProgramConfigFormat::Ini, ProgramConfigSeed::EmptyIni {}) => {}
        (ProgramConfigFormat::XmlAttributes, ProgramConfigSeed::XmlRoot { attributes, .. }) => {
            unique_ids(attributes.iter().map(|a| a.name.as_str()))?;
        }
        _ => return Err(invalid("Config format and seed are incompatible.")),
    }
    let mut addresses = BTreeSet::new();
    for key in &config.keys {
        if !matches!(
            (&config.format, &key.address),
            (ProgramConfigFormat::Ini, ProgramConfigAddress::Ini { .. })
                | (
                    ProgramConfigFormat::XmlAttributes,
                    ProgramConfigAddress::XmlAttribute { .. }
                )
        ) {
            return Err(invalid("Config format and key address are incompatible."));
        }
        let address =
            serde_json::to_string(&key.address).map_err(|_| invalid("Invalid config address."))?;
        if !addresses.insert(address) {
            return Err(invalid("Config key addresses must be unique."));
        }
        if key
            .allowed_values
            .iter()
            .any(|v| !scalar_has_type(v, key.scalar_type))
        {
            return Err(invalid(
                "Config domain is inconsistent with its scalar type.",
            ));
        }
    }
    Ok(())
}

fn scalar_has_type(value: &ProgramScalar, kind: ProgramScalarType) -> bool {
    match (value, kind) {
        (ProgramScalar::Boolean(_), ProgramScalarType::Boolean)
        | (ProgramScalar::String(_), ProgramScalarType::String) => true,
        (ProgramScalar::Number(n), ProgramScalarType::Integer) => n
            .as_f64()
            .is_some_and(|v| v.is_finite() && v.fract() == 0.0 && v.abs() <= MAX_REVISION as f64),
        (ProgramScalar::Number(n), ProgramScalarType::Number) => n
            .as_f64()
            .is_some_and(|v| v.is_finite() && v.abs() <= MAX_REVISION as f64),
        _ => false,
    }
}

fn scalar_equal(a: &ProgramScalar, b: &ProgramScalar) -> bool {
    match (a, b) {
        (ProgramScalar::Number(a), ProgramScalar::Number(b)) => a.as_f64() == b.as_f64(),
        _ => a == b,
    }
}
