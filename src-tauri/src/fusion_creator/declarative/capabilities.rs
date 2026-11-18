//! Closed host capabilities. Source data never supplies executable implementations.
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::program_schema::ProgramControlsProfile;

mod eden_param_package;

type ParameterEncoder = fn(&[(&str, &str)]) -> Result<String, FusionError>;

enum Implementation {
    Ready,
    // A pure encoder cannot apply a controls profile or authorize execution.
    EncoderOnly(ParameterEncoder),
}

struct Capability {
    id: &'static str,
    version: &'static str,
    implementation: Implementation,
}

const HOST_API_V1: &str = "1.0";
const EDEN_PARAM_PACKAGE: &str = "eden_param_package_v1";
const APPLICATION_DEFAULTS: &str = "application_defaults";
static CAPABILITIES: &[Capability] = &[
    Capability {
        id: "fusion.declarative",
        version: HOST_API_V1,
        implementation: Implementation::Ready,
    },
    Capability {
        id: APPLICATION_DEFAULTS,
        version: HOST_API_V1,
        implementation: Implementation::Ready,
    },
    Capability {
        id: EDEN_PARAM_PACKAGE,
        version: HOST_API_V1,
        implementation: Implementation::EncoderOnly(eden_param_package::encode),
    },
];

fn lookup(id: &str, version: &str) -> Option<&'static Capability> {
    CAPABILITIES
        .iter()
        .find(|entry| entry.id == id && entry.version == version)
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ProgramCapabilityRegistry;

impl ProgramCapabilityRegistry {
    /// Pure host-side encoding of already translated fields, not an INI writer.
    /// This does not make a corresponding controls profile available to the planner.
    pub fn encode_parameters(
        &self,
        id: &str,
        version: &str,
        entries: &[(&str, &str)],
    ) -> Result<String, FusionError> {
        match lookup(id, version).map(|entry| &entry.implementation) {
            Some(Implementation::EncoderOnly(encode)) => encode(entries),
            _ => Err(FusionError::new(
                ErrorCode::ActionUnavailable,
                "Host parameter encoder is not registered.",
            )),
        }
    }

    pub fn require(&self, id: &str, version: &str) -> Result<String, FusionError> {
        let Some(entry) =
            lookup(id, version).filter(|e| matches!(e.implementation, Implementation::Ready))
        else {
            return Err(FusionError::new(
                ErrorCode::ActionUnavailable,
                "Host capability is not available for execution.",
            ));
        };
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
        hash.update(entry.id.as_bytes());
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
            ProgramControlsProfile::ApplicationDefaults { .. } => APPLICATION_DEFAULTS,
            ProgramControlsProfile::EdenParamPackageV1 { .. } => EDEN_PARAM_PACKAGE,
        };
        self.require(id, HOST_API_V1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDEN: &str = "eden_param_package_v1";

    #[test]
    fn parameter_encoder_requires_the_exact_registered_capability_and_version() {
        let registry = ProgramCapabilityRegistry;
        assert_eq!(
            registry.encode_parameters(EDEN, "1.0", &[]).unwrap(),
            "[empty]"
        );
        for (id, version) in [
            (EDEN, "1"),
            (EDEN, "2.0"),
            ("Eden_param_package_v1", "1.0"),
            ("application_defaults", "1.0"),
            ("fusion.declarative", "1.0"),
        ] {
            assert_eq!(
                registry
                    .encode_parameters(id, version, &[])
                    .unwrap_err()
                    .code,
                ErrorCode::ActionUnavailable
            );
        }
    }

    #[test]
    fn parameter_encoder_escapes_keys_and_values_in_deterministic_order() {
        let registry = ProgramCapabilityRegistry;
        let entries = [("z:,", "雪$0"), ("engine", "key:,$")];
        let expected = "engine:key$0$1$2,z$0$1:雪$20";
        assert_eq!(
            registry.encode_parameters(EDEN, "1.0", &entries).unwrap(),
            expected
        );
        assert_eq!(
            registry
                .encode_parameters(EDEN, "1.0", &[entries[1], entries[0]])
                .unwrap(),
            expected
        );
    }

    #[test]
    fn parameter_encoder_escapes_nested_stick_packages_at_each_level() {
        let registry = ProgramCapabilityRegistry;
        let right = registry
            .encode_parameters(EDEN, "1.0", &[("engine", "keyboard"), ("code", "68")])
            .unwrap();
        let empty = registry.encode_parameters(EDEN, "1.0", &[]).unwrap();
        assert_eq!(
            registry
                .encode_parameters(
                    EDEN,
                    "1.0",
                    &[
                        ("right", &right),
                        ("left", &empty),
                        ("engine", "analog_from_button")
                    ]
                )
                .unwrap(),
            "engine:analog_from_button,left:[empty],right:code$068$1engine$0keyboard"
        );
    }

    #[test]
    fn parameter_encoder_rejects_lossy_or_oversized_entries_without_echoing_data() {
        let registry = ProgramCapabilityRegistry;
        let oversized = "$".repeat(8192);
        let too_many: Vec<String> = (0..65).map(|i| i.to_string()).collect();
        let too_many: Vec<_> = too_many.iter().map(|key| (key.as_str(), "v")).collect();
        for entries in [
            &[("engine", "keyboard"), ("engine", "sdl")][..],
            &[("", "v")],
            &[("key", "")],
            &[("key", "value\r\n[Controls]")],
            &[("key\0", "value")],
            &[("key", oversized.as_str())],
            &too_many,
        ] {
            let error = registry
                .encode_parameters(EDEN, "1.0", entries)
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidCapability);
            assert!(error.details.is_empty());
            assert!(!error.message.contains("[Controls]"));
        }
    }
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
