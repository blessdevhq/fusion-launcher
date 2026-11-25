//! Closed host capabilities. Source data never supplies executable implementations.
use std::collections::BTreeMap;

use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::program_schema::{ProgramConfig, ProgramControlsProfile};

pub(crate) mod eden_param_package;
use eden_param_package::EdenLocalSdlBinding;

#[derive(Clone, Copy)]
enum Implementation {
    Ready,
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
        implementation: Implementation::Ready,
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
    pub fn encode_parameters(
        &self,
        id: &str,
        version: &str,
        entries: &[(&str, &str)],
    ) -> Result<String, FusionError> {
        match (
            id,
            version,
            lookup(id, version).map(|entry| entry.implementation),
        ) {
            (EDEN_PARAM_PACKAGE, HOST_API_V1, Some(Implementation::Ready)) => {
                eden_param_package::encode(entries)
            }
            _ => Err(FusionError::new(
                ErrorCode::ActionUnavailable,
                "Host parameter encoder is not registered.",
            )),
        }
    }

    pub(crate) fn validate_controls_profile(
        &self,
        profile: &ProgramControlsProfile,
        configs: &[ProgramConfig],
    ) -> Result<(), FusionError> {
        self.require_controls(profile)?;
        eden_param_package::validate_profile(profile, configs)
    }

    pub(crate) fn render_controls_plan(
        &self,
        profile: &ProgramControlsProfile,
        config: &ProgramConfig,
        local_sdl: Option<&EdenLocalSdlBinding>,
    ) -> Result<BTreeMap<(String, String), String>, FusionError> {
        self.require_controls(profile)?;
        eden_param_package::render_ini_plan(profile, config, local_sdl)
    }

    pub fn require(&self, id: &str, version: &str) -> Result<String, FusionError> {
        let Some(entry) =
            lookup(id, version).filter(|e| matches!(e.implementation, Implementation::Ready))
        else {
            return Err(FusionError::new(
                ErrorCode::ActionUnavailable,
                "Host capability is not registered.",
            ));
        };
        // ABI strings alone cannot authorize reuse after implementation changes.
        let sources: &[&[u8]] = &[
            include_bytes!("capabilities.rs"),
            include_bytes!("capabilities/eden_param_package.rs"),
            include_bytes!("../../input/eden.rs"),
            include_bytes!("../../input/service.rs"),
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
    use crate::fusion_creator::input::RawInputBinding;
    use crate::fusion_source::creator_schema::{AxisDirection, LogicalBinding};

    const EDEN: &str = EDEN_PARAM_PACKAGE;

    fn ini_config() -> ProgramConfig {
        serde_json::from_value(serde_json::json!({
            "id": "eden-main",
            "path": "eden/user/config/qt-config.ini",
            "format": "ini",
            "seed": {"kind": "empty_ini"},
            "preserve_unmanaged": true,
            "keys": [{
                "id": "backend",
                "address": {"kind": "ini", "section": "Renderer", "key": "backend"},
                "scalar_type": "integer",
                "allowed_values": [0, 1]
            }]
        }))
        .unwrap()
    }

    #[test]
    fn param_package_escapes_sorted_entries_and_nested_sticks() {
        let registry = ProgramCapabilityRegistry;
        let expected = "engine:keyboard,key:code$068$1split$2x";
        let entries = [("key", "code:68,split$x"), ("engine", "keyboard")];
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
    fn param_package_rejects_lossy_or_oversized_entries_without_echoing_data() {
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
    fn eden_capability_plans_keyboard_mouse_and_sdl_and_rejects_missing_or_colliding_inputs() {
        let registry = ProgramCapabilityRegistry;
        let cfg = ini_config();

        let kb_profile: ProgramControlsProfile = serde_json::from_value(serde_json::json!({
            "kind": "eden_param_package_v1",
            "id": "kb",
            "title": "Keyboard",
            "device": "keyboard_mouse",
            "config_id": "eden-main",
            "bindings": {
                "a": {"kind": "keyboard", "code": "KeyX"},
                "r": {"kind": "mouse_button", "button": 1},
                "lstick_right": {"kind": "keyboard", "code": "KeyD"}
            }
        }))
        .unwrap();
        let kb_plan = registry
            .render_controls_plan(&kb_profile, &cfg, None)
            .unwrap();
        assert_eq!(
            kb_plan[&("Controls".into(), "player_0_connected".into())],
            "true"
        );
        assert_eq!(
            kb_plan[&("Controls".into(), "player_0_button_a".into())],
            "\"code:88,engine:keyboard\""
        );
        assert_eq!(
            kb_plan[&("Controls".into(), "player_0_button_r".into())],
            "\"button:1,engine:mouse\""
        );
        assert_eq!(
            kb_plan[&("Controls".into(), "player_0_lstick".into())],
            "\"down:[empty],engine:analog_from_button,left:[empty],right:code$068$1engine$0keyboard,up:[empty]\""
        );

        let sdl_profile: ProgramControlsProfile = serde_json::from_value(serde_json::json!({
            "kind": "eden_param_package_v1",
            "id": "pad",
            "title": "Gamepad",
            "device": "sdl_gamecontroller",
            "config_id": "eden-main",
            "bindings": {
                "a": {"kind": "controller_button", "button": "a"},
                "lstick_up": {"kind": "controller_axis", "axis": "lefty", "direction": "negative"},
                "lstick_down": {"kind": "controller_axis", "axis": "lefty", "direction": "positive"},
                "lstick_left": {"kind": "controller_axis", "axis": "leftx", "direction": "negative"},
                "lstick_right": {"kind": "controller_axis", "axis": "leftx", "direction": "positive"}
            }
        }))
        .unwrap();
        assert_eq!(
            registry
                .render_controls_plan(&sdl_profile, &cfg, None)
                .unwrap_err()
                .code,
            ErrorCode::ActionUnavailable
        );

        let local = EdenLocalSdlBinding {
            guid: "030000005e0400008e02000000007200".into(),
            port: 1,
            bindings: BTreeMap::from([(
                "a".into(),
                LogicalBinding::ControllerButton { button: "b".into() },
            )]),
            raw_bindings: BTreeMap::from([
                ("a".into(), RawInputBinding::Button { index: 1 }),
                (
                    "lstick_up".into(),
                    RawInputBinding::Axis {
                        index: 1,
                        direction: AxisDirection::Negative,
                    },
                ),
                (
                    "lstick_down".into(),
                    RawInputBinding::Axis {
                        index: 1,
                        direction: AxisDirection::Positive,
                    },
                ),
                (
                    "lstick_left".into(),
                    RawInputBinding::Axis {
                        index: 0,
                        direction: AxisDirection::Negative,
                    },
                ),
                (
                    "lstick_right".into(),
                    RawInputBinding::Axis {
                        index: 0,
                        direction: AxisDirection::Positive,
                    },
                ),
            ]),
            deadzones: BTreeMap::from([("leftx".into(), 0.2)]),
            sdl_mapping: String::new(),
        };
        let sdl_plan = registry
            .render_controls_plan(&sdl_profile, &cfg, Some(&local))
            .unwrap();
        assert_eq!(
            sdl_plan[&("Controls".into(), "enable_raw_input".into())],
            "false"
        );
        assert_eq!(
            sdl_plan[&("Controls".into(), "enable_joycon_driver".into())],
            "true"
        );
        assert_eq!(
            sdl_plan[&("Controls".into(), "enable_procon_driver".into())],
            "false"
        );
        assert_eq!(
            sdl_plan[&("Controls".into(), "player_0_button_a".into())],
            "\"button:1,engine:sdl,guid:030000005e0400008e02000000007200,port:1\""
        );
        assert!(sdl_plan[&("Controls".into(), "player_0_lstick".into())]
            .contains("axis_x:0,axis_y:1,deadzone:0.200000,engine:sdl,guid:030000005e0400008e02000000007200,invert_x:false,invert_y:false,port:1,range:1.000000,threshold:0.500000"));

        let mut colliding = cfg;
        colliding.keys.push(
            serde_json::from_value(serde_json::json!({
                "id": "ctrl_connected",
                "address": {"kind": "ini", "section": "Controls", "key": "player_0_connected"},
                "scalar_type": "boolean",
                "allowed_values": [true, false]
            }))
            .unwrap(),
        );
        assert_eq!(
            registry
                .render_controls_plan(&kb_profile, &colliding, None)
                .unwrap_err()
                .code,
            ErrorCode::InvalidCapability
        );
    }

    #[test]
    fn only_closed_host_implementations_are_available_and_pinned_by_code() {
        let registry = ProgramCapabilityRegistry;
        for id in ["fusion.declarative", "application_defaults", EDEN] {
            assert_eq!(registry.require(id, "1.0").unwrap().len(), 64);
        }
        for (id, version) in [
            ("fusion.declarative", "2.0"),
            ("source.native_plugin", "1.0"),
            (EDEN, "2.0"),
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
        assert_eq!(registry.require_controls(&requested).unwrap().len(), 64);
    }
}
