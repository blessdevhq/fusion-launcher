//! Throwaway native SDL experiment; not a production input service or DTO contract.

use sdl2::controller::{Axis, Button, GameController};
use sdl2::event::Event;

use sdl2::{sys, EventPump, GameControllerSubsystem, JoystickSubsystem};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct Report {
    pub worker_thread: String,
    pub sdl_version: String,
    pub initial_joystick_count: u32,
    pub fixture_guid: String,
    pub instances: Vec<u32>,
    pub polled_events: Vec<String>,
    pub stages: Vec<&'static str>,
    pub initialized_flags_after_drop: u32,
}

#[derive(Debug, PartialEq)]
enum Signal {
    Added(u32), // Device index, NOT instance ID.
    RawAdded(u32),
    Removed(u32), // Instance ID.
    RawRemoved(u32),
    Button(u32, Button, bool),
    RawButton(u32, u8, bool),
    Axis(u32, Axis, i16),
    RawAxis(u32, u8, i16),
    Hat(u32, u8),
}

impl Signal {
    fn from_event(event: Event) -> Option<Self> {
        Some(match event {
            Event::ControllerDeviceAdded { which, .. } => Self::Added(which),
            Event::JoyDeviceAdded { which, .. } => Self::RawAdded(which),
            Event::ControllerDeviceRemoved { which, .. } => Self::Removed(which),
            Event::JoyDeviceRemoved { which, .. } => Self::RawRemoved(which),
            Event::ControllerButtonDown { which, button, .. } => Self::Button(which, button, true),
            Event::ControllerButtonUp { which, button, .. } => Self::Button(which, button, false),
            Event::JoyButtonDown {
                which, button_idx, ..
            } => Self::RawButton(which, button_idx, true),
            Event::JoyButtonUp {
                which, button_idx, ..
            } => Self::RawButton(which, button_idx, false),
            Event::ControllerAxisMotion {
                which, axis, value, ..
            } => Self::Axis(which, axis, value),
            Event::JoyAxisMotion {
                which,
                axis_idx,
                value,
                ..
            } => Self::RawAxis(which, axis_idx, value),
            Event::JoyHatMotion {
                which,
                hat_idx: 0,
                state,
                ..
            } => Self::Hat(which, state.to_raw()),
            _ => return None,
        })
    }
}

fn require(condition: bool, diagnostic: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(diagnostic.into())
    }
}

fn sdl_result(result: i32) -> Result<(), String> {
    if result < 0 {
        Err(sdl2::get_error())
    } else {
        Ok(())
    }
}

// This owns only a device created by this harness. Re-resolve the index by instance
// before detaching: a device index is not stable across hotplug.
struct VirtualDevice {
    instance: u32,
    attached: bool,
}

impl VirtualDevice {
    fn detach(&mut self) -> Result<(), String> {
        if self.attached {
            // SAFETY: worker thread owns the initialized joystick subsystem.
            let index = unsafe {
                (0..sys::SDL_NumJoysticks()).find(|&index| {
                    sys::SDL_JoystickGetDeviceInstanceID(index) == self.instance as i32
                })
            }
            .ok_or("fixture device disappeared before detach")?;
            // SAFETY: index was resolved to our attached virtual fixture above.
            sdl_result(unsafe { sys::SDL_JoystickDetachVirtual(index) })?;
            self.attached = false;
        }
        Ok(())
    }
}

impl Drop for VirtualDevice {
    fn drop(&mut self) {
        // Best-effort error cleanup; normal paths call detach() and check errors.
        let _ = self.detach();
    }
}

struct Fixture {
    // Field drop order closes handles before detaching the virtual device.
    controller: GameController,
    device: VirtualDevice,
    index_at_attach: u32,
    guid: String,
}

impl Fixture {
    fn attach(
        joysticks: &JoystickSubsystem,
        controllers: &GameControllerSubsystem,
    ) -> Result<Self, String> {
        // A fixture-only VID/PID gives SDL's GUID matcher its standard form.
        // The legacy no-VID/PID virtual GUID hits a CRC mismatch in bundled SDL.
        // SAFETY: SDL documents zero-initialization, then setting version/fields.
        let mut description: sys::SDL_VirtualJoystickDesc = unsafe { std::mem::zeroed() };
        description.version = sys::SDL_VIRTUAL_JOYSTICK_DESC_VERSION as u16;
        description.type_ = sys::SDL_JoystickType::SDL_JOYSTICK_TYPE_UNKNOWN as u16;
        description.naxes = 6;
        description.nbuttons = 2;
        description.nhats = 1;
        description.vendor_id = 0xf001;
        description.product_id = 0xf002;
        // SAFETY: valid versioned descriptor; initialized subsystem on owner thread.
        let index = unsafe { sys::SDL_JoystickAttachVirtualEx(&description) };
        sdl_result(index)?;
        // SAFETY: successful attach returned this live device index.
        let instance = unsafe { sys::SDL_JoystickGetDeviceInstanceID(index) };
        let device = VirtualDevice {
            instance: instance as u32,
            attached: true,
        };
        require(instance >= 0, "new virtual device has no instance ID")?;
        // Do not open a second raw Joystick: sdl2 0.38.0's Joystick::drop skips
        // SDL_JoystickClose when detached. Borrow the controller-owned handle below.
        let device_guid = joysticks
            .device_guid(index as u32)
            .map_err(|e| e.to_string())?;
        let guid = device_guid.to_string();
        // Explicit synthetic mapping only. Never infer an unknown physical layout.
        controllers.add_mapping(&format!(
            "{guid},Fusion spike virtual fixture,a:b0,b:b1,dpup:h0.1,dpright:h0.2,dpdown:h0.4,dpleft:h0.8,leftx:a0,lefty:a1,rightx:a2,righty:a3,lefttrigger:a4,righttrigger:a5,platform:Windows,"
        )).map_err(|e| e.to_string())?;
        require(
            controllers.is_game_controller(index as u32),
            &format!(
                "fixture mapping not recognized: guid={guid}, roundtrip={}, lookup={:?}",
                sdl2::joystick::Guid::from_string(&guid).map_err(|e| e.to_string())?,
                controllers.mapping_for_guid(device_guid)
            ),
        )?;
        let controller = controllers.open(index as u32).map_err(|e| e.to_string())?;
        require(
            controller.instance_id() == device.instance,
            "controller/raw instance mismatch",
        )?;
        Ok(Self {
            controller,
            device,
            index_at_attach: index as u32,
            guid,
        })
    }

    fn raw(&self) -> Result<*mut sys::SDL_Joystick, String> {
        // SAFETY: borrowed lookup only, while self.controller owns its joystick.
        let raw = unsafe { sys::SDL_JoystickFromInstanceID(self.device.instance as i32) };
        require(!raw.is_null(), "fixture joystick handle is missing")?;
        Ok(raw)
    }

    fn button(&self, value: bool) -> Result<(), String> {
        // SAFETY: live virtual joystick, same worker; fixed button 0 is in bounds.
        sdl_result(unsafe { sys::SDL_JoystickSetVirtualButton(self.raw()?, 0, u8::from(value)) })
    }

    fn axis(&self, index: u8, value: i16) -> Result<(), String> {
        // SAFETY: all callers use the six axes configured at attach, on the owner thread.
        sdl_result(unsafe { sys::SDL_JoystickSetVirtualAxis(self.raw()?, index.into(), value) })
    }

    fn hat(&self, value: u8) -> Result<(), String> {
        // SAFETY: fixture has one hat and callers use valid SDL hat bitmasks.
        sdl_result(unsafe { sys::SDL_JoystickSetVirtualHat(self.raw()?, 0, value) })
    }
}

fn await_signals(
    pump: &mut EventPump,
    report: &mut Report,
    expected: &[Signal],
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut seen = Vec::new();
    loop {
        // Real SDL_PollEvent path, not SDL_PushEvent or a mocked provider.
        for event in pump.poll_iter() {
            if let Some(signal) = Signal::from_event(event) {
                let relevant = match &signal {
                    Signal::Added(index) | Signal::RawAdded(index) => {
                        expected.contains(&Signal::Added(*index))
                            || expected.contains(&Signal::RawAdded(*index))
                    }
                    Signal::Removed(id)
                    | Signal::RawRemoved(id)
                    | Signal::Button(id, ..)
                    | Signal::RawButton(id, ..)
                    | Signal::Axis(id, ..)
                    | Signal::RawAxis(id, ..)
                    | Signal::Hat(id, ..) => report.instances.contains(id),
                };
                if relevant {
                    report.polled_events.push(format!("{signal:?}"));
                    seen.push(signal);
                }
            }
        }
        if expected.iter().all(|signal| seen.contains(signal)) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "SDL event deadline: expected {expected:?}, observed {seen:?}"
            ));
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn handle_is_open(instance: u32) -> bool {
    // SAFETY: queried only on the owner thread before subsystem teardown; no dereference.
    unsafe { !sys::SDL_JoystickFromInstanceID(instance as i32).is_null() }
}

enum Reply {
    Ready,
    Released(Report),
    Failed(String),
}

enum Command {
    Stop,
}

fn native_fixture(commands: Receiver<Command>, replies: &Sender<Reply>) -> Result<Report, String> {
    require(cfg!(target_os = "windows"), "this spike requires Windows")?;
    // Every context, pump and handle is created and destroyed on this worker.
    let sdl = sdl2::init()?;
    let joysticks = sdl.joystick()?;
    let controllers = sdl.game_controller()?;
    joysticks.set_event_state(true);
    controllers.set_event_state(true);
    let mut pump = sdl.event_pump()?;
    let mut report = Report {
        worker_thread: format!("{:?}", thread::current().id()),
        sdl_version: sdl2::version::version().to_string(),
        initial_joystick_count: joysticks.num_joysticks()?,
        fixture_guid: String::new(),
        instances: Vec::new(),
        polled_events: Vec::new(),
        stages: vec!["context-created"],
        initialized_flags_after_drop: u32::MAX,
    };
    // Discard pre-existing device announcements, never open physical controllers.
    for _ in pump.poll_iter() {}
    let mut first = Fixture::attach(&joysticks, &controllers)?;
    let id = first.device.instance;
    report.instances.push(id);
    report.fixture_guid.clone_from(&first.guid);
    // First attach precedes our explicit mapping registration. SDL's event watch
    // classifies the joystick at attach time; adding a mapping does not replay
    // ControllerDeviceAdded. Reconnect below must emit both added event types.
    await_signals(
        &mut pump,
        &mut report,
        &[Signal::RawAdded(first.index_at_attach)],
    )?;

    for pressed in [true, false] {
        first.button(pressed)?;
        await_signals(
            &mut pump,
            &mut report,
            &[
                Signal::RawButton(id, 0, pressed),
                Signal::Button(id, Button::A, pressed),
            ],
        )?;
        require(
            first.controller.button(Button::A) == pressed,
            "logical button state mismatch",
        )?;
        // SAFETY: live borrowed joystick; fixture button 0 is in bounds.
        let raw_pressed = unsafe { sys::SDL_JoystickGetButton(first.raw()?, 0) } != 0;
        require(raw_pressed == pressed, "raw button state mismatch")?;
    }
    for (hat, expected) in [
        (
            1,
            vec![Signal::Hat(id, 1), Signal::Button(id, Button::DPadUp, true)],
        ),
        (
            3,
            vec![
                Signal::Hat(id, 3),
                Signal::Button(id, Button::DPadRight, true),
            ],
        ),
        (
            0,
            vec![
                Signal::Hat(id, 0),
                Signal::Button(id, Button::DPadUp, false),
                Signal::Button(id, Button::DPadRight, false),
            ],
        ),
    ] {
        first.hat(hat)?;
        await_signals(&mut pump, &mut report, &expected)?;
        // SAFETY: live borrowed joystick; fixture hat 0 is in bounds.
        let raw_hat = unsafe { sys::SDL_JoystickGetHat(first.raw()?, 0) };
        require(raw_hat == hat, "raw hat state mismatch")?;
    }
    for (index, axis) in [
        (0, Axis::LeftX),
        (1, Axis::LeftY),
        (2, Axis::RightX),
        (3, Axis::RightY),
    ] {
        for value in [i16::MIN, i16::MAX, 0] {
            first.axis(index, value)?;
            await_signals(
                &mut pump,
                &mut report,
                &[
                    Signal::RawAxis(id, index, value),
                    Signal::Axis(id, axis, value),
                ],
            )?;
            require(
                first.controller.axis(axis) == value,
                "logical stick state mismatch",
            )?;
        }
    }
    // SDL virtual raw triggers span -32768..32767; GameController maps to 0..32767.
    for (index, axis) in [(4, Axis::TriggerLeft), (5, Axis::TriggerRight)] {
        first.axis(index, i16::MIN)?;
        await_signals(
            &mut pump,
            &mut report,
            &[Signal::RawAxis(id, index, i16::MIN)],
        )?;
        require(
            first.controller.axis(axis) == 0,
            "trigger at rest must map to zero",
        )?;
        for (raw, logical) in [(i16::MAX, i16::MAX), (i16::MIN, 0)] {
            first.axis(index, raw)?;
            await_signals(
                &mut pump,
                &mut report,
                &[
                    Signal::RawAxis(id, index, raw),
                    Signal::Axis(id, axis, logical),
                ],
            )?;
            require(
                first.controller.axis(axis) == logical,
                "logical trigger state mismatch",
            )?;
        }
    }
    first.device.detach()?;
    await_signals(
        &mut pump,
        &mut report,
        &[Signal::RawRemoved(id), Signal::Removed(id)],
    )?;
    require(
        !first.controller.attached(),
        "detached fixture still reports attached",
    )?;
    drop(first);
    require(!handle_is_open(id), "disconnected fixture handle leaked")?;

    let mut second = Fixture::attach(&joysticks, &controllers)?;
    let reconnected = second.device.instance;
    require(reconnected != id, "reconnect reused a session instance ID")?;
    require(
        second.guid == report.fixture_guid,
        "identical virtual hardware changed GUID",
    )?;
    report.instances.push(reconnected);
    await_signals(
        &mut pump,
        &mut report,
        &[
            Signal::RawAdded(second.index_at_attach),
            Signal::Added(second.index_at_attach),
        ],
    )?;
    second.button(true)?;
    await_signals(
        &mut pump,
        &mut report,
        &[
            Signal::RawButton(reconnected, 0, true),
            Signal::Button(reconnected, Button::A, true),
        ],
    )?;

    require(
        second.controller.attached() && handle_is_open(reconnected),
        "ready without live controller handles",
    )?;
    report.stages.push("ready-with-live-handles");
    replies.send(Reply::Ready).map_err(|e| e.to_string())?;
    commands
        .recv_timeout(Duration::from_secs(5))
        .map_err(|e| format!("stop was not received: {e}"))?;
    report.stages.push("stop-received");
    second.device.detach()?;
    // A stop acknowledgement does not rely on a timeout or an attached=false flag.
    drop(second);
    require(
        report.instances.iter().all(|id| !handle_is_open(*id)),
        "stop left a native joystick handle open",
    )?;
    report.stages.push("handles-closed-and-lookups-null");
    drop(pump);
    drop(controllers);
    drop(joysticks);
    drop(sdl);
    // All SDL-owning values have now been dropped; no value escapes this function.
    Ok(report)
}

/// Execute real virtual-device fixtures, request stop, then wait for release AND join.
/// No physical-hardware, Eden, Tauri IPC or production-service success is implied.
pub fn run_virtual_fixture() -> Result<Report, String> {
    let (commands, command_rx) = mpsc::channel();
    let (reply_tx, replies) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("creator-sdl-spike".into())
        .spawn(move || {
            let outcome = native_fixture(command_rx, &reply_tx);
            // SAFETY: read-only SDL status, on the owner thread, AFTER native_fixture's
            // complete stack (including error paths) has unwound normally.
            let flags = unsafe { sys::SDL_WasInit(0) };
            let reply = match outcome {
                Ok(mut report) if flags == 0 => {
                    report.initialized_flags_after_drop = flags;
                    report
                        .stages
                        .extend(["context-dropped-flags-zero", "stop-acknowledged"]);
                    Reply::Released(report)
                }
                Ok(_) => Reply::Failed(format!("SDL still initialized after drop: {flags:#x}")),
                Err(error) => Reply::Failed(format!("{error}; SDL flags after cleanup={flags:#x}")),
            };
            let _ = reply_tx.send(reply);
        })
        .map_err(|e| e.to_string())?;
    let result = (|| {
        match replies
            .recv_timeout(Duration::from_secs(30))
            .map_err(|e| e.to_string())?
        {
            Reply::Ready => {}
            Reply::Failed(error) => return Err(error),
            Reply::Released(_) => return Err("release acknowledged before stop".into()),
        }
        require(
            matches!(replies.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "unexpected reply before stop command",
        )?;
        commands.send(Command::Stop).map_err(|e| e.to_string())?;
        match replies
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| format!("NO release proof: {e}"))?
        {
            Reply::Released(report) => Ok(report),
            Reply::Failed(error) => Err(error),
            Reply::Ready => Err("duplicate ready reply".into()),
        }
    })();
    // On failure disconnect commands so a worker waiting for stop cannot remain orphaned.
    drop(commands);
    worker
        .join()
        .map_err(|_| "SDL worker panicked; no release acknowledgement".to_owned())?;
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn virtual_controller_round_trip_and_worker_release() {
        let report = super::run_virtual_fixture()
            .expect("native SDL fixture must finish and release resources");
        println!("{report:#?}");
    }
}
