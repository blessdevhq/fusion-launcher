//! Closed host capabilities. Source data never supplies executable implementations.
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::program_schema::ProgramControlsProfile;

#[derive(Debug, Default, Clone, Copy)]
pub struct ProgramCapabilityRegistry;

impl ProgramCapabilityRegistry {
    pub fn require(&self, id: &str, version: &str) -> Result<String, FusionError> {
        if version != "1.0" || !matches!(id, "fusion.declarative" | "application_defaults") {
            return Err(FusionError::new(
                ErrorCode::ActionUnavailable,
                "Host capability is not registered.",
            ));
        }
        // ABI strings alone cannot authorize reuse after implementation changes.
        let sources: &[&[u8]] = &[
            include_bytes!("capabilities.rs"),
            include_bytes!("runtime.rs"),
            include_bytes!("compiler.rs"),
            include_bytes!("candidate.rs"),
            include_bytes!("codecs.rs"),
            include_bytes!("io.rs"),
            include_bytes!("zip_policy.rs"),
            include_bytes!("state.rs"),
            include_bytes!("../projection.rs"),
            include_bytes!("../../fusion_source/supervisor.rs"),
            include_bytes!("../../fusion_source/supervisor_windows.rs"),
            include_bytes!("../../fusion_source/workspace.rs"),
            include_bytes!("../../fusion_source/plan.rs"),
            include_bytes!("../../fusion_source/integrity.rs"),
            include_bytes!("../../fusion_source/fs_safety.rs"),
            include_bytes!("../../fusion_source/cas.rs"),
            include_bytes!("../../fusion_source/transforms.rs"),
            include_bytes!("../../fusion_source/program_schema.rs"),
            include_bytes!("../../fusion_source/program_schema/validation.rs"),
            include_bytes!("../../../Cargo.lock"),
        ];
        let mut hash = blake3::Hasher::new();
        hash.update(b"fusion-declarative-host-implementation-v1\0");
        hash.update(id.as_bytes());
        for source in sources {
            hash.update(&(source.len() as u64).to_le_bytes());
            hash.update(source);
        }
        Ok(hash.finalize().to_hex().to_string())
    }

    pub fn require_controls(
        &self,
        profile: &ProgramControlsProfile,
    ) -> Result<String, FusionError> {
        let id = match profile {
            ProgramControlsProfile::ApplicationDefaults { .. } => "application_defaults",
            ProgramControlsProfile::EdenParamPackageV1 { .. } => "eden_param_package_v1",
        };
        self.require(id, "1.0")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_closed_host_implementations_are_available_and_pinned_by_code() {
        let registry = ProgramCapabilityRegistry;
        assert_eq!(
            registry.require("fusion.declarative", "1.0").unwrap().len(),
            64
        );
        assert_eq!(
            registry
                .require("application_defaults", "1.0")
                .unwrap()
                .len(),
            64
        );
        for (id, version) in [
            ("fusion.declarative", "2.0"),
            ("source.native_plugin", "1.0"),
            ("eden_param_package_v1", "1.0"),
        ] {
            assert_eq!(
                registry.require(id, version).unwrap_err().code,
                ErrorCode::ActionUnavailable
            );
        }
        let requested = serde_json::from_value(serde_json::json!({
            "kind":"eden_param_package_v1", "id":"requested", "title":"Requested writer",
            "device":"keyboard_mouse", "config_id":"config", "bindings":{"a":{"kind":"keyboard","code":"KeyA"}}
        })).unwrap();
        assert_eq!(
            registry.require_controls(&requested).unwrap_err().code,
            ErrorCode::ActionUnavailable
        );
    }
}
