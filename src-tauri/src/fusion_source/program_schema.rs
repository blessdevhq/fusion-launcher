//! Strict wire DTOs for the frozen ProgramDescriptor authoring contract.

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod reader_tests;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Number, Value};

use super::creator_schema::{AxisDirection, ComponentOrigin, InputDeviceKind, MouseAxis};
use super::errors::{ErrorCode, FusionError};

mod validation;
pub(crate) use validation::parse_unique_json;
use validation::*;
pub use validation::{MAX_PROGRAM_BYTES, MAX_REVISION};

macro_rules! constant {
    ($name:ident, $variant:ident, $wire:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name {
            #[serde(rename = $wire)]
            $variant,
        }
    };
}
constant!(ProgramDocumentType, ProgramDescriptor, "program_descriptor");
constant!(ProgramSchemaVersion, V1, "1.0");
constant!(ProjectSchemaVersion, V11, "1.1");
constant!(ProgramOs, Windows, "windows");
constant!(ProgramExecutorId, Declarative, "fusion.declarative");
constant!(ProgramPersistencePolicy, PreserveLatest, "preserve_latest");
constant!(ProgramPersistenceExport, Never, "never");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramArchitecture {
    X86_64,
    X86,
    Aarch64,
    Armv7,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramFileRole {
    Data,
    MainExecutable,
    CoreLibrary,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramConfigFormat {
    Ini,
    XmlAttributes,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramScalarType {
    Boolean,
    Integer,
    Number,
    String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramPersistenceKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum ProgramScalar {
    Boolean(bool),
    Number(Number),
    String(String),
}
impl<'de> Deserialize<'de> for ProgramScalar {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        match Value::deserialize(d)? {
            Value::Bool(v) => Ok(Self::Boolean(v)),
            Value::Number(v) if v.as_f64().is_some_and(|v| v.abs() <= MAX_REVISION as f64) => {
                Ok(Self::Number(v))
            }
            Value::String(v) if v.chars().count() <= 2048 && !v.chars().any(char::is_control) => {
                Ok(Self::String(v))
            }
            _ => Err(serde::de::Error::custom("Invalid program scalar.")),
        }
    }
}
fn scalar_values<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ProgramScalar>, D::Error> {
    let values = list::<D, ProgramScalar, 1, 64>(d)?;
    for (index, value) in values.iter().enumerate() {
        if values[..index].iter().any(|other| match (value, other) {
            (ProgramScalar::Number(a), ProgramScalar::Number(b)) => a.as_f64() == b.as_f64(),
            _ => value == other,
        }) {
            return Err(serde::de::Error::custom("Duplicate allowed scalar values."));
        }
    }
    Ok(values)
}
fn true_value<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    if bool::deserialize(d)? {
        Ok(true)
    } else {
        Err(serde::de::Error::custom(
            "Unmanaged state must be preserved.",
        ))
    }
}
fn mouse_button<'de, D: Deserializer<'de>>(d: D) -> Result<Number, D::Error> {
    let value = Number::deserialize(d)?;
    if !value.as_f64().is_some_and(|v| v >= 0.0 && v.fract() == 0.0) {
        return Err(serde::de::Error::custom(
            "Expected a nonnegative button index.",
        ));
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramIdentity {
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub id: String,
    #[serde(deserialize_with = "text::<_, 1, 256>")]
    pub title: String,
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub version: String,
    pub os: ProgramOs,
    pub architecture: ProgramArchitecture,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramExecutor {
    pub id: ProgramExecutorId,
    pub version: ProgramSchemaVersion,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramComponent {
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub id: String,
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub layer_id: String,
    #[serde(deserialize_with = "origin")]
    pub origin: ComponentOrigin,
    #[serde(deserialize_with = "directory")]
    pub source_root: String,
    #[serde(deserialize_with = "directory")]
    pub mount_path: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramFile {
    #[serde(deserialize_with = "path")]
    pub path: String,
    pub role: ProgramFileRole,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramProjectionRule {
    #[serde(deserialize_with = "path")]
    pub path: String,
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub winning_layer: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProgramArgument {
    Literal {
        #[serde(deserialize_with = "clean")]
        value: String,
    },
    WorkspacePath {
        #[serde(deserialize_with = "directory")]
        path: String,
        #[serde(deserialize_with = "prefix")]
        prefix: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramEnvironmentPath {
    #[serde(deserialize_with = "environment")]
    pub name: String,
    #[serde(deserialize_with = "directory")]
    pub path: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramEntrypoint {
    #[serde(deserialize_with = "path")]
    pub executable: String,
    #[serde(deserialize_with = "directory")]
    pub working_directory: String,
    #[serde(deserialize_with = "list::<_, _, 0, 64>")]
    pub arguments: Vec<ProgramArgument>,
    #[serde(deserialize_with = "list::<_, _, 0, 32>")]
    pub environment_paths: Vec<ProgramEnvironmentPath>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProgramConfigAddress {
    Ini {
        #[serde(deserialize_with = "ini_name::<_, false>")]
        section: String,
        #[serde(deserialize_with = "ini_name::<_, true>")]
        key: String,
    },
    XmlAttribute {
        #[serde(deserialize_with = "xml_elements")]
        elements: Vec<String>,
        #[serde(deserialize_with = "xml_name")]
        attribute: String,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramConfigKey {
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub id: String,
    pub address: ProgramConfigAddress,
    pub scalar_type: ProgramScalarType,
    #[serde(deserialize_with = "scalar_values")]
    pub allowed_values: Vec<ProgramScalar>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramXmlAttribute {
    #[serde(deserialize_with = "xml_name")]
    pub name: String,
    pub value: ProgramScalar,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProgramConfigSeed {
    EmptyIni {},
    XmlRoot {
        #[serde(deserialize_with = "xml_name")]
        root: String,
        #[serde(deserialize_with = "list::<_, _, 0, 16>")]
        attributes: Vec<ProgramXmlAttribute>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramConfig {
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub id: String,
    #[serde(deserialize_with = "path")]
    pub path: String,
    pub format: ProgramConfigFormat,
    pub seed: ProgramConfigSeed,
    #[serde(deserialize_with = "true_value")]
    pub preserve_unmanaged: bool,
    #[serde(deserialize_with = "list::<_, _, 0, 128>")]
    pub keys: Vec<ProgramConfigKey>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramSetting {
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub config_id: String,
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub key_id: String,
    pub value: ProgramScalar,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramBuildProfile {
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub id: String,
    #[serde(deserialize_with = "text::<_, 0, 256>")]
    pub title: String,
    #[serde(deserialize_with = "ids::<_, 64>")]
    pub layer_ids: Vec<String>,
    #[serde(deserialize_with = "ids::<_, 16>")]
    pub controls_profile_ids: Vec<String>,
    #[serde(deserialize_with = "list::<_, _, 0, 256>")]
    pub settings: Vec<ProgramSetting>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProgramLogicalBinding {
    Keyboard {
        #[serde(deserialize_with = "text::<_, 1, 128>")]
        code: String,
    },
    MouseButton {
        #[serde(deserialize_with = "mouse_button")]
        button: Number,
    },
    MouseAxis {
        axis: MouseAxis,
        direction: AxisDirection,
    },
    ControllerButton {
        #[serde(deserialize_with = "text::<_, 1, 128>")]
        button: String,
    },
    ControllerAxis {
        #[serde(deserialize_with = "text::<_, 1, 128>")]
        axis: String,
        direction: AxisDirection,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProgramControlsProfile {
    ApplicationDefaults {
        #[serde(deserialize_with = "text::<_, 1, 128>")]
        id: String,
        #[serde(deserialize_with = "text::<_, 0, 256>")]
        title: String,
    },
    EdenParamPackageV1 {
        #[serde(deserialize_with = "text::<_, 1, 128>")]
        id: String,
        #[serde(deserialize_with = "text::<_, 0, 256>")]
        title: String,
        device: InputDeviceKind,
        #[serde(deserialize_with = "bindings")]
        bindings: BTreeMap<String, ProgramLogicalBinding>,
        #[serde(deserialize_with = "text::<_, 1, 128>")]
        config_id: String,
    },
}
impl ProgramControlsProfile {
    pub fn id(&self) -> &str {
        match self {
            Self::ApplicationDefaults { id, .. } | Self::EdenParamPackageV1 { id, .. } => id,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramPersistence {
    #[serde(deserialize_with = "path")]
    pub path: String,
    pub kind: ProgramPersistenceKind,
    pub policy: ProgramPersistencePolicy,
    pub export: ProgramPersistenceExport,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramDescriptor {
    pub document_type: ProgramDocumentType,
    pub schema_version: ProgramSchemaVersion,
    pub identity: ProgramIdentity,
    pub executor: ProgramExecutor,
    #[serde(deserialize_with = "list::<_, _, 1, 64>")]
    pub components: Vec<ProgramComponent>,
    #[serde(deserialize_with = "list::<_, _, 1, 256>")]
    pub required_files: Vec<ProgramFile>,
    #[serde(deserialize_with = "list::<_, _, 0, 4096>")]
    pub projection_rules: Vec<ProgramProjectionRule>,
    #[serde(deserialize_with = "directories")]
    pub directories: Vec<String>,
    pub entrypoint: ProgramEntrypoint,
    #[serde(deserialize_with = "list::<_, _, 0, 16>")]
    pub configs: Vec<ProgramConfig>,
    #[serde(deserialize_with = "list::<_, _, 1, 32>")]
    pub build_profiles: Vec<ProgramBuildProfile>,
    #[serde(deserialize_with = "list::<_, _, 1, 16>")]
    pub controls_profiles: Vec<ProgramControlsProfile>,
    #[serde(deserialize_with = "list::<_, _, 0, 64>")]
    pub persistence: Vec<ProgramPersistence>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatorProgramProject {
    pub schema_version: ProjectSchemaVersion,
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub project_id: String,
    #[serde(deserialize_with = "revision")]
    pub revision: u64,
    pub title: String,
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub source_id: String,
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub game_id: String,
    #[serde(deserialize_with = "text::<_, 1, 128>")]
    pub game_version: String,
    pub program_descriptor: ProgramDescriptor,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramCreatorManifest {
    pub program_descriptor: ProgramDescriptor,
}
fn decode<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, FusionError> {
    serde_json::from_value(parse_unique_json(json, MAX_PROGRAM_BYTES)?).map_err(|_| {
        FusionError::new(
            ErrorCode::InvalidManifest,
            "Invalid program authoring contract.",
        )
    })
}
macro_rules! reader {
    ($type:ty) => {
        impl $type {
            pub fn read_json(json: &str) -> Result<Self, FusionError> {
                decode(json)
            }
            /// Recheck values constructed by Rust callers, not only wire input.
            /// Cross-references, codec/type compatibility and execution policy belong to X.
            pub fn validate(&self) -> Result<(), FusionError> {
                let json = serde_json::to_string(self).map_err(|_| {
                    FusionError::new(
                        ErrorCode::InvalidManifest,
                        "Program metadata cannot be serialized.",
                    )
                })?;
                Self::read_json(&json)?;
                Ok(())
            }
        }
    };
}
reader!(ProgramDescriptor);
reader!(CreatorProgramProject);
reader!(ProgramCreatorManifest);
