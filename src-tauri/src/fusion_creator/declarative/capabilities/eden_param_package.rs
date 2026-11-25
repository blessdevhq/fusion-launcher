//! Eden 0.2.1 ParamPackage encoding and closed player-0 controls planner.
//! Reference: Eden commit 58c1e20ee58efa3900ba616207d460886214480b,
//! src/common/param_package.cpp, src/input_common/drivers/sdl_driver.cpp, and src/frontend_common/config.cpp.

use crate::fusion_creator::input::RawInputBinding;
use crate::fusion_source::creator_schema::{AxisDirection, InputDeviceKind, LogicalBinding};
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::program_schema::{
    ProgramConfig, ProgramConfigAddress, ProgramConfigFormat, ProgramConfigSeed,
    ProgramControlsProfile, ProgramLogicalBinding,
};
use std::collections::BTreeMap;

// Host limits for this codec, not claims about Eden's parser limits.
const MAX_ENTRIES: usize = 64;
const MAX_BYTES: usize = 16 * 1024;
const CONTROLS_SECTION: &str = "Controls";
const SDL_CONTROLLER_BUTTONS: &[&str] = &[
    "a",
    "b",
    "x",
    "y",
    "back",
    "guide",
    "start",
    "leftstick",
    "rightstick",
    "leftshoulder",
    "rightshoulder",
    "dpup",
    "dpdown",
    "dpleft",
    "dpright",
    "misc1",
    "paddle1",
    "paddle2",
    "paddle3",
    "paddle4",
    "touchpad",
];
const SDL_CONTROLLER_AXES: &[&str] = &[
    "leftx",
    "lefty",
    "rightx",
    "righty",
    "lefttrigger",
    "righttrigger",
];
const EDEN_BUTTON_ACTIONS: &[&str] = &[
    "a",
    "b",
    "x",
    "y",
    "lstick",
    "rstick",
    "l",
    "r",
    "zl",
    "zr",
    "plus",
    "minus",
    "dleft",
    "dup",
    "dright",
    "ddown",
    "sl",
    "sr",
    "home",
    "screenshot",
];
const EDEN_STICK_ACTIONS: &[&str] = &[
    "lstick_up",
    "lstick_down",
    "lstick_left",
    "lstick_right",
    "rstick_up",
    "rstick_down",
    "rstick_left",
    "rstick_right",
];

/// Validated host-local SDL state for `eden_param_package_v1`. Never serialized into portable exports.
#[derive(Debug, Clone)]
pub(crate) struct EdenLocalSdlBinding {
    pub(crate) guid: String,
    pub(crate) port: u32,
    pub(crate) bindings: BTreeMap<String, LogicalBinding>,
    pub(crate) raw_bindings: BTreeMap<String, RawInputBinding>,
    pub(crate) deadzones: BTreeMap<String, f64>,
    pub(crate) sdl_mapping: String,
}

#[cfg(windows)]
impl From<&crate::input::service::PrivateBindingView> for EdenLocalSdlBinding {
    fn from(view: &crate::input::service::PrivateBindingView) -> Self {
        Self {
            guid: view.identity.guid.clone(),
            port: view.identity.port.unwrap_or(0),
            bindings: view.bindings.clone(),
            raw_bindings: view.raw_bindings.clone(),
            deadzones: view.deadzones.clone(),
            sdl_mapping: view.sdl_mapping.clone(),
        }
    }
}

pub(super) fn encode(entries: &[(&str, &str)]) -> Result<String, FusionError> {
    if entries.len() > MAX_ENTRIES {
        return Err(invalid("Too many ParamPackage fields."));
    }
    if entries.is_empty() {
        return Ok("[empty]".into());
    }

    let mut sorted = BTreeMap::new();
    for &(key, value) in entries {
        // Eden splits with getline: a trailing empty value after ':' is lost.
        for field in [key, value] {
            if field.len() > MAX_BYTES {
                return Err(invalid("ParamPackage exceeds the encoded byte limit."));
            }
            if field.is_empty() || field.chars().any(char::is_control) {
                return Err(invalid(
                    "ParamPackage fields must be nonempty and contain no control characters.",
                ));
            }
        }
        if sorted.insert(key, value).is_some() {
            return Err(invalid("Duplicate ParamPackage key."));
        }
    }

    let mut output = String::new();
    for (key, value) in sorted {
        if !output.is_empty() {
            output.push(',');
        }
        push_escaped(&mut output, key)?;
        output.push(':');
        push_escaped(&mut output, value)?;
    }
    Ok(output)
}

fn push_escaped(output: &mut String, field: &str) -> Result<(), FusionError> {
    for ch in field.chars() {
        match ch {
            ':' => output.push_str("$0"),
            ',' => output.push_str("$1"),
            '$' => output.push_str("$2"),
            _ => output.push(ch),
        }
        if output.len() > MAX_BYTES {
            return Err(invalid("ParamPackage exceeds the encoded byte limit."));
        }
    }
    Ok(())
}

fn qt_key_code(code: &str) -> Option<u32> {
    if let Some(letter) = code.strip_prefix("Key") {
        let [byte] = letter.as_bytes() else {
            return None;
        };
        if byte.is_ascii_uppercase() {
            return Some(u32::from(*byte));
        }
    }
    if let Some(digit) = code.strip_prefix("Digit") {
        let [byte] = digit.as_bytes() else {
            return None;
        };
        if byte.is_ascii_digit() {
            return Some(u32::from(*byte));
        }
    }
    if let Some(num) = code.strip_prefix('F') {
        if let Ok(index @ 1..=12) = num.parse::<u32>() {
            return Some(16_777_263 + index);
        }
    }
    Some(match code {
        "Space" => 32,
        "Quote" => 39,
        "Comma" => 44,
        "Minus" => 45,
        "Period" => 46,
        "Slash" => 47,
        "Semicolon" => 59,
        "Equal" => 61,
        "BracketLeft" => 91,
        "Backslash" => 92,
        "BracketRight" => 93,
        "Backquote" => 96,
        "Escape" => 16_777_216,
        "Tab" => 16_777_217,
        "Backspace" => 16_777_219,
        "Enter" => 16_777_220,
        "ArrowLeft" => 16_777_234,
        "ArrowUp" => 16_777_235,
        "ArrowRight" => 16_777_236,
        "ArrowDown" => 16_777_237,
        "ShiftLeft" | "ShiftRight" => 16_777_248,
        "ControlLeft" | "ControlRight" => 16_777_249,
        "AltLeft" | "AltRight" => 16_777_251,
        _ => return None,
    })
}

fn is_supported_action(action: &str) -> bool {
    EDEN_BUTTON_ACTIONS.contains(&action) || EDEN_STICK_ACTIONS.contains(&action)
}

fn to_logical_binding(binding: &ProgramLogicalBinding) -> Result<LogicalBinding, FusionError> {
    Ok(match binding {
        ProgramLogicalBinding::Keyboard { code } => LogicalBinding::Keyboard { code: code.clone() },
        ProgramLogicalBinding::MouseButton { button } => {
            let button = button
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| invalid("Unsupported mouse button index."))?;
            LogicalBinding::MouseButton { button }
        }
        ProgramLogicalBinding::MouseAxis { axis, direction } => LogicalBinding::MouseAxis {
            axis: axis.clone(),
            direction: direction.clone(),
        },
        ProgramLogicalBinding::ControllerButton { button } => LogicalBinding::ControllerButton {
            button: button.clone(),
        },
        ProgramLogicalBinding::ControllerAxis { axis, direction } => {
            LogicalBinding::ControllerAxis {
                axis: axis.clone(),
                direction: direction.clone(),
            }
        }
    })
}

fn validate_logical_binding_for_device(
    device: &InputDeviceKind,
    binding: &LogicalBinding,
) -> Result<(), FusionError> {
    match (device, binding) {
        (InputDeviceKind::KeyboardMouse, LogicalBinding::Keyboard { code })
            if qt_key_code(code).is_some() =>
        {
            Ok(())
        }
        (InputDeviceKind::KeyboardMouse, LogicalBinding::MouseButton { button })
            if *button <= 7 =>
        {
            Ok(())
        }
        (InputDeviceKind::SdlGamecontroller, LogicalBinding::ControllerButton { button })
            if SDL_CONTROLLER_BUTTONS.contains(&button.as_str()) =>
        {
            Ok(())
        }
        (InputDeviceKind::SdlGamecontroller, LogicalBinding::ControllerAxis { axis, .. })
            if SDL_CONTROLLER_AXES.contains(&axis.as_str()) =>
        {
            Ok(())
        }
        _ => Err(invalid(
            "Controls binding is outside the supported capability device or input allowlist.",
        )),
    }
}

pub(crate) fn validate_profile(
    profile: &ProgramControlsProfile,
    configs: &[ProgramConfig],
) -> Result<(), FusionError> {
    let ProgramControlsProfile::EdenParamPackageV1 {
        device,
        bindings,
        config_id,
        ..
    } = profile
    else {
        return Ok(());
    };
    let config = configs
        .iter()
        .find(|item| &item.id == config_id)
        .ok_or_else(|| invalid("Controls profile references an undeclared config."))?;
    if config.format != ProgramConfigFormat::Ini
        || !matches!(config.seed, ProgramConfigSeed::EmptyIni {})
    {
        return Err(invalid(
            "eden_param_package_v1 requires an INI config with an empty_ini seed.",
        ));
    }
    for (action, binding) in bindings {
        if !is_supported_action(action) {
            return Err(invalid("Unsupported controls action in Eden capability."));
        }
        let logical = to_logical_binding(binding)?;
        validate_logical_binding_for_device(device, &logical)?;
    }
    Ok(())
}

pub(crate) fn render_ini_plan(
    profile: &ProgramControlsProfile,
    config: &ProgramConfig,
    local_sdl: Option<&EdenLocalSdlBinding>,
) -> Result<BTreeMap<(String, String), String>, FusionError> {
    validate_profile(profile, std::slice::from_ref(config))?;
    let ProgramControlsProfile::EdenParamPackageV1 {
        device, bindings, ..
    } = profile
    else {
        return Ok(BTreeMap::new());
    };

    let mut plan = BTreeMap::new();
    let insert = |plan: &mut BTreeMap<(String, String), String>, key: &str, value: String| {
        if config.keys.iter().any(|managed| {
            matches!(
                &managed.address,
                ProgramConfigAddress::Ini {
                    section,
                    key: managed_key,
                } if section == CONTROLS_SECTION && managed_key == key
            )
        }) {
            return Err(invalid(
                "eden_param_package_v1 key collides with a managed scalar config key.",
            ));
        }
        plan.insert((CONTROLS_SECTION.to_owned(), key.to_owned()), value);
        Ok(())
    };
    let quote_param = |raw: &str| -> Result<String, FusionError> {
        serde_json::to_string(raw).map_err(|_| invalid("ParamPackage cannot be quoted."))
    };

    insert(&mut plan, "player_0_connected", "true".into())?;
    insert(&mut plan, "player_0_connected\\default", "false".into())?;
    insert(&mut plan, "player_0_type", "0".into())?;
    insert(&mut plan, "player_0_type\\default", "false".into())?;

    let mut logical_bindings = BTreeMap::new();
    for (action, binding) in bindings {
        logical_bindings.insert(action.clone(), to_logical_binding(binding)?);
    }

    match device {
        InputDeviceKind::KeyboardMouse => {
            for &action in EDEN_BUTTON_ACTIONS {
                if let Some(binding) = logical_bindings.get(action) {
                    let encoded = encode_kb_mouse(binding)?;
                    insert(
                        &mut plan,
                        &format!("player_0_button_{action}"),
                        quote_param(&encoded)?,
                    )?;
                    insert(
                        &mut plan,
                        &format!("player_0_button_{action}\\default"),
                        "false".into(),
                    )?;
                }
            }
            for (stick, dirs) in [
                (
                    "player_0_lstick",
                    ["lstick_up", "lstick_down", "lstick_left", "lstick_right"],
                ),
                (
                    "player_0_rstick",
                    ["rstick_up", "rstick_down", "rstick_left", "rstick_right"],
                ),
            ] {
                if dirs.iter().any(|dir| logical_bindings.contains_key(*dir)) {
                    let dir_pkg = |dir: &str| -> Result<String, FusionError> {
                        match logical_bindings.get(dir) {
                            Some(binding) => encode_kb_mouse(binding),
                            None => encode(&[]),
                        }
                    };
                    let u = dir_pkg(dirs[0])?;
                    let d = dir_pkg(dirs[1])?;
                    let l = dir_pkg(dirs[2])?;
                    let r = dir_pkg(dirs[3])?;
                    let stick_pkg = encode(&[
                        ("down", &d),
                        ("engine", "analog_from_button"),
                        ("left", &l),
                        ("right", &r),
                        ("up", &u),
                    ])?;
                    insert(&mut plan, stick, quote_param(&stick_pkg)?)?;
                    insert(&mut plan, &format!("{stick}\\default"), "false".into())?;
                }
            }
        }
        InputDeviceKind::SdlGamecontroller => {
            let Some(sdl) = local_sdl else {
                return Err(FusionError::new(
                    ErrorCode::ActionUnavailable,
                    "Select and save a live SDL GameController session before applying this controls profile.",
                ));
            };
            validate_local_sdl(sdl)?;
            for (action, binding) in &sdl.bindings {
                if !is_supported_action(action) {
                    return Err(invalid("Unsupported controls action in local bindings."));
                }
                validate_logical_binding_for_device(&InputDeviceKind::SdlGamecontroller, binding)?;
                logical_bindings.insert(action.clone(), binding.clone());
            }

            // Pin SDL hints in INI to match src-tauri/src/input/eden.rs::HINTS.
            insert(&mut plan, "enable_raw_input", "false".into())?;
            insert(&mut plan, "enable_raw_input\\default", "false".into())?;
            insert(&mut plan, "enable_joycon_driver", "true".into())?;
            insert(&mut plan, "enable_joycon_driver\\default", "false".into())?;
            insert(&mut plan, "enable_procon_driver", "false".into())?;
            insert(&mut plan, "enable_procon_driver\\default", "false".into())?;

            for &action in EDEN_BUTTON_ACTIONS {
                if let Some(binding) = logical_bindings.get(action) {
                    let raw = resolve_sdl_raw(action, binding, sdl)?;
                    let encoded = encode_sdl_button(&raw, &sdl.guid, sdl.port)?;
                    insert(
                        &mut plan,
                        &format!("player_0_button_{action}"),
                        quote_param(&encoded)?,
                    )?;
                    insert(
                        &mut plan,
                        &format!("player_0_button_{action}\\default"),
                        "false".into(),
                    )?;
                }
            }

            for (stick, dirs) in [
                (
                    "player_0_lstick",
                    ["lstick_up", "lstick_down", "lstick_left", "lstick_right"],
                ),
                (
                    "player_0_rstick",
                    ["rstick_up", "rstick_down", "rstick_left", "rstick_right"],
                ),
            ] {
                if dirs.iter().any(|dir| logical_bindings.contains_key(*dir)) {
                    let stick_pkg = encode_sdl_stick(dirs, &logical_bindings, sdl)?;
                    insert(&mut plan, stick, quote_param(&stick_pkg)?)?;
                    insert(&mut plan, &format!("{stick}\\default"), "false".into())?;
                }
            }
        }
    }

    Ok(plan)
}

fn validate_local_sdl(sdl: &EdenLocalSdlBinding) -> Result<(), FusionError> {
    if sdl.guid.len() != 32
        || !sdl
            .guid
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("Invalid local SDL device GUID."));
    }
    if sdl.deadzones.len() > SDL_CONTROLLER_AXES.len()
        || sdl.deadzones.iter().any(|(axis, value)| {
            !SDL_CONTROLLER_AXES.contains(&axis.as_str())
                || !value.is_finite()
                || !(0.0..1.0).contains(value)
        })
    {
        return Err(invalid("Invalid local SDL deadzone calibration."));
    }
    Ok(())
}

fn encode_kb_mouse(binding: &LogicalBinding) -> Result<String, FusionError> {
    match binding {
        LogicalBinding::Keyboard { code } => {
            let qt = qt_key_code(code).ok_or_else(|| invalid("Unsupported keyboard code."))?;
            let qt_str = qt.to_string();
            encode(&[("code", &qt_str), ("engine", "keyboard")])
        }
        LogicalBinding::MouseButton { button } if *button <= 7 => {
            let btn = button.to_string();
            encode(&[("button", &btn), ("engine", "mouse")])
        }
        _ => Err(invalid(
            "Keyboard/mouse profile requires keyboard or mouse bindings.",
        )),
    }
}

fn resolve_sdl_raw(
    action: &str,
    logical: &LogicalBinding,
    sdl: &EdenLocalSdlBinding,
) -> Result<RawInputBinding, FusionError> {
    if let Some(raw) = sdl.raw_bindings.get(action) {
        return Ok(raw.clone());
    }
    #[cfg(windows)]
    {
        if !sdl.sdl_mapping.is_empty() {
            return crate::input::eden::resolve_raw(&sdl.sdl_mapping, logical)
                .map_err(|_| invalid("SDL mapping has no source for controls binding."));
        }
    }
    let _ = logical;
    Err(invalid(
        "Missing local SDL raw binding for controls profile.",
    ))
}

fn encode_sdl_button(raw: &RawInputBinding, guid: &str, port: u32) -> Result<String, FusionError> {
    let port_str = port.to_string();
    match raw {
        RawInputBinding::Button { index } => {
            let idx = index.to_string();
            encode(&[
                ("button", &idx),
                ("engine", "sdl"),
                ("guid", guid),
                ("port", &port_str),
            ])
        }
        RawInputBinding::Hat { index, mask } => {
            let direction = match mask {
                1 => "up",
                2 => "right",
                4 => "down",
                8 => "left",
                _ => return Err(invalid("Unsupported SDL hat mask.")),
            };
            let idx = index.to_string();
            encode(&[
                ("direction", direction),
                ("engine", "sdl"),
                ("guid", guid),
                ("hat", &idx),
                ("port", &port_str),
            ])
        }
        RawInputBinding::Axis { index, direction } => {
            let idx = index.to_string();
            let dir = match direction {
                AxisDirection::Positive => "+",
                AxisDirection::Negative => "-",
            };
            encode(&[
                ("axis", &idx),
                ("direction", dir),
                ("engine", "sdl"),
                ("guid", guid),
                ("port", &port_str),
                ("threshold", "0.500000"),
            ])
        }
    }
}

fn encode_sdl_stick(
    dirs: [&str; 4],
    logical_bindings: &BTreeMap<String, LogicalBinding>,
    sdl: &EdenLocalSdlBinding,
) -> Result<String, FusionError> {
    // Emit a native SDL two-axis stick package when opposite directions share raw X and Y axes.
    if let (Some(u_log), Some(d_log), Some(l_log), Some(r_log)) = (
        logical_bindings.get(dirs[0]),
        logical_bindings.get(dirs[1]),
        logical_bindings.get(dirs[2]),
        logical_bindings.get(dirs[3]),
    ) {
        let u_raw = resolve_sdl_raw(dirs[0], u_log, sdl)?;
        let d_raw = resolve_sdl_raw(dirs[1], d_log, sdl)?;
        let l_raw = resolve_sdl_raw(dirs[2], l_log, sdl)?;
        let r_raw = resolve_sdl_raw(dirs[3], r_log, sdl)?;
        if let (
            RawInputBinding::Axis {
                index: uy,
                direction: dy_up,
            },
            RawInputBinding::Axis {
                index: dy,
                direction: dy_down,
            },
            RawInputBinding::Axis {
                index: lx,
                direction: dx_left,
            },
            RawInputBinding::Axis {
                index: rx,
                direction: dx_right,
            },
        ) = (&u_raw, &d_raw, &l_raw, &r_raw)
        {
            if lx == rx && dx_left != dx_right && uy == dy && dy_up != dy_down {
                let ax_x = rx.to_string();
                let ax_y = dy.to_string();
                let invert_x = if *dx_right == AxisDirection::Negative {
                    "true"
                } else {
                    "false"
                };
                let invert_y = if *dy_down == AxisDirection::Negative {
                    "true"
                } else {
                    "false"
                };
                let axis_dz = |b: &LogicalBinding| -> f64 {
                    match b {
                        LogicalBinding::ControllerAxis { axis, .. } => {
                            sdl.deadzones.get(axis).copied().unwrap_or(0.0)
                        }
                        _ => 0.0,
                    }
                };
                let deadzone = axis_dz(l_log)
                    .max(axis_dz(r_log))
                    .max(axis_dz(u_log))
                    .max(axis_dz(d_log));
                let dz_str = format!("{deadzone:.6}");
                let port_str = sdl.port.to_string();
                return encode(&[
                    ("axis_x", &ax_x),
                    ("axis_y", &ax_y),
                    ("deadzone", &dz_str),
                    ("engine", "sdl"),
                    ("guid", &sdl.guid),
                    ("invert_x", invert_x),
                    ("invert_y", invert_y),
                    ("port", &port_str),
                    ("range", "1.000000"),
                    ("threshold", "0.500000"),
                ]);
            }
        }
    }

    let dir_pkg = |dir: &str| -> Result<String, FusionError> {
        match logical_bindings.get(dir) {
            Some(logical) => {
                let raw = resolve_sdl_raw(dir, logical, sdl)?;
                encode_sdl_button(&raw, &sdl.guid, sdl.port)
            }
            None => encode(&[]),
        }
    };
    let u = dir_pkg(dirs[0])?;
    let d = dir_pkg(dirs[1])?;
    let l = dir_pkg(dirs[2])?;
    let r = dir_pkg(dirs[3])?;
    encode(&[
        ("down", &d),
        ("engine", "analog_from_button"),
        ("left", &l),
        ("right", &r),
        ("up", &u),
    ])
}

fn invalid(message: &'static str) -> FusionError {
    // Parameter data may contain local device identities or values; never echo them.
    FusionError::new(ErrorCode::InvalidCapability, message)
}
