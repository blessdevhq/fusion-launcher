//! Windows SDL feasibility spike, not the Creator input IPC implementation.
//! Native objects never leave their worker. Only owned commands and values cross
//! threads. Virtual hardware exists exclusively in cfg(test).
use sdl2::controller::{Axis, Button, GameController};
use sdl2::{sys, GameControllerSubsystem, JoystickSubsystem};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[cfg(test)]
mod fixture;
#[cfg(test)]
mod tests;

static WORKER_OWNED: AtomicBool = AtomicBool::new(false);
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);

struct Lease;
impl Lease {
    fn acquire() -> Result<Self, String> {
        WORKER_OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| "SDL input worker already owned".into())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        WORKER_OWNED.store(false, Ordering::Release);
    }
}

#[derive(Debug, Serialize)]
pub struct DeviceInfo {
    /// Ephemeral instance, NOT a device index, GUID or portable profile identity.
    pub instance_id: u32,
    pub name: String,
    pub is_game_controller: bool,
}

#[derive(Debug, Serialize)]
pub struct RawState {
    pub buttons: Vec<bool>,
    pub axes: Vec<i16>,
    pub hats: Vec<u8>,
}

#[derive(Debug, Serialize)]
pub struct ControllerState {
    pub instance_id: u32,
    pub buttons: BTreeMap<String, bool>,
    /// SDL LeftX, LeftY, RightX, RightY; untouched signed values, no calibration.
    pub sticks: [i16; 4],
    /// SDL logical trigger range 0..32767, not the raw joystick axis range.
    pub triggers: [u16; 2],
    pub raw: RawState,
}

#[derive(Debug, Serialize)]
pub struct ReleaseProof {
    pub handles_closed: bool,
    pub sdl_initialized_flags: u32,
}

#[derive(Debug, Serialize)]
pub struct ProbeReport {
    pub sdl_version: String,
    pub worker_thread: String,
    pub devices: Vec<DeviceInfo>,
    pub release: ReleaseProof,
}

struct Metadata {
    sdl_version: String,
    worker_thread: String,
}
enum Operation {
    Devices,
    Open(u32),
    Read,
    Stop,
    #[cfg(test)]
    Fixture(fixture::Action),
}
enum Reply {
    Devices(Vec<DeviceInfo>),
    Opened(u32),
    State(ControllerState),
    Released(ReleaseProof),
}
type ReplySender = Sender<Result<Reply, String>>;
struct Request {
    operation: Operation,
    reply: ReplySender,
}

pub struct InputWorker {
    commands: Option<Sender<Request>>,
    worker: Option<JoinHandle<()>>,
    metadata: Metadata,
}

impl InputWorker {
    pub fn start() -> Result<Self, String> {
        let lease = Lease::acquire()?;
        let (commands, receiver) = mpsc::channel();
        let (ready, initialized) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("fusion-input-sdl".into())
            .spawn(move || {
                let _lease = lease;
                let outcome = serve(receiver, &ready);
                // serve's SDL stack has fully dropped, including error paths.
                // SAFETY: read-only query on the owner thread after all context drops.
                let flags = unsafe { sys::SDL_WasInit(0) };
                match outcome {
                    Ok(Some(reply)) => {
                        let result = if flags == 0 {
                            Ok(Reply::Released(ReleaseProof {
                                handles_closed: true,
                                sdl_initialized_flags: flags,
                            }))
                        } else {
                            Err(format!(
                                "SDL release unconfirmed: initialized flags {flags:#x}"
                            ))
                        };
                        let _ = reply.send(result);
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let _ = ready.send(Err(format!("{error}; cleanup flags={flags:#x}")));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let mut client = Self {
            commands: Some(commands),
            worker: Some(worker),
            metadata: Metadata {
                sdl_version: String::new(),
                worker_thread: String::new(),
            },
        };
        client.metadata = initialized
            .recv_timeout(REPLY_TIMEOUT)
            .map_err(|e| format!("SDL worker initialization not acknowledged: {e}"))??;
        Ok(client)
    }

    fn call(&self, operation: Operation) -> Result<Reply, String> {
        let (reply, receive) = mpsc::channel();
        self.commands
            .as_ref()
            .ok_or("SDL worker stopped")?
            .send(Request { operation, reply })
            .map_err(|e| e.to_string())?;
        receive
            .recv_timeout(REPLY_TIMEOUT)
            .map_err(|e| format!("SDL operation NOT acknowledged: {e}"))?
    }

    pub fn devices(&self) -> Result<Vec<DeviceInfo>, String> {
        match self.call(Operation::Devices)? {
            Reply::Devices(v) => Ok(v),
            _ => Err("invalid devices reply".into()),
        }
    }
    pub fn open(&self, instance_id: u32) -> Result<u32, String> {
        match self.call(Operation::Open(instance_id))? {
            Reply::Opened(id) => Ok(id),
            _ => Err("invalid open reply".into()),
        }
    }
    pub fn read(&self) -> Result<ControllerState, String> {
        match self.call(Operation::Read)? {
            Reply::State(v) => Ok(v),
            _ => Err("invalid read reply".into()),
        }
    }
    /// Success requires checked handle closure, SDL stack teardown, explicit
    /// worker acknowledgement AND join. A timeout is never release evidence.
    pub fn stop(mut self) -> Result<ReleaseProof, String> {
        let result = self.call(Operation::Stop);
        self.join()?;
        match result? {
            Reply::Released(proof) => Ok(proof),
            _ => Err("invalid release reply".into()),
        }
    }
    fn join(&mut self) -> Result<(), String> {
        // Disconnect before join; a worker whose client vanished must exit.
        self.commands.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "SDL worker panicked; release unconfirmed".to_string())?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn fixture_attach(&self) -> Result<u32, String> {
        match self.call(Operation::Fixture(fixture::Action::Attach))? {
            Reply::Devices(v) => v
                .first()
                .map(|d| d.instance_id)
                .ok_or("missing fixture ID".into()),
            _ => Err("invalid fixture attach reply".into()),
        }
    }
    #[cfg(test)]
    fn fixture_action(&self, action: fixture::Action) -> Result<(), String> {
        match self.call(Operation::Fixture(action))? {
            Reply::Opened(_) => Ok(()),
            _ => Err("invalid fixture reply".into()),
        }
    }
    #[cfg(test)]
    fn fixture_button(&self, index: u8, value: bool) -> Result<(), String> {
        self.fixture_action(fixture::Action::Button(index, value))
    }
    #[cfg(test)]
    fn fixture_axis(&self, index: u8, value: i16) -> Result<(), String> {
        self.fixture_action(fixture::Action::Axis(index, value))
    }
    #[cfg(test)]
    fn fixture_hat(&self, value: u8) -> Result<(), String> {
        self.fixture_action(fixture::Action::Hat(value))
    }
    #[cfg(test)]
    fn fixture_detach(&self) -> Result<(), String> {
        self.fixture_action(fixture::Action::Detach)
    }
}
impl Drop for InputWorker {
    fn drop(&mut self) {
        let _ = self.join();
    }
}

fn instances(
    joysticks: &JoystickSubsystem,
    controllers: &GameControllerSubsystem,
) -> Result<Vec<(u32, DeviceInfo)>, String> {
    (0..joysticks.num_joysticks()?)
        .map(|index| {
            // SAFETY: live subsystem, index within the just-enumerated range. Hotplug
            // may invalidate it; the negative return is handled as an error.
            let id = unsafe { sys::SDL_JoystickGetDeviceInstanceID(index as i32) };
            if id < 0 {
                return Err(sdl2::get_error());
            }
            Ok((
                index,
                DeviceInfo {
                    instance_id: id as u32,
                    name: joysticks.name_for_index(index).map_err(|e| e.to_string())?,
                    is_game_controller: controllers.is_game_controller(index),
                },
            ))
        })
        .collect()
}

fn snapshot(controller: &GameController) -> Result<ControllerState, String> {
    if !controller.attached() {
        return Err("selected controller disconnected; select a fresh instance".into());
    }
    let id = controller.instance_id();
    // SAFETY: borrowed controller-owned joystick. No additional SDL open/ref.
    let raw = unsafe { sys::SDL_JoystickFromInstanceID(id as i32) };
    if raw.is_null() {
        return Err("selected controller has no live joystick handle".into());
    }
    // SAFETY: borrowed live joystick on owner thread; no pointer escapes.
    let (buttons, axes, hats) = unsafe {
        (
            sys::SDL_JoystickNumButtons(raw),
            sys::SDL_JoystickNumAxes(raw),
            sys::SDL_JoystickNumHats(raw),
        )
    };
    if buttons < 0 || axes < 0 || hats < 0 {
        return Err(sdl2::get_error());
    }
    let raw_state = RawState {
        // SAFETY: bounds come from that same live joystick; all reads stay on owner thread.
        buttons: (0..buttons)
            .map(|i| unsafe { sys::SDL_JoystickGetButton(raw, i) != 0 })
            .collect(),
        // SAFETY: as above, validated axis bounds.
        axes: (0..axes)
            .map(|i| unsafe { sys::SDL_JoystickGetAxis(raw, i) })
            .collect(),
        // SAFETY: as above, validated hat bounds.
        hats: (0..hats)
            .map(|i| unsafe { sys::SDL_JoystickGetHat(raw, i) })
            .collect(),
    };
    let logical_buttons = [
        Button::A,
        Button::B,
        Button::X,
        Button::Y,
        Button::Back,
        Button::Guide,
        Button::Start,
        Button::LeftStick,
        Button::RightStick,
        Button::LeftShoulder,
        Button::RightShoulder,
        Button::DPadUp,
        Button::DPadDown,
        Button::DPadLeft,
        Button::DPadRight,
        Button::Misc1,
        Button::Paddle1,
        Button::Paddle2,
        Button::Paddle3,
        Button::Paddle4,
        Button::Touchpad,
    ];
    Ok(ControllerState {
        instance_id: id,
        buttons: logical_buttons
            .into_iter()
            .map(|b| (b.string(), controller.button(b)))
            .collect(),
        sticks: [Axis::LeftX, Axis::LeftY, Axis::RightX, Axis::RightY].map(|a| controller.axis(a)),
        triggers: [Axis::TriggerLeft, Axis::TriggerRight]
            .map(|a| {
                u16::try_from(controller.axis(a))
                    .map_err(|_| "SDL logical trigger outside 0..32767".to_string())
            })
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| "invalid trigger count")?,
        raw: raw_state,
    })
}

/// Everything owning SDL is confined to this stack. Returning a Stop reply
/// sender transfers only acknowledgement responsibility AFTER this stack drops.
fn serve(
    receiver: Receiver<Request>,
    ready: &Sender<Result<Metadata, String>>,
) -> Result<Option<ReplySender>, String> {
    let sdl = sdl2::init()?;
    let joysticks = sdl.joystick()?;
    let controllers = sdl.game_controller()?;
    let mut pump = sdl.event_pump()?;
    let mut selected: Option<GameController> = None;
    #[cfg(test)]
    let mut fixture = None;
    ready
        .send(Ok(Metadata {
            sdl_version: sdl2::version::version().to_string(),
            worker_thread: format!("{:?}", thread::current().id()),
        }))
        .map_err(|e| e.to_string())?;
    loop {
        // Drain rather than accumulate events. Public API returns polled state;
        // this spike does not emit production IPC events or synthesize shell keys.
        for _ in pump.poll_iter() {}
        controllers.update();
        let request = match receiver.recv_timeout(Duration::from_millis(8)) {
            Ok(request) => request,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
        };
        if matches!(request.operation, Operation::Stop) {
            let id = selected.as_ref().map(|c| c.instance_id());
            drop(selected.take());
            // SAFETY: lookup only, before subsystem teardown, on the owner thread.
            if id.is_some_and(|id| unsafe { !sys::SDL_JoystickFromInstanceID(id as i32).is_null() })
            {
                let _ = request
                    .reply
                    .send(Err("controller handle closure NOT confirmed".into()));
                return Ok(None);
            }
            return Ok(Some(request.reply));
        }
        let result = match request.operation {
            Operation::Devices => instances(&joysticks, &controllers)
                .map(|v| Reply::Devices(v.into_iter().map(|(_, d)| d).collect())),
            Operation::Open(id) => (|| {
                if selected.is_some() {
                    return Err("a controller is already selected".into());
                }
                let (index, info) = instances(&joysticks, &controllers)?
                    .into_iter()
                    .find(|(_, d)| d.instance_id == id)
                    .ok_or("controller instance is no longer present")?;
                if !info.is_game_controller {
                    return Err("SDL has no GameController mapping for this device".into());
                }
                let controller = controllers.open(index).map_err(|e| e.to_string())?;
                if controller.instance_id() != id || !controller.attached() {
                    return Err("controller changed during open".into());
                }
                selected = Some(controller);
                Ok(Reply::Opened(id))
            })(),
            Operation::Read => selected
                .as_ref()
                .ok_or("no controller selected".to_string())
                .and_then(snapshot)
                .map(Reply::State),
            #[cfg(test)]
            Operation::Fixture(action) => fixture::apply(
                &mut fixture,
                action,
                &joysticks,
                &controllers,
                selected.as_ref().map(|c| c.instance_id()),
            ),
            Operation::Stop => unreachable!("stop handled before dispatch"),
        };
        let _ = request.reply.send(result);
    }
}

/// Native application diagnostic: enumerate without auto-opening a physical
/// device, then prove teardown. No UI, calibration, job/IPC or Eden behavior.
pub fn probe() -> Result<ProbeReport, String> {
    let worker = InputWorker::start()?;
    let sdl_version = worker.metadata.sdl_version.clone();
    let worker_thread = worker.metadata.worker_thread.clone();
    let devices = worker.devices();
    let release = worker.stop()?;
    Ok(ProbeReport {
        sdl_version,
        worker_thread,
        devices: devices?,
        release,
    })
}

/// Explicit diagnostic path in the real application binary. Ordinary startup
/// does not initialize SDL, open a controller or interfere with shell input.
pub(crate) fn run_probe_if_requested() -> bool {
    if !std::env::args_os().any(|arg| arg == "--input-spike-probe") {
        return false;
    }
    match probe().and_then(|report| serde_json::to_string(&report).map_err(|e| e.to_string())) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("native SDL input probe failed: {error}");
            std::process::exit(1);
        }
    }
    true
}
