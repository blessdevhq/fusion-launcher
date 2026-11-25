//! Memory-only extension contract. v1 deliberately embeds no Wasm runtime and
//! exposes no filesystem, network, process, environment or host-object imports.

use std::time::Duration;

use super::errors::{ErrorCode, FusionError};

pub const WASM_TRANSFORM_CAPABILITY: &str = "ext.wasm_transform.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WasmAccessMode {
    MemoryBufferOnly,
}

#[derive(Debug, Clone, Copy)]
pub struct WasmExecutionLimits {
    pub max_input_bytes: usize,
    pub max_output_bytes: usize,
    /// Total linear memory, including module allocations and input/output.
    pub max_memory_bytes: usize,
    pub execution_timeout: Duration,
}

impl Default for WasmExecutionLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 1024 * 1024,
            max_output_bytes: 1024 * 1024,
            max_memory_bytes: 16 * 1024 * 1024,
            execution_timeout: Duration::from_secs(1),
        }
    }
}

impl WasmExecutionLimits {
    pub fn validate(&self) -> Result<(), FusionError> {
        if self.max_input_bytes == 0
            || self.max_output_bytes == 0
            || self.max_memory_bytes == 0
            || self.max_memory_bytes > 256 * 1024 * 1024
            || self.max_input_bytes > self.max_memory_bytes
            || self.max_output_bytes > self.max_memory_bytes
            || self.execution_timeout.is_zero()
            || self.execution_timeout > Duration::from_secs(30)
        {
            return Err(FusionError::new(
                ErrorCode::InvalidManifest,
                "Invalid memory-only extension execution budget.",
            ));
        }
        Ok(())
    }
}

pub struct WasmTransformRequest<'a> {
    pub module_bytes: &'a [u8],
    pub input: &'a [u8],
    pub access_mode: WasmAccessMode,
    pub limits: WasmExecutionLimits,
}

pub trait WasmExtensionHost: Send + Sync {
    /// A future engine must enforce memory/fuel/wall-clock budgets throughout
    /// execution, reject host imports and bound output before allocation.
    fn transform(&self, request: &WasmTransformRequest<'_>) -> Result<Vec<u8>, FusionError>;

    fn supported_capabilities(&self) -> &'static [&'static str];
}

#[derive(Debug, Clone, Copy, Default)]
pub struct UnsupportedWasmHost;

impl WasmExtensionHost for UnsupportedWasmHost {
    fn transform(&self, _request: &WasmTransformRequest<'_>) -> Result<Vec<u8>, FusionError> {
        Err(FusionError::new(
            ErrorCode::MissingCapability,
            "This client cannot execute Wasm transforms.",
        )
        .with_detail("capability", WASM_TRANSFORM_CAPABILITY)
        .with_detail("accessMode", "memory_buffer_only"))
    }

    fn supported_capabilities(&self) -> &'static [&'static str] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::fusion_source::capabilities::{CapabilityNegotiator, CapabilityRegistry};
    use crate::fusion_source::evolution::SchemaUpcaster;
    use crate::fusion_source::schema::{CapabilityFallback, FallbackStrategy};

    #[test]
    fn unsupported_host_never_claims_execution_and_fallback_is_negotiated() {
        let host = UnsupportedWasmHost;
        let error = host
            .transform(&WasmTransformRequest {
                module_bytes: b"not executed",
                input: b"private input",
                access_mode: WasmAccessMode::MemoryBufferOnly,
                limits: WasmExecutionLimits::default(),
            })
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::MissingCapability);
        assert!(host.supported_capabilities().is_empty());
        let mut manifest = SchemaUpcaster::upcast_game_manifest(
            "fixtures",
            include_str!("fixtures/switch-zelda.json"),
            &BTreeMap::new(),
        )
        .unwrap();
        manifest.capabilities.required.clear();
        manifest.capabilities.optional = vec![WASM_TRANSFORM_CAPABILITY.into()];
        manifest.fallback = vec![CapabilityFallback {
            capability: WASM_TRANSFORM_CAPABILITY.into(),
            strategy: FallbackStrategy::BuiltinConfigTemplates,
            requires: vec![],
        }];
        let report =
            CapabilityNegotiator::negotiate(&manifest, &CapabilityRegistry::current_client());
        assert!(!report.is_blocked());
        assert!(report.uses_fallback(&FallbackStrategy::BuiltinConfigTemplates));
        assert!(!CapabilityRegistry::current_client().supports(WASM_TRANSFORM_CAPABILITY));
        manifest.capabilities.required = vec![WASM_TRANSFORM_CAPABILITY.into()];
        manifest.capabilities.optional.clear();
        manifest.fallback.clear();
        assert!(
            CapabilityNegotiator::negotiate(&manifest, &CapabilityRegistry::current_client())
                .is_blocked()
        );
    }

    #[test]
    fn execution_budgets_reject_unbounded_memory_or_time() {
        WasmExecutionLimits::default().validate().unwrap();
        assert!(WasmExecutionLimits {
            max_memory_bytes: usize::MAX,
            ..WasmExecutionLimits::default()
        }
        .validate()
        .is_err());
        assert!(WasmExecutionLimits {
            execution_timeout: Duration::ZERO,
            ..WasmExecutionLimits::default()
        }
        .validate()
        .is_err());
    }
}
