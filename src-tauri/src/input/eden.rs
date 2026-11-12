//! Local SDL compatibility with Eden 0.2.1, NOT a ParamPackage writer.
//! Pin: 58c1e20ee58efa3900ba616207d460886214480b,
//! src/input_common/drivers/sdl_driver.cpp (GetGUID, constructor, InitJoystick).
use crate::fusion_creator::input::RawInputBinding;
use crate::fusion_source::creator_schema::{AxisDirection, LogicalBinding};

/// Pinned Windows defaults: raw_input=false, joycon_driver=true, procon_driver=false.
/// D must use these same settings; changed driver settings require fresh selection.
/// No motion/sensor APIs are enabled. Dual Joy-Con are deliberately disabled.
pub const HINTS: &[(&str, &str)] = &[
    ("SDL_APP_NAME", "Fusion Launcher"),
    ("SDL_JOYSTICK_RAWINPUT", "0"),
    ("SDL_ACCELEROMETER_AS_JOYSTICK", "0"),
    ("SDL_JOYSTICK_HIDAPI_PS4_RUMBLE", "1"),
    ("SDL_JOYSTICK_HIDAPI_PS5_RUMBLE", "1"),
    ("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1"),
    ("SDL_JOYSTICK_HIDAPI_JOY_CONS", "0"),
    ("SDL_JOYSTICK_HIDAPI_COMBINE_JOY_CONS", "0"),
    ("SDL_JOYSTICK_HIDAPI_SWITCH", "1"),
    ("SDL_JOYSTICK_HIDAPI_SWITCH_HOME_LED", "0"),
    ("SDL_JOYSTICK_HIDAPI_SWITCH_PLAYER_LED", "1"),
    ("SDL_GAMECONTROLLER_USE_BUTTON_LABELS", "0"),
    ("SDL_JOYSTICK_HIDAPI_XBOX", "0"),
];

pub(super) fn apply_hints() -> Result<(), String> {
    for &(key, value) in HINTS {
        if !sdl2::hint::set_with_priority(key, value, &sdl2::hint::Hint::Override)
            || sdl2::hint::get(key).as_deref() != Some(value)
        {
            return Err(format!("required SDL hint not applied: {key}"));
        }
    }
    Ok(())
}

pub fn normalize_guid(guid: &str) -> Result<String, String> {
    if guid.len() != 32 || !guid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid 16-byte SDL GUID".into());
    }
    let mut value = guid.to_ascii_lowercase();
    // SDL's string is byte-ordered. Eden zeroes the name CRC, NOT VID/PID/version.
    value.replace_range(4..8, "0000");
    Ok(value)
}

pub const AXES: [&str; 6] = [
    "leftx",
    "lefty",
    "rightx",
    "righty",
    "lefttrigger",
    "righttrigger",
];

pub fn calibrate(value: f64, deadzone: f64) -> f64 {
    if value.abs() <= deadzone {
        0.0
    } else {
        (value.signum() * (value.abs() - deadzone) / (1.0 - deadzone)).clamp(-1.0, 1.0)
    }
}

/// Resolve ONLY an explicit SDL mapping. No fallback button numbers. Keep the
/// original mapping in the private view: raw axis half-ranges/inversion must be
/// interpreted by D, not discarded by converting everything into button indices.
pub fn resolve_raw(mapping: &str, logical: &LogicalBinding) -> Result<RawInputBinding, String> {
    let (name, direction) = match logical {
        LogicalBinding::ControllerButton { button }
            if sdl2::controller::Button::from_string(button).is_some() =>
        {
            (button.as_str(), AxisDirection::Positive)
        }
        LogicalBinding::ControllerAxis { axis, direction } if AXES.contains(&axis.as_str()) => {
            (axis.as_str(), direction.clone())
        }
        _ => return Err("this session supports only known SDL GameController inputs".into()),
    };
    let mut values = mapping.split(',').skip(2).filter_map(|field| {
        let (key, value) = field.split_once(':')?;
        (key == name).then_some(value)
    });
    let value = values
        .next()
        .ok_or("SDL mapping has no source for logical input")?;
    if values.next().is_some() {
        return Err("ambiguous SDL mapping source".into());
    }
    if let Some(index) = value.strip_prefix('b') {
        return Ok(RawInputBinding::Button {
            index: parse_index(index)?,
        });
    }
    if let Some(hat) = value.strip_prefix('h') {
        let (index, mask) = hat.split_once('.').ok_or("invalid SDL hat mapping")?;
        let mask: u8 = mask.parse().map_err(|_| "invalid SDL hat mask")?;
        if !matches!(mask, 1 | 2 | 4 | 8) {
            return Err("unsupported SDL hat mapping mask".into());
        }
        return Ok(RawInputBinding::Hat {
            index: parse_index(index)?,
            mask,
        });
    }
    let inverted = value.ends_with('~');
    let value = value.trim_end_matches('~');
    let (half, axis) = if let Some(axis) = value.strip_prefix('+') {
        (Some(AxisDirection::Positive), axis)
    } else if let Some(axis) = value.strip_prefix('-') {
        (Some(AxisDirection::Negative), axis)
    } else {
        (None, value)
    };
    let index = axis
        .strip_prefix('a')
        .ok_or("unsupported SDL mapping source")?;
    let direction = half.unwrap_or(direction);
    Ok(RawInputBinding::Axis {
        index: parse_index(index)?,
        direction: if inverted {
            match direction {
                AxisDirection::Positive => AxisDirection::Negative,
                AxisDirection::Negative => AxisDirection::Positive,
            }
        } else {
            direction
        },
    })
}

fn parse_index(value: &str) -> Result<u32, String> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("invalid raw SDL input index".into());
    }
    value
        .parse()
        .map_err(|_| "raw SDL input index overflow".into())
}
