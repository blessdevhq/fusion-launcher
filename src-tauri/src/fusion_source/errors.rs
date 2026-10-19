//! Stable, structured errors shared by the pure core and future IPC adapters.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidJson,
    UnsupportedSchema,
    InvalidId,
    InvalidDigest,
    InvalidUri,
    InvalidPath,
    InvalidPublicKey,
    InvalidSourceSignature,
    DuplicateId,
    MissingLayer,
    LayerCycle,
    ExtendsCycle,
    MissingParent,
    InvalidManifest,
    InvalidCapability,
    MissingCapability,
    NoFallback,
    UnknownAddon,
    InvalidRuntimeChannel,
    CrossVolumeLinkNotSupported,
    ProjectionUnavailable,
    IntegrityViolation,
    MissingInjectorDll,
    UntrustedExecutable,
    Io,
    CorruptCasObject,
    SnapshotNotFound,
    SnapshotConflict,
    OperationInProgress,
    InvalidTemplate,
    PersistenceConflict,
    ArchiveInvalid,
    ExtractionLimitExceeded,
    MissingPatchTool,
    PatchToolFailed,
    PatchToolTimeout,
    Cancelled,
    Network,
    TransferLimitExceeded,
    TransportDigestMismatch,
    MirrorsExhausted,
    ManifestDigestMismatch,
    ProviderResolutionFailed,
    LocalBaseModified,
    CorruptedCriticalBinary,
    StrictSyncRequired,
    ProcessLaunchFailed,
    ProcessSupervisionFailed,
    InvalidEnvironment,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FusionError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default)]
    pub details: BTreeMap<String, Value>,
}

impl FusionError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: BTreeMap::new(),
        }
    }

    pub fn with_detail(mut self, key: impl Into<String>, value: impl Serialize) -> Self {
        // Error reporting must remain usable even if an adapter's custom serializer fails.
        let value = serde_json::to_value(value)
            .unwrap_or_else(|_| Value::String("detail could not be serialized".into()));
        self.details.insert(key.into(), value);
        self
    }
}

impl std::fmt::Display for FusionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for FusionError {}
