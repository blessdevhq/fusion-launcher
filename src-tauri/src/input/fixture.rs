//! Test-only virtual hardware. Never compiled into the application/probe.
use super::{DeviceInfo, Reply};
use sdl2::{sys, GameControllerSubsystem, JoystickSubsystem};

pub(super) enum Action {
    Attach,
    Button(u8, bool),
    Axis(u8, i16),
    Hat(u8),
    Detach,
}

pub(super) struct Fixture {
    id: u32,
    attached: bool,
}

fn checked(code: i32) -> Result<(), String> {
    if code < 0 {
        Err(sdl2::get_error())
    } else {
        Ok(())
    }
}

impl Fixture {
    fn attach(
        joysticks: &JoystickSubsystem,
        controllers: &GameControllerSubsystem,
    ) -> Result<Self, String> {
        // SAFETY: SDL's versioned descriptor is zero initialized as required.
        let mut desc: sys::SDL_VirtualJoystickDesc = unsafe { std::mem::zeroed() };
        desc.version = sys::SDL_VIRTUAL_JOYSTICK_DESC_VERSION as u16;
        desc.type_ = sys::SDL_JoystickType::SDL_JOYSTICK_TYPE_UNKNOWN as u16;
        desc.naxes = 6;
        desc.nbuttons = 2;
        desc.nhats = 1;
        // Synthetic identities make SDL's GUID/CRC matching deterministic.
        desc.vendor_id = 0xf001;
        desc.product_id = 0xf002;
        // SAFETY: live subsystem on its owner thread, valid versioned descriptor.
        let index = unsafe { sys::SDL_JoystickAttachVirtualEx(&desc) };
        checked(index)?;
        // SAFETY: successful attach returned this device index.
        let id = unsafe { sys::SDL_JoystickGetDeviceInstanceID(index) };
        if id < 0 {
            // SAFETY: failed instance lookup has not transferred this fixture index.
            let _ = unsafe { sys::SDL_JoystickDetachVirtual(index) };
            return Err(sdl2::get_error());
        }
        let fixture = Self {
            id: id as u32,
            attached: true,
        };
        let guid = joysticks
            .device_guid(index as u32)
            .map_err(|e| e.to_string())?;
        // Only this synthetic device gets an explicit mapping; never guess a
        // physical device's button numbers or create a portable physical binding.
        controllers.add_mapping(&format!(
            "{guid},Fusion native input fixture,a:b0,b:b1,dpup:h0.1,dpright:h0.2,dpdown:h0.4,dpleft:h0.8,leftx:a0,lefty:a1,rightx:a2,righty:a3,lefttrigger:a4,righttrigger:a5,platform:Windows,"
        )).map_err(|e| e.to_string())?;
        Ok(fixture)
    }

    fn detach(&mut self) -> Result<(), String> {
        if self.attached {
            // SAFETY: subsystem still lives on this thread; no pointer is kept.
            let index = unsafe {
                (0..sys::SDL_NumJoysticks())
                    .find(|&i| sys::SDL_JoystickGetDeviceInstanceID(i) == self.id as i32)
            }
            .ok_or("fixture disappeared before explicit detach")?;
            // SAFETY: index belongs to this fixture, never a physical device.
            checked(unsafe { sys::SDL_JoystickDetachVirtual(index) })?;
            self.attached = false;
        }
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Normal detach is checked. This is error-path cleanup only.
        let _ = self.detach();
    }
}

pub(super) fn apply(
    fixture: &mut Option<Fixture>,
    action: Action,
    joysticks: &JoystickSubsystem,
    controllers: &GameControllerSubsystem,
    selected: Option<u32>,
) -> Result<Reply, String> {
    if matches!(action, Action::Attach) {
        if fixture.is_some() {
            return Err("only one test fixture may be attached".into());
        }
        let device = Fixture::attach(joysticks, controllers)?;
        let id = device.id;
        *fixture = Some(device);
        return Ok(Reply::Devices(vec![DeviceInfo {
            instance_id: id,
            name: "Fusion native input fixture".into(),
            is_game_controller: true,
        }]));
    }
    let fixture = fixture.as_mut().ok_or("fixture is not attached")?;
    if matches!(action, Action::Detach) {
        fixture.detach()?;
        return Ok(Reply::Opened(fixture.id));
    }
    if selected != Some(fixture.id) || !fixture.attached {
        return Err("fixture writes require its live selected controller".into());
    }
    // SAFETY: borrow the GameController-owned joystick, with no second open/ref.
    // sdl2 0.38 Joystick::drop skips closing detached handles; never own one here.
    let raw = unsafe { sys::SDL_JoystickFromInstanceID(fixture.id as i32) };
    if raw.is_null() {
        return Err("fixture controller-owned joystick is missing".into());
    }
    match action {
        Action::Button(index, value) if index < 2 => {
            // SAFETY: fixture live on owner thread; validated button range.
            checked(unsafe {
                sys::SDL_JoystickSetVirtualButton(raw, index.into(), u8::from(value))
            })?;
        }
        Action::Axis(index, value) if index < 6 => {
            // SAFETY: fixture live on owner thread; validated axis range.
            checked(unsafe { sys::SDL_JoystickSetVirtualAxis(raw, index.into(), value) })?;
        }
        Action::Hat(value) if matches!(value, 0 | 1 | 2 | 3 | 4 | 6 | 8 | 9 | 12) => {
            // SAFETY: one fixture hat, valid SDL bitmask.
            checked(unsafe { sys::SDL_JoystickSetVirtualHat(raw, 0, value) })?;
        }
        _ => return Err("invalid fixture input".into()),
    }
    Ok(Reply::Opened(fixture.id))
}
