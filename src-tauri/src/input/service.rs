//! Production input service. SDL values travel through one native owner thread.
//! All identities, mapping sources and calibration are PRIVATE local data.
use super::{eden, ControllerState, DeviceInfo, InputWorker};
use crate::fusion_creator::input::*;
use crate::fusion_source::creator_schema::LogicalBinding;
use crate::fusion_source::errors::{ErrorCode, FusionError};
use crate::fusion_source::fs_safety;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

const EVENT_INTERVAL: Duration = Duration::from_nanos(33_333_334);
const MAX_SEQUENCE: u64 = (1u64 << 53) - 1;
const MAX_BINDINGS: usize = 256;
const MAX_LOCAL_BYTES: u64 = 256 * 1024;
type Sink = Arc<dyn Fn(InputState) + Send + Sync>;

fn unavailable(message: impl Into<String>) -> FusionError {
    FusionError::new(ErrorCode::ActionUnavailable, message)
}
fn invalid(message: impl Into<String>) -> FusionError {
    FusionError::new(ErrorCode::InvalidManifest, message)
}
fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, FusionError> {
    mutex
        .lock()
        .map_err(|_| unavailable("Input service lock poisoned; restart Fusion."))
}
fn token(prefix: &str) -> String {
    format!("{prefix}-{:032x}", rand::random::<u128>())
}
fn valid_id(value: &str) -> Result<(), FusionError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(FusionError::new(
            ErrorCode::InvalidId,
            "Invalid local controls/action ID.",
        ));
    }
    Ok(())
}

/// Internal read-only data for D. NOT a wire DTO or portable ControlsProfile.
/// No SDL references. A persisted view alone never proves physical selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateBindingView {
    pub schema_version: u32,
    pub controls_profile_id: String,
    pub binding_revision: String,
    pub session_id: String,
    pub instance_id: u32,
    pub identity: InputPrivateIdentity,
    pub bindings: BTreeMap<String, LogicalBinding>,
    pub raw_bindings: BTreeMap<String, RawInputBinding>,
    pub deadzones: BTreeMap<String, f64>,
    /// Retain half-axis/inversion/range semantics for the D writer.
    pub sdl_mapping: String,
    pub driver_pin: String,
    pub topology: Vec<u32>,
}
impl PrivateBindingView {
    /// C/D local cache stamp; never include this private fingerprint in exports.
    pub fn fingerprint(&self) -> Result<String, FusionError> {
        let bytes = serde_json::to_vec(self).map_err(|e| invalid(e.to_string()))?;
        Ok(blake3::hash(&bytes).to_hex().to_string())
    }
}

#[derive(Default)]
struct Runtime {
    worker: Option<InputWorker>,
    catalog: BTreeMap<String, DeviceInfo>,
    /// A timeout/panic/failed closure is sticky: absence of a client is NOT proof.
    release_error: Option<FusionError>,
}

pub(super) struct Session {
    pub(super) state: InputState,
    device: DeviceInfo,
    topology: Vec<u32>,
    mapping: String,
    deadzones: BTreeMap<String, f64>,
    last_emit: Instant,
    release_confirmed: bool,
}
impl Session {
    fn next(&mut self) -> Result<(), FusionError> {
        if self.state.sequence >= MAX_SEQUENCE {
            return Err(unavailable(
                "Input sequence exhausted; restart and select a new session.",
            ));
        }
        self.state.sequence += 1;
        Ok(())
    }
    pub(super) fn observe(
        &mut self,
        id: &str,
        observation: Result<ControllerState, String>,
        now: Instant,
    ) -> Option<InputState> {
        if self.state.session_id != id || self.state.status != InputSessionStatus::Active {
            return None;
        }
        match observation {
            Ok(value) => {
                if value.instance_id != self.device.instance_id
                    || now.duration_since(self.last_emit) < EVENT_INTERVAL
                {
                    return None;
                }
                self.state.buttons = value.buttons;
                self.state.axes = eden::AXES
                    .into_iter()
                    .enumerate()
                    .map(|(index, name)| {
                        let value = if index < 4 {
                            let raw = value.sticks[index];
                            f64::from(raw) / if raw < 0 { 32768.0 } else { 32767.0 }
                        } else {
                            f64::from(value.triggers[index - 4]) / 32767.0
                        };
                        (
                            name.to_owned(),
                            eden::calibrate(value, *self.deadzones.get(name).unwrap_or(&0.0)),
                        )
                    })
                    .collect();
            }
            Err(message) => {
                self.state.status = InputSessionStatus::Disconnected;
                self.state.buttons.clear();
                self.state.axes.clear();
                self.state.error =
                    Some(unavailable(message).with_detail("reason", "reselect_device"));
            }
        }
        if self.state.sequence >= MAX_SEQUENCE - 1 {
            self.state.status = InputSessionStatus::Failed;
            self.state.error = Some(unavailable(
                "Input sequence exhausted; select a new session.",
            ));
        }
        if self.next().is_err() {
            return None;
        }
        self.last_emit = now;
        Some(self.state.clone())
    }
    #[cfg(test)]
    pub(super) fn fixture(id: &str, now: Instant) -> Self {
        Self {
            state: InputState {
                session_id: id.into(),
                sequence: 1,
                device_id: Some("opaque".into()),
                status: InputSessionStatus::Active,
                buttons: BTreeMap::new(),
                axes: BTreeMap::new(),
                error: None,
            },
            device: DeviceInfo {
                instance_id: 1,
                name: "fixture".into(),
                is_game_controller: true,
                guid: "0".repeat(32),
                port: 0,
            },
            topology: vec![1],
            mapping: String::new(),
            deadzones: BTreeMap::new(),
            last_emit: now,
            release_confirmed: false,
        }
    }
}

pub struct InputService {
    runtime: Mutex<Runtime>,
    session: Arc<Mutex<Option<Session>>>,
    local_root: PathBuf,
    sink: Sink,
    confirmed_bindings: Mutex<BTreeMap<String, String>>,
}
impl InputService {
    /// Lazy SDL initialization. Creating the service opens NO devices or SDL context.
    pub fn new(local_root: PathBuf, sink: impl Fn(InputState) + Send + Sync + 'static) -> Self {
        Self {
            runtime: Mutex::new(Runtime::default()),
            session: Arc::new(Mutex::new(None)),
            local_root,
            sink: Arc::new(sink),
            confirmed_bindings: Mutex::new(BTreeMap::new()),
        }
    }
    fn worker(runtime: &mut Runtime) -> Result<&InputWorker, FusionError> {
        if let Some(error) = &runtime.release_error {
            return Err(error.clone());
        }
        if runtime.worker.is_none() {
            runtime.worker = Some(InputWorker::start().map_err(|message| {
                // Failed startup may leave a detached owner; no client is not release proof.
                let error = unavailable(format!(
                    "SDL input startup failed; release unconfirmed, restart Fusion: {message}"
                ));
                runtime.release_error = Some(error.clone());
                error
            })?);
        }
        Ok(runtime.worker.as_ref().expect("worker initialized above"))
    }
    fn current<'a>(
        session: &'a mut Option<Session>,
        id: &str,
    ) -> Result<&'a mut Session, FusionError> {
        session
            .as_mut()
            .filter(|s| s.state.session_id == id)
            .ok_or_else(|| {
                unavailable(
                    "Unknown or stale input session; query current session, do not reuse old IDs.",
                )
            })
    }
    fn refresh(runtime: &mut Runtime) -> Result<Vec<InputDevice>, FusionError> {
        let devices = Self::worker(runtime)?.devices().map_err(unavailable)?;
        // Every discovery requires an explicit fresh token selection. GUID and
        // port are not encoded into public IDs, including for identical devices.
        runtime.catalog.clear();
        Ok(devices
            .into_iter()
            .map(|device| {
                let id = token("input-device");
                let public = InputDevice {
                    device_id: id.clone(),
                    name: device.name.clone(),
                    connected: true,
                    mapping_known: device.is_game_controller,
                };
                runtime.catalog.insert(id, device);
                public
            })
            .collect())
    }
    fn path(&self, profile: &str) -> Result<PathBuf, FusionError> {
        valid_id(profile)?;
        Ok(self.local_root.join(format!(
            "{}.json",
            blake3::hash(profile.as_bytes()).to_hex()
        )))
    }
    /// Private local loading for diagnostics/migration only; NEVER use this as
    /// authority for a current physical selection. D uses private_binding_view.
    pub fn local_binding_view(&self, profile: &str) -> Result<PrivateBindingView, FusionError> {
        let path = self.path(profile)?;
        let file = fs_safety::open_read(&path)?;
        if file
            .metadata()
            .map_err(|e| unavailable(e.to_string()))?
            .len()
            > MAX_LOCAL_BYTES
        {
            return Err(invalid("Local binding file too large."));
        }
        let mut data = Vec::new();
        file.take(MAX_LOCAL_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|e| unavailable(e.to_string()))?;
        if data.len() as u64 > MAX_LOCAL_BYTES {
            return Err(invalid("Local binding file too large."));
        }
        let view: PrivateBindingView =
            serde_json::from_slice(&data).map_err(|e| invalid(e.to_string()))?;
        if view.schema_version != 1 || view.controls_profile_id != profile {
            return Err(invalid("Invalid local binding schema/profile."));
        }
        validate_deadzones(&view.deadzones)?;
        Ok(view)
    }
    /// D: revalidate against the explicitly selected LIVE session and topology.
    /// A fresh session must save/reconfirm bindings; persisted GUID is not selection.
    pub fn private_binding_view(&self, profile: &str) -> Result<PrivateBindingView, FusionError> {
        let mut runtime = lock(&self.runtime)?;
        let view = self.local_binding_view(profile)?;
        if lock(&self.confirmed_bindings)?.get(profile) != Some(&view.fingerprint()?) {
            return Err(unavailable("Local binding changed or was not confirmed in this process; reselect and save again."));
        }
        let session = lock(&self.session)?;
        let current = session
            .as_ref()
            .ok_or_else(|| unavailable("Select a controller before reading private bindings."))?;
        if current.state.session_id != view.session_id
            || current.state.status != InputSessionStatus::Active
            || current.device.instance_id != view.instance_id
        {
            return Err(unavailable(
                "Private binding requires fresh selection and confirmation.",
            ));
        }
        // Never hold session lock while requesting worker: observer needs it.
        drop(session);
        let topology = Self::worker(&mut runtime)?
            .devices()
            .map_err(unavailable)?
            .into_iter()
            .map(|d| d.instance_id)
            .collect::<Vec<_>>();
        if topology != view.topology
            || Self::worker(&mut runtime)?.mapping().map_err(unavailable)? != view.sdl_mapping
        {
            return Err(unavailable(
                "Device topology/mapping changed; select and confirm bindings again.",
            ));
        }
        Ok(view)
    }
    fn release_locked(
        &self,
        runtime: &mut Runtime,
        id: &str,
    ) -> Result<InputReleaseReceipt, FusionError> {
        if let Some(error) = &runtime.release_error {
            return Err(error.clone());
        }
        let (initial_sequence, sequence_result) = {
            let mut state = lock(&self.session)?;
            let session = Self::current(&mut state, id)?;
            if session.release_confirmed && runtime.worker.is_none() {
                return Ok(InputReleaseReceipt {
                    session_id: id.into(),
                    handles_closed: true,
                });
            }
            let sequence = session.state.sequence;
            let result = session.next();
            if !session.release_confirmed {
                session.state.status = InputSessionStatus::Stopping;
            }
            (sequence, result)
        };
        // Always attempt teardown, even if the session sequence is exhausted.
        let mut result = runtime
            .worker
            .take()
            .ok_or_else(|| unavailable("No worker ACK exists; launch blocked."))
            .and_then(|worker| worker.stop().map_err(unavailable))
            .and_then(|proof| {
                if proof.handles_closed && proof.sdl_initialized_flags == 0 {
                    Ok(())
                } else {
                    Err(unavailable("Device release not confirmed; launch blocked."))
                }
            })
            .and(sequence_result);
        runtime.catalog.clear();
        let final_state = {
            let mut state = lock(&self.session)?;
            let session = Self::current(&mut state, id)?;
            if let Err(error) = session.next() {
                result = Err(error);
            }
            session.release_confirmed = result.is_ok();
            session.state.buttons.clear();
            session.state.axes.clear();
            session.state.status = if result.is_ok() {
                InputSessionStatus::Stopped
            } else {
                InputSessionStatus::Failed
            };
            session.state.error = result.as_ref().err().cloned();
            (session.state.sequence > initial_sequence).then(|| session.state.clone())
        };
        if let Some(final_state) = final_state {
            (self.sink)(final_state);
        }
        match result {
            Ok(()) => Ok(InputReleaseReceipt {
                session_id: id.into(),
                handles_closed: true,
            }),
            Err(error) => {
                runtime.release_error = Some(error.clone());
                Err(error)
            }
        }
    }
    /// B/D: mandatory atomic pre-spawn barrier, including application_defaults.
    /// Holds the command gate through launch() so another capture cannot start
    /// between ACK and spawn. Propagate any error; NEVER fallback to launching.
    pub fn with_released_for_launch<T>(
        &self,
        launch: impl FnOnce() -> Result<T, FusionError>,
    ) -> Result<T, FusionError> {
        let mut runtime = lock(&self.runtime)?;
        if let Some(error) = &runtime.release_error {
            return Err(error.clone());
        }
        let id = lock(&self.session)?
            .as_ref()
            .map(|s| s.state.session_id.clone());
        if let Some(id) = id {
            self.release_locked(&mut runtime, &id)?;
        } else if let Some(worker) = runtime.worker.take() {
            match worker.stop().map_err(unavailable) {
                Ok(proof) if proof.handles_closed && proof.sdl_initialized_flags == 0 => {}
                result => {
                    let error = result.err().unwrap_or_else(|| {
                        unavailable("SDL release proof invalid; launch blocked.")
                    });
                    runtime.release_error = Some(error.clone());
                    return Err(error);
                }
            }
            runtime.catalog.clear();
        }
        launch()
    }
}
impl InputProvider for InputService {
    fn devices(&self) -> Result<Vec<InputDevice>, FusionError> {
        Self::refresh(&mut *lock(&self.runtime)?)
    }
    fn start(&self, request: InputSessionStartRequest) -> Result<InputSessionReceipt, FusionError> {
        let mut runtime = lock(&self.runtime)?;
        if lock(&self.session)?
            .as_ref()
            .is_some_and(|s| !s.release_confirmed)
        {
            return Err(unavailable(
                "Stop the previous input session before selecting another controller.",
            ));
        }
        let device = runtime
            .catalog
            .get(&request.device_id)
            .cloned()
            .ok_or_else(|| {
                unavailable("Stale/unknown device token; enumerate and select again.")
            })?;
        if !device.is_game_controller {
            return Err(unavailable(
                "Unknown SDL mapping; no button-number fallback is permitted.",
            ));
        }
        let worker = Self::worker(&mut runtime)?;
        let devices = worker.devices().map_err(unavailable)?;
        if !devices.iter().any(|d| {
            d.instance_id == device.instance_id && d.guid == device.guid && d.port == device.port
        }) {
            return Err(unavailable(
                "Device disconnected/reordered; enumerate and select again.",
            ));
        }
        let opened = (|| {
            worker.open(device.instance_id)?;
            Ok::<_, String>((worker.read()?, worker.mapping()?))
        })();
        let (snapshot, mapping) = match opened {
            Ok(value) => value,
            Err(message) => {
                let cleanup = runtime.worker.take().expect("worker exists").stop();
                runtime.catalog.clear();
                if let Err(error) = cleanup {
                    let error = unavailable(format!(
                        "Input start failed: {message}; release unconfirmed: {error}"
                    ));
                    runtime.release_error = Some(error.clone());
                    return Err(error);
                }
                return Err(unavailable(message));
            }
        };
        let id = token("input-session");
        let now = Instant::now();
        let mut session = Session {
            state: InputState {
                session_id: id.clone(),
                sequence: 1,
                device_id: Some(request.device_id),
                status: InputSessionStatus::Active,
                buttons: BTreeMap::new(),
                axes: BTreeMap::new(),
                error: None,
            },
            device,
            topology: devices.into_iter().map(|d| d.instance_id).collect(),
            mapping,
            deadzones: BTreeMap::new(),
            last_emit: now - EVENT_INTERVAL,
            release_confirmed: false,
        };
        let initial = session
            .observe(&id, Ok(snapshot), now)
            .expect("initial observation");
        *lock(&self.session)? = Some(session);
        (self.sink)(initial);
        let shared = self.session.clone();
        let sink = self.sink.clone();
        let observed_id = id.clone();
        let observing = Self::worker(&mut runtime)?.observe(Arc::new(move |observation| {
            let event = shared.lock().ok().and_then(|mut s| {
                s.as_mut()
                    .and_then(|s| s.observe(&observed_id, observation, Instant::now()))
            });
            if let Some(event) = event {
                sink(event);
            }
        }));
        if let Err(error) = observing {
            self.release_locked(&mut runtime, &id)?;
            return Err(unavailable(error));
        }
        Ok(InputSessionReceipt { session_id: id })
    }
    fn state(&self, request: InputSessionRequest) -> Result<InputState, FusionError> {
        Ok(
            Self::current(&mut *lock(&self.session)?, &request.session_id)?
                .state
                .clone(),
        )
    }
    fn stop(&self, request: InputSessionRequest) -> Result<InputReleaseReceipt, FusionError> {
        self.release_locked(&mut *lock(&self.runtime)?, &request.session_id)
    }
    fn release_for_launch(
        &self,
        request: InputSessionRequest,
    ) -> Result<InputReleaseReceipt, FusionError> {
        self.stop(request)
    }
    fn save_bindings(
        &self,
        request: InputBindingsSaveRequest,
    ) -> Result<InputBindingsReceipt, FusionError> {
        valid_id(&request.controls_profile_id)?;
        validate_deadzones(&request.deadzones)?;
        if request.bindings.is_empty()
            || request.bindings.len() > MAX_BINDINGS
            || request.raw_bindings.len() > MAX_BINDINGS
        {
            return Err(invalid("Expected 1..256 logical bindings."));
        }
        let mut runtime = lock(&self.runtime)?;
        let (device, topology, mapping) = {
            let mut state = lock(&self.session)?;
            let session = Self::current(&mut state, &request.session_id)?;
            if session.state.status != InputSessionStatus::Active {
                return Err(unavailable(
                    "Bindings require an active selected controller.",
                ));
            }
            (
                session.device.clone(),
                session.topology.clone(),
                session.mapping.clone(),
            )
        };
        let worker = Self::worker(&mut runtime)?;
        if worker
            .devices()
            .map_err(unavailable)?
            .iter()
            .map(|d| d.instance_id)
            .collect::<Vec<_>>()
            != topology
            || worker.mapping().map_err(unavailable)? != mapping
        {
            return Err(unavailable(
                "Topology/mapping changed; reselect controller.",
            ));
        }
        let live = worker.read().map_err(unavailable)?;
        let mut raw_bindings = BTreeMap::new();
        for (action, binding) in &request.bindings {
            valid_id(action)?;
            let raw = eden::resolve_raw(&mapping, binding).map_err(unavailable)?;
            let in_range = match &raw {
                RawInputBinding::Button { index } => (*index as usize) < live.raw.buttons.len(),
                RawInputBinding::Hat { index, .. } => (*index as usize) < live.raw.hats.len(),
                RawInputBinding::Axis { index, .. } => (*index as usize) < live.raw.axes.len(),
            };
            if !in_range {
                return Err(invalid("SDL raw binding outside live device range."));
            }
            if request.raw_bindings.get(action).is_some_and(|proposed| {
                serde_json::to_value(proposed).ok() != serde_json::to_value(&raw).ok()
            }) {
                return Err(invalid("Supplied raw binding contradicts SDL mapping."));
            }
            raw_bindings.insert(action.clone(), raw);
        }
        if request
            .raw_bindings
            .keys()
            .any(|action| !request.bindings.contains_key(action))
        {
            return Err(invalid("Raw binding has no logical action."));
        }
        let view = PrivateBindingView {
            schema_version: 1,
            controls_profile_id: request.controls_profile_id.clone(),
            binding_revision: token("binding"),
            session_id: request.session_id.clone(),
            instance_id: device.instance_id,
            identity: InputPrivateIdentity {
                guid: device.guid,
                path: None,
                serial: None,
                port: Some(device.port),
            },
            bindings: request.bindings,
            raw_bindings,
            deadzones: request.deadzones.clone(),
            sdl_mapping: mapping,
            driver_pin: "58c1e20ee58efa3900ba616207d460886214480b".into(),
            topology,
        };
        fs_safety::atomic_write_json(&self.path(&request.controls_profile_id)?, &view)?;
        lock(&self.confirmed_bindings)?
            .insert(request.controls_profile_id.clone(), view.fingerprint()?);
        Self::current(&mut *lock(&self.session)?, &request.session_id)?.deadzones =
            request.deadzones;
        Ok(InputBindingsReceipt {
            session_id: request.session_id,
            controls_profile_id: request.controls_profile_id,
        })
    }
}
fn validate_deadzones(values: &BTreeMap<String, f64>) -> Result<(), FusionError> {
    if values.len() > eden::AXES.len()
        || values.iter().any(|(axis, value)| {
            !eden::AXES.contains(&axis.as_str())
                || !value.is_finite()
                || !(0.0..1.0).contains(value)
        })
    {
        return Err(invalid(
            "Deadzones must name SDL axes and be finite in [0,1).",
        ));
    }
    Ok(())
}

#[cfg(test)]
impl InputService {
    pub(super) fn fixture_fail_read(&self) -> Result<(), FusionError> {
        Self::worker(&mut *lock(&self.runtime)?)?
            .call(super::Operation::FailRead)
            .map_err(unavailable)?;
        Ok(())
    }
    pub(super) fn fixture_sequence_exhaustion(&self) {
        let mut session = self.session.lock().unwrap();
        let session = session.as_mut().unwrap();
        session.state.sequence = MAX_SEQUENCE;
        session.state.status = InputSessionStatus::Failed;
    }
    pub(super) fn fixture_release_fault(
        &self,
        delay: Duration,
        suppress_ack: bool,
    ) -> Result<(), FusionError> {
        let mut runtime = lock(&self.runtime)?;
        Self::worker(&mut runtime)?
            .call(super::Operation::ReleaseFault(delay, suppress_ack))
            .map_err(unavailable)?;
        runtime.worker.as_mut().unwrap().reply_timeout = Duration::from_millis(25);
        Ok(())
    }
    pub(super) fn fixture_attach_identical(&self) -> Result<(), FusionError> {
        Self::worker(&mut *lock(&self.runtime)?)?
            .call(super::Operation::DuplicateFixture)
            .map_err(unavailable)?;
        Ok(())
    }
    pub(super) fn fixture_attach(&self) -> Result<String, FusionError> {
        let mut runtime = lock(&self.runtime)?;
        let id = Self::worker(&mut runtime)?
            .fixture_attach()
            .map_err(unavailable)?;
        Self::refresh(&mut runtime)?;
        Ok(runtime
            .catalog
            .iter()
            .find(|(_, d)| d.instance_id == id)
            .unwrap()
            .0
            .clone())
    }
    pub(super) fn fixture_button(&self, index: u8, value: bool) -> Result<(), FusionError> {
        Self::worker(&mut *lock(&self.runtime)?)?
            .fixture_button(index, value)
            .map_err(unavailable)
    }
    pub(super) fn fixture_axis(&self, index: u8, value: i16) -> Result<(), FusionError> {
        Self::worker(&mut *lock(&self.runtime)?)?
            .fixture_axis(index, value)
            .map_err(unavailable)
    }
    pub(super) fn fixture_hat(&self, value: u8) -> Result<(), FusionError> {
        Self::worker(&mut *lock(&self.runtime)?)?
            .fixture_hat(value)
            .map_err(unavailable)
    }
    pub(super) fn fixture_detach(&self) -> Result<(), FusionError> {
        Self::worker(&mut *lock(&self.runtime)?)?
            .fixture_detach()
            .map_err(unavailable)
    }
    pub(super) fn wait_state(
        &self,
        request: &InputSessionRequest,
        predicate: impl Fn(&InputState) -> bool,
    ) -> InputState {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let state = self.state(request.clone()).unwrap();
            if predicate(&state) {
                return state;
            }
            assert!(Instant::now() < deadline, "state timeout: {state:?}");
            std::thread::sleep(Duration::from_millis(4));
        }
    }
}
