//! Portable Creator extension of GameManifest 1.1. No filesystem or device identity.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const MANIFEST_SCHEMA_VERSION: &str = "1.1";
pub const PROJECT_SCHEMA_VERSION: &str = "1.0";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterReference {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComponentOrigin {
    Managed {
        artifact_ref: String,
        redistribution_allowed: bool,
    },
    External {
        url: String,
        transport_blake3: String,
        inventory_blake3: String,
    },
    UserProvided {
        requirement_id: String,
        inventory_blake3: String,
        instructions: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildProfile {
    pub id: String,
    pub title: String,
    /// Exact selected layer set; dependencies do not imply precedence.
    pub layer_ids: Vec<String>,
    /// Finite supported pairings. Arbitrary addons cannot be mixed with a profile.
    pub controls_profile_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputDeviceKind {
    KeyboardMouse,
    SdlGamecontroller,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AxisDirection {
    Positive,
    Negative,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseAxis {
    X,
    Y,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LogicalBinding {
    Keyboard {
        code: String,
    },
    MouseButton {
        button: u32,
    },
    MouseAxis {
        axis: MouseAxis,
        direction: AxisDirection,
    },
    ControllerButton {
        button: String,
    },
    ControllerAxis {
        axis: String,
        direction: AxisDirection,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlsProfile {
    pub id: String,
    pub title: String,
    pub device: InputDeviceKind,
    /// Portable logical SDL/keyboard inputs. Only adapter D produces ParamPackage.
    pub bindings: BTreeMap<String, LogicalBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionRule {
    /// Validated workspace-relative Windows case-insensitive path, slash separated.
    pub path: String,
    /// Must supply this exact path in the selected, verified inventory.
    pub winning_layer: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestComponent {
    pub layer_id: String,
    pub origin: ComponentOrigin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatorManifest {
    pub adapter: AdapterReference,
    pub components: Vec<ManifestComponent>,
    pub build_profiles: Vec<BuildProfile>,
    pub controls_profiles: Vec<ControlsProfile>,
    pub projection_rules: Vec<ProjectionRule>,
}

/// Strict wire alternatives, not two independent publication formats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CreatorExtension {
    Program(Box<super::program_schema::ProgramCreatorManifest>),
    Legacy(CreatorManifest),
}

impl CreatorExtension {
    pub fn as_legacy(&self) -> Option<&CreatorManifest> {
        match self {
            Self::Legacy(value) => Some(value),
            Self::Program(_) => None,
        }
    }
    pub fn as_legacy_mut(&mut self) -> Option<&mut CreatorManifest> {
        match self {
            Self::Legacy(value) => Some(value),
            Self::Program(_) => None,
        }
    }
    pub fn as_program(&self) -> Option<&super::program_schema::ProgramCreatorManifest> {
        match self {
            Self::Program(value) => Some(value),
            Self::Legacy(_) => None,
        }
    }
}
