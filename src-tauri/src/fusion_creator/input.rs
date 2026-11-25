//! SDL-worker boundary; physical identity and calibration remain local to the player.
use crate::fusion_source::creator_schema::{AxisDirection, LogicalBinding};
use crate::fusion_source::errors::FusionError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const INPUT_STATE_EVENT: &str = "fusion-input:state";
pub const INPUT_COMMANDS: &[&str] = &[
    "fusion_input_devices",
    "fusion_input_session_start",
    "fusion_input_session_stop",
    "fusion_input_session_get",
    "fusion_input_bindings_save",
    "fusion_input_release_for_launch",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputDevice {
    pub device_id: String,
    pub name: String,
    pub connected: bool,
    pub mapping_known: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputPrivateIdentity {
    pub guid: String,
    pub path: Option<String>,
    pub serial: Option<String>,
    pub port: Option<u32>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RawInputBinding {
    Button {
        index: u32,
    },
    Hat {
        index: u32,
        mask: u8,
    },
    Axis {
        index: u32,
        direction: AxisDirection,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSessionStatus {
    Active,
    Disconnected,
    Stopping,
    Stopped,
    Failed,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputState {
    pub session_id: String,
    /// Strictly increasing per session, 1..=2^53-1; terminal state cannot be reopened.
    pub sequence: u64,
    pub device_id: Option<String>,
    pub status: InputSessionStatus,
    pub buttons: BTreeMap<String, bool>,
    /// Logical stick axes -1..1, triggers 0..1; unknown mapping is unavailable, not guessed.
    pub axes: BTreeMap<String, f64>,
    pub error: Option<FusionError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputSessionRequest {
    pub session_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputSessionStartRequest {
    pub device_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputSessionReceipt {
    pub session_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputReleaseReceipt {
    pub session_id: String,
    /// True only after worker confirms all handles closed. Timeout is an error.
    pub handles_closed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputBindingsReceipt {
    pub session_id: String,
    pub controls_profile_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputBindingsSaveRequest {
    pub session_id: String,
    pub controls_profile_id: String,
    pub bindings: BTreeMap<String, LogicalBinding>,
    pub deadzones: BTreeMap<String, f64>,
    pub raw_bindings: BTreeMap<String, RawInputBinding>,
}

/// Message-based worker API. SDL contexts and handles must never leave their worker.
/// No implementation/default success is installed in wave zero.
pub trait InputProvider: Send + Sync {
    fn devices(&self) -> Result<Vec<InputDevice>, FusionError>;
    fn start(&self, request: InputSessionStartRequest) -> Result<InputSessionReceipt, FusionError>;
    fn state(&self, request: InputSessionRequest) -> Result<InputState, FusionError>;
    fn stop(&self, request: InputSessionRequest) -> Result<InputReleaseReceipt, FusionError>;
    fn save_bindings(
        &self,
        request: InputBindingsSaveRequest,
    ) -> Result<InputBindingsReceipt, FusionError>;
    fn release_for_launch(
        &self,
        request: InputSessionRequest,
    ) -> Result<InputReleaseReceipt, FusionError>;
}
