//! Windows sidecars run in a private Job Object, assigned atomically at birth.
//!
//! PyInstaller --onefile has a bootloader and a Python descendant. Observing the
//! bootloader exit alone cannot prove that payload writers have stopped. The job
//! handle is never inherited; closing the launcher kills the entire tree.
//! API contract: https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute

use std::ffi::{c_void, OsStr};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use tauri_plugin_shell::process::{CommandEvent, TerminatedPayload};

type Handle = *mut c_void;

#[repr(C)]
struct SecurityAttributes {
    size: u32,
    descriptor: *mut c_void,
    inherit: i32,
}
#[repr(C)]
struct StartupInfo {
    size: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    chars_x: u32,
    chars_y: u32,
    fill: u32,
    flags: u32,
    show: u16,
    reserved_size: u16,
    reserved_data: *mut u8,
    stdin: Handle,
    stdout: Handle,
    stderr: Handle,
}
#[repr(C)]
struct StartupInfoEx {
    startup: StartupInfo,
    attributes: *mut c_void,
}
#[repr(C)]
struct ProcessInfo {
    process: Handle,
    thread: Handle,
    pid: u32,
    tid: u32,
}
#[repr(C)]
struct BasicLimits {
    process_time: i64,
    job_time: i64,
    flags: u32,
    min_working_set: usize,
    max_working_set: usize,
    active_process_limit: u32,
    affinity: usize,
    priority: u32,
    scheduling: u32,
}
#[repr(C)]
struct ExtendedLimits {
    basic: BasicLimits,
    io: [u64; 6],
    process_memory: usize,
    job_memory: usize,
    peak_process_memory: usize,
    peak_job_memory: usize,
}
#[repr(C)]
#[derive(Default)]
struct Accounting {
    times: [i64; 4],
    faults: u32,
    total: u32,
    active: u32,
    terminated: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn CreateJobObjectW(attributes: *mut c_void, name: *const u16) -> Handle;
    fn SetInformationJobObject(job: Handle, class: u32, data: *const c_void, length: u32) -> i32;
    fn QueryInformationJobObject(
        job: Handle,
        class: u32,
        data: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
    fn TerminateJobObject(job: Handle, code: u32) -> i32;
    fn CreatePipe(
        read: *mut Handle,
        write: *mut Handle,
        attributes: *mut SecurityAttributes,
        size: u32,
    ) -> i32;
    fn SetHandleInformation(handle: Handle, mask: u32, flags: u32) -> i32;
    fn InitializeProcThreadAttributeList(
        list: *mut c_void,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *mut c_void,
        size: usize,
        previous: *mut c_void,
        returned: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn CreateProcessW(
        application: *const u16,
        command: *mut u16,
        process_attributes: *mut c_void,
        thread_attributes: *mut c_void,
        inherit: i32,
        flags: u32,
        environment: *mut c_void,
        directory: *const u16,
        startup: *mut StartupInfo,
        info: *mut ProcessInfo,
    ) -> i32;
    fn WaitForSingleObject(handle: Handle, timeout: u32) -> u32;
    fn GetExitCodeProcess(process: Handle, code: *mut u32) -> i32;
}

fn error(context: &str) -> String {
    format!("{context}: {}", std::io::Error::last_os_error())
}

pub(crate) struct WindowsJob(OwnedHandle);
impl WindowsJob {
    fn new() -> Result<Self, String> {
        // SAFETY: unnamed, noninheritable kernel handle, owned exactly once.
        let raw = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
        if raw.is_null() {
            return Err(error("CreateJobObjectW failed"));
        }
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        let mut limits: ExtendedLimits = unsafe { std::mem::zeroed() };
        limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, no breakaway.
        if unsafe {
            SetInformationJobObject(
                raw,
                9,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(error("Configuring sidecar Job Object failed"));
        }
        Ok(job)
    }

    pub(crate) fn terminate(&self) -> Result<(), String> {
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            return Err(error("Terminating sidecar process tree failed"));
        }
        Ok(())
    }

    pub(crate) fn is_empty(&self) -> Result<bool, String> {
        let mut accounting = Accounting::default();
        if unsafe {
            QueryInformationJobObject(
                self.0.as_raw_handle(),
                1,
                &mut accounting as *mut _ as _,
                std::mem::size_of_val(&accounting) as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(error("Querying sidecar process tree failed"));
        }
        Ok(accounting.active == 0)
    }
}

pub(crate) struct WindowsChild {
    stdin: File,
    job: Arc<WindowsJob>,
}
impl WindowsChild {
    pub(crate) fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        self.stdin.write_all(bytes)
    }
    pub(crate) fn kill(self) -> Result<(), String> {
        self.job.terminate()
    }
    pub(crate) fn job(&self) -> Arc<WindowsJob> {
        Arc::clone(&self.job)
    }
}

struct Attributes {
    storage: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn new() -> Result<Self, String> {
        let mut bytes = 0;
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(error("Sizing process attributes failed"));
        }
        let mut list = Self {
            storage: vec![0; bytes.div_ceil(std::mem::size_of::<usize>())],
            initialized: false,
        };
        if unsafe { InitializeProcThreadAttributeList(list.ptr(), 2, 0, &mut bytes) } == 0 {
            return Err(error("Initializing process attributes failed"));
        }
        list.initialized = true;
        Ok(list)
    }
    fn ptr(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast()
    }
    fn set(&mut self, key: usize, values: &mut [Handle]) -> Result<(), String> {
        if unsafe {
            UpdateProcThreadAttribute(
                self.ptr(),
                0,
                key,
                values.as_mut_ptr().cast(),
                std::mem::size_of_val(values),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(error("Setting process attributes failed"));
        }
        Ok(())
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                DeleteProcThreadAttributeList(self.ptr());
            }
        }
    }
}

fn pipe(parent_reads: bool) -> Result<(OwnedHandle, OwnedHandle), String> {
    let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
    let mut attributes = SecurityAttributes {
        size: std::mem::size_of::<SecurityAttributes>() as u32,
        descriptor: std::ptr::null_mut(),
        inherit: 1,
    };
    if unsafe { CreatePipe(&mut read, &mut write, &mut attributes, 0) } == 0 {
        return Err(error("Creating sidecar pipe failed"));
    }
    let read = unsafe { OwnedHandle::from_raw_handle(read) };
    let write = unsafe { OwnedHandle::from_raw_handle(write) };
    let (parent, child) = if parent_reads {
        (read, write)
    } else {
        (write, read)
    };
    if unsafe { SetHandleInformation(parent.as_raw_handle(), 1, 0) } == 0 {
        return Err(error("Restricting pipe inheritance failed"));
    }
    Ok((parent, child))
}

fn wide(value: &OsStr) -> Result<Vec<u16>, String> {
    let mut units: Vec<_> = value.encode_wide().collect();
    if units.contains(&0) {
        return Err("NUL in sidecar command argument".to_string());
    }
    units.push(0);
    Ok(units)
}

// Windows CRT quoting, including trailing backslashes before the closing quote.
fn quoted(value: &OsStr, output: &mut Vec<u16>) -> Result<(), String> {
    output.push(b'"' as u16);
    let mut slashes = 0;
    for unit in value.encode_wide() {
        if unit == 0 {
            return Err("NUL in sidecar command argument".to_string());
        }
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        output.extend(std::iter::repeat_n(
            b'\\' as u16,
            if unit == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        slashes = 0;
        output.push(unit);
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
    Ok(())
}

fn send(tx: &tauri::async_runtime::Sender<CommandEvent>, event: CommandEvent) -> bool {
    tauri::async_runtime::block_on(tx.send(event)).is_ok()
}
fn reader(
    handle: OwnedHandle,
    tx: tauri::async_runtime::Sender<CommandEvent>,
    stderr: bool,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("fusion-sidecar-output".into())
        .spawn(move || {
            let mut reader = BufReader::new(File::from(handle));
            loop {
                let mut bytes = Vec::new();
                // Torrent metadata feeds the inventory JSON line. Bound its
                // memory even if the metadata or the sidecar is malformed.
                let mut limited = (&mut reader).take(16 * 1024 * 1024 + 1);
                match limited.read_until(b'\n', &mut bytes) {
                    Ok(0) => break,
                    Ok(_) => {
                        if bytes.len() > 16 * 1024 * 1024 {
                            send(
                                &tx,
                                CommandEvent::Error(
                                    "Sidecar output line exceeds 16 MiB".to_string(),
                                ),
                            );
                            break;
                        }
                        if !send(
                            &tx,
                            if stderr {
                                CommandEvent::Stderr(bytes)
                            } else {
                                CommandEvent::Stdout(bytes)
                            },
                        ) {
                            break;
                        }
                    }
                    Err(e) => {
                        send(&tx, CommandEvent::Error(e.to_string()));
                        break;
                    }
                }
            }
        })
}

#[derive(Default)]
struct ObserverState {
    process: Option<Arc<OwnedHandle>>,
    cancelled: bool,
    stdout: Option<JoinHandle<()>>,
    stderr: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct ObserverHandoff {
    state: Mutex<ObserverState>,
    ready: Condvar,
}

impl ObserverHandoff {
    fn lock(&self) -> MutexGuard<'_, ObserverState> {
        // Rollback must also work while unwinding from an observer setup panic.
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn join_readers(&self) {
        let (stdout, stderr) = {
            let mut state = self.lock();
            (state.stdout.take(), state.stderr.take())
        };
        for reader in [stdout, stderr].into_iter().flatten() {
            let _ = reader.join();
        }
    }
}

/// All fallible worker setup precedes native process creation. Until publication
/// this owner closes the pipes, cancels the handoff and joins every worker on
/// both ordinary errors and unwinding.
struct PreparedObservers {
    handoff: Arc<ObserverHandoff>,
    exit: Option<JoinHandle<()>>,
    receiver: Option<tauri::async_runtime::Receiver<CommandEvent>>,
    child_pipes: Option<[OwnedHandle; 3]>,
    process: Option<Arc<OwnedHandle>>,
    job: Arc<WindowsJob>,
    published: bool,
}

impl Drop for PreparedObservers {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        // A full output channel must not prevent its reader from being joined.
        drop(self.receiver.take());
        drop(self.child_pipes.take());
        if let Some(process) = &self.process {
            // There are no fallible operations after native birth. This path
            // covers unexpected unwinding nevertheless: never release caller
            // control (which can remove staging) without positive exit proof.
            loop {
                let _ = self.job.terminate();
                let exited = unsafe { WaitForSingleObject(process.as_raw_handle(), 25) } == 0;
                if exited && matches!(self.job.is_empty(), Ok(true)) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
        self.handoff.lock().cancelled = true;
        self.handoff.ready.notify_one();
        if let Some(exit) = self.exit.take() {
            let _ = exit.join();
        }
        self.handoff.join_readers();
    }
}

fn exit_observer(handoff: Arc<ObserverHandoff>, tx: tauri::async_runtime::Sender<CommandEvent>) {
    let process = {
        let mut state = handoff.lock();
        while state.process.is_none() && !state.cancelled {
            state = handoff
                .ready
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        if state.cancelled {
            None
        } else {
            state.process.take()
        }
    };
    let event = process.map(|process| {
        let result = unsafe { WaitForSingleObject(process.as_raw_handle(), u32::MAX) };
        let mut code = 0;
        if result == 0 && unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) } != 0 {
            CommandEvent::Terminated(TerminatedPayload {
                code: Some(code as i32),
                signal: None,
            })
        } else {
            CommandEvent::Error(error("Waiting for sidecar process failed"))
        }
    });
    // Preserve the final progress/error line before publishing exit. If
    // descendants retain pipes, stop's bounded timeout terminates the job.
    handoff.join_readers();
    if let Some(event) = event {
        send(&tx, event);
    }
}

pub(crate) fn spawn(
    command: Command,
) -> Result<(tauri::async_runtime::Receiver<CommandEvent>, WindowsChild), String> {
    spawn_prepared(command, |_| Ok(()))
}

fn spawn_prepared(
    command: Command,
    mut before_observer: impl FnMut(&Arc<WindowsJob>) -> std::io::Result<()>,
) -> Result<(tauri::async_runtime::Receiver<CommandEvent>, WindowsChild), String> {
    // Callers currently inherit the launcher environment. Reject unsupported
    // overrides instead of silently changing the shell plugin's semantics.
    if command.get_envs().next().is_some() {
        return Err("Sidecar environment overrides are unsupported".to_string());
    }
    let job = Arc::new(WindowsJob::new()?);
    let (stdin, child_stdin) = pipe(false)?;
    let (stdout, child_stdout) = pipe(true)?;
    let (stderr, child_stderr) = pipe(true)?;
    let mut handles = [
        child_stdin.as_raw_handle(),
        child_stdout.as_raw_handle(),
        child_stderr.as_raw_handle(),
    ];
    let mut jobs = [job.0.as_raw_handle()];
    let mut attributes = Attributes::new()?;
    attributes.set(0x20002, &mut handles)?; // PROC_THREAD_ATTRIBUTE_HANDLE_LIST
    attributes.set(0x2000d, &mut jobs)?; // PROC_THREAD_ATTRIBUTE_JOB_LIST
    let application = wide(command.get_program())?;
    let mut line = Vec::new();
    quoted(command.get_program(), &mut line)?;
    for argument in command.get_args() {
        line.push(b' ' as u16);
        quoted(argument, &mut line)?;
    }
    line.push(0);
    let directory = command
        .get_current_dir()
        .map(|path| wide(path.as_os_str()))
        .transpose()?;
    let mut startup: StartupInfoEx = unsafe { std::mem::zeroed() };
    startup.startup.size = std::mem::size_of::<StartupInfoEx>() as u32;
    startup.startup.flags = 0x100; // STARTF_USESTDHANDLES
    startup.startup.stdin = handles[0];
    startup.startup.stdout = handles[1];
    startup.startup.stderr = handles[2];
    startup.attributes = attributes.ptr();
    let (tx, rx) = tauri::async_runtime::channel(32);
    let mut observers = PreparedObservers {
        handoff: Arc::new(ObserverHandoff::default()),
        exit: None,
        receiver: Some(rx),
        child_pipes: Some([child_stdin, child_stdout, child_stderr]),
        process: None,
        job: Arc::clone(&job),
        published: false,
    };
    before_observer(&job).map_err(|error| error.to_string())?;
    observers.handoff.lock().stdout =
        Some(reader(stdout, tx.clone(), false).map_err(|error| error.to_string())?);
    before_observer(&job).map_err(|error| error.to_string())?;
    observers.handoff.lock().stderr =
        Some(reader(stderr, tx.clone(), true).map_err(|error| error.to_string())?);
    before_observer(&job).map_err(|error| error.to_string())?;
    let handoff = Arc::clone(&observers.handoff);
    observers.exit = Some(
        std::thread::Builder::new()
            .name("fusion-sidecar-exit".into())
            .spawn(move || exit_observer(handoff, tx))
            .map_err(|error| error.to_string())?,
    );
    // Acquire the handoff before birth: publication is only an infallible
    // assignment/notification, with no channel send or further thread spawn.
    let mut handoff = observers.handoff.lock();
    let mut info: ProcessInfo = unsafe { std::mem::zeroed() };
    // Assignment is part of process creation. There is no running or suspended
    // unassigned process if the launcher crashes between spawn and publication.
    if unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            1,
            0x08000000 | 0x00080000,
            std::ptr::null_mut(),
            directory.as_ref().map_or(std::ptr::null(), |d| d.as_ptr()),
            &mut startup.startup,
            &mut info,
        )
    } == 0
    {
        return Err(error(
            "Creating job-bound sidecar failed (Windows 10+ required)",
        ));
    }
    let process = Arc::new(unsafe { OwnedHandle::from_raw_handle(info.process) });
    observers.process = Some(Arc::clone(&process));
    handoff.process = Some(process);
    let thread = unsafe { OwnedHandle::from_raw_handle(info.thread) };
    drop(thread);
    drop(observers.child_pipes.take());
    drop(handoff);
    observers.handoff.ready.notify_one();
    let rx = observers.receiver.take().expect("prepared event receiver");
    observers.published = true;
    Ok((
        rx,
        WindowsChild {
            stdin: File::from(stdin),
            job,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::time::{Duration, Instant};

    struct FixtureLauncher(std::process::Child);
    impl Drop for FixtureLauncher {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    }

    fn powershell(script: &str) -> Command {
        let executable = std::env::var_os("SystemRoot").unwrap();
        let executable = std::path::PathBuf::from(executable)
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let mut command = Command::new(executable);
        command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
        command
    }

    fn descendant_script(marker: &Path) -> String {
        let marker = marker.to_string_lossy().replace('\'', "''");
        format!("$p=Start-Process -FilePath \"$env:SystemRoot\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 120') -PassThru -WindowStyle Hidden; [IO.File]::WriteAllText('{marker}',[string]$p.Id); exit 0")
    }

    fn descendant_handle(marker: &Path) -> OwnedHandle {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Ok(pid) = std::fs::read_to_string(marker) {
                if let Ok(pid) = pid.trim().parse() {
                    let raw = unsafe { OpenProcess(0x00100000, 0, pid) }; // SYNCHRONIZE
                    assert!(
                        !raw.is_null(),
                        "OpenProcess fixture descendant: {}",
                        std::io::Error::last_os_error()
                    );
                    return unsafe { OwnedHandle::from_raw_handle(raw) };
                }
            }
            assert!(
                Instant::now() < deadline,
                "fixture descendant did not start"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    #[test]
    fn observer_setup_failures_precede_native_birth_and_release_workers() {
        for failed_observer in 0..3 {
            let temp = tempfile::tempdir().unwrap();
            let marker = temp.path().join("payload-writer-started.txt");
            let marker_literal = marker.to_string_lossy().replace('\'', "''");
            let command = powershell(&format!(
                "[IO.File]::WriteAllText('{marker_literal}', 'started'); Start-Sleep -Seconds 120"
            ));
            let mut observer = 0;
            let mut retained_job = None;
            let result = spawn_prepared(command, |job| {
                assert!(
                    job.is_empty().unwrap(),
                    "observer setup followed native birth"
                );
                retained_job = Some(Arc::clone(job));
                let current = observer;
                observer += 1;
                if current == failed_observer {
                    Err(std::io::Error::other("injected observer creation failure"))
                } else {
                    Ok(())
                }
            });
            let Err(error) = result else {
                panic!("observer {failed_observer} fault did not reject spawn");
            };
            assert!(error.contains("injected observer creation failure"));
            assert_eq!(observer, failed_observer + 1);
            let job = retained_job.unwrap();
            assert!(
                job.is_empty().unwrap(),
                "failed setup left a native process"
            );
            assert!(!marker.exists(), "failed setup started a payload writer");
            // Error return has synchronously joined every prepared worker.
            assert_eq!(Arc::strong_count(&job), 1);
            let released = temp.path().with_extension("released");
            std::fs::rename(temp.path(), &released).unwrap();
            std::fs::remove_dir_all(released).unwrap();
        }
    }

    #[test]
    fn native_create_failure_cancels_and_joins_all_prepared_observers() {
        let temp = tempfile::tempdir().unwrap();
        let command = Command::new(temp.path().join("missing-sidecar.exe"));
        let mut observers = 0;
        let mut retained_job = None;
        let result = spawn_prepared(command, |job| {
            retained_job = Some(Arc::clone(job));
            observers += 1;
            Ok(())
        });
        let Err(error) = result else {
            panic!("missing native executable unexpectedly started");
        };
        assert!(error.contains("Creating job-bound sidecar failed"));
        assert_eq!(observers, 3);
        let job = retained_job.unwrap();
        assert!(job.is_empty().unwrap());
        assert_eq!(Arc::strong_count(&job), 1);
    }

    #[test]
    fn observer_setup_unwind_cancels_and_joins_prepared_workers() {
        let mut retained_job = None;
        let mut observers = 0;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = spawn_prepared(powershell("Start-Sleep -Seconds 120"), |job| {
                retained_job = Some(Arc::clone(job));
                observers += 1;
                assert!(observers < 3, "injected observer setup panic");
                Ok(())
            });
        }));
        assert!(result.is_err());
        let job = retained_job.unwrap();
        assert!(job.is_empty().unwrap());
        assert_eq!(Arc::strong_count(&job), 1);
    }

    fn job_process_handles(job: &WindowsJob) -> Vec<(usize, OwnedHandle)> {
        #[repr(C)]
        struct ProcessIds {
            assigned: u32,
            count: u32,
            ids: [usize; 64],
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let mut ids: ProcessIds = unsafe { std::mem::zeroed() };
            assert_ne!(
                unsafe {
                    QueryInformationJobObject(
                        job.0.as_raw_handle(),
                        3,
                        (&mut ids as *mut ProcessIds).cast(),
                        std::mem::size_of::<ProcessIds>() as u32,
                        std::ptr::null_mut(),
                    )
                },
                0,
                "QueryInformationJobObject process ids: {}",
                std::io::Error::last_os_error()
            );
            if ids.count >= 2 {
                return ids.ids[..ids.count as usize]
                    .iter()
                    .map(|pid| {
                        let raw = unsafe { OpenProcess(0x00100000, 0, *pid as u32) };
                        assert!(!raw.is_null(), "opening actual onefile process {pid}");
                        (*pid, unsafe { OwnedHandle::from_raw_handle(raw) })
                    })
                    .collect();
            }
            assert!(
                Instant::now() < deadline,
                "onefile bootloader and Python child did not appear"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn onefile_command(destination: &Path) -> Command {
        let executable = std::env::var_os("FUSION_NATIVE_SIDECAR_EXE")
            .expect("set FUSION_NATIVE_SIDECAR_EXE to the built production sidecar");
        let mut command = Command::new(executable);
        command
            .arg("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567")
            .arg(destination);
        command
    }

    #[tokio::test]
    #[ignore = "requires the built production PyInstaller onefile sidecar"]
    async fn production_onefile_force_stop_empties_job_and_exits_all_processes() {
        let temp = tempfile::tempdir().unwrap();
        let (mut rx, child) = spawn(onefile_command(temp.path())).unwrap();
        let job = child.job();
        let processes = job_process_handles(&job);
        println!(
            "Actual onefile job process ids: {:?}",
            processes.iter().map(|(pid, _)| *pid).collect::<Vec<_>>()
        );
        child.kill().unwrap();
        for (pid, process) in processes {
            assert_eq!(
                unsafe { WaitForSingleObject(process.as_raw_handle(), 10000) },
                0,
                "actual onefile process {pid} survived force stop"
            );
        }
        tokio::time::timeout(Duration::from_secs(10), async {
            while !job.is_empty().unwrap() {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            while let Some(event) = rx.recv().await {
                if matches!(event, CommandEvent::Terminated(_)) {
                    return;
                }
            }
            panic!("actual onefile root exit was not observed");
        })
        .await
        .unwrap();
        println!("Actual onefile force stop confirmed: job empty, every retained process handle signaled");
    }

    #[test]
    fn production_onefile_crash_helper() {
        let Some(marker) = std::env::var_os("FUSION_ONEFILE_CRASH_FIXTURE") else {
            return;
        };
        let destination = std::env::var_os("FUSION_ONEFILE_CRASH_DESTINATION").unwrap();
        let (_rx, child) = spawn(onefile_command(Path::new(&destination))).unwrap();
        let processes = job_process_handles(&child.job());
        let ids = processes
            .iter()
            .map(|(pid, _)| pid.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(marker, ids).unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    #[ignore = "requires the built production PyInstaller onefile sidecar"]
    fn production_onefile_launcher_crash_leaves_no_orphan() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("onefile-processes.txt");
        let mut launcher = FixtureLauncher(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "windows_sidecar::tests::production_onefile_crash_helper",
                    "--nocapture",
                ])
                .env("FUSION_ONEFILE_CRASH_FIXTURE", &marker)
                .env("FUSION_ONEFILE_CRASH_DESTINATION", temp.path())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let deadline = Instant::now() + Duration::from_secs(40);
        let pids = loop {
            if let Ok(text) = std::fs::read_to_string(&marker) {
                break text
                    .lines()
                    .map(|pid| pid.parse::<u32>().unwrap())
                    .collect::<Vec<_>>();
            }
            assert!(
                Instant::now() < deadline,
                "onefile fixture launcher did not initialize"
            );
            std::thread::sleep(Duration::from_millis(25));
        };
        let processes = pids
            .iter()
            .map(|pid| {
                let raw = unsafe { OpenProcess(0x00100000, 0, *pid) };
                assert!(!raw.is_null(), "OpenProcess actual onefile child {pid}");
                unsafe { OwnedHandle::from_raw_handle(raw) }
            })
            .collect::<Vec<_>>();
        println!("Actual onefile job ids before fixture launcher crash: {pids:?}");
        launcher.0.kill().unwrap();
        launcher.0.wait().unwrap();
        for (pid, process) in pids.iter().zip(processes) {
            assert_eq!(
                unsafe { WaitForSingleObject(process.as_raw_handle(), 10000) },
                0,
                "actual onefile process {pid} survived launcher crash"
            );
        }
        println!("Actual onefile launcher crash confirmed: bootloader and Python child exited");
    }

    #[tokio::test]
    async fn force_stop_terminates_native_descendant_after_bootloader_exit() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("descendant.pid");
        let (mut rx, child) = spawn(powershell(&descendant_script(&marker))).unwrap();
        let job = child.job();
        let descendant = descendant_handle(&marker);
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = rx.recv().await {
                if matches!(event, CommandEvent::Terminated(_)) {
                    return;
                }
            }
            panic!("fixture root exit was not observed");
        })
        .await
        .unwrap();
        // A PID-derived process handle is retained only for this test's exit
        // assertion. Production never uses PID-based killing or lookup.
        assert!(!job.is_empty().unwrap());
        job.terminate().unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(descendant.as_raw_handle(), 10000) },
            0
        );
        drop(descendant);
        tokio::time::timeout(Duration::from_secs(10), async {
            while !job.is_empty().unwrap() {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .unwrap();
        drop(child);
        std::fs::rename(temp.path(), temp.path().with_extension("released")).unwrap();
        std::fs::remove_dir_all(temp.path().with_extension("released")).unwrap();
    }

    #[test]
    fn launcher_crash_helper() {
        let Some(marker) = std::env::var_os("FUSION_JOB_CRASH_FIXTURE") else {
            return;
        };
        let (_rx, _child) = spawn(powershell(&descendant_script(Path::new(&marker)))).unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    #[test]
    fn killed_launcher_leaves_no_native_sidecar_descendant() {
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("crash-descendant.pid");
        let mut launcher = FixtureLauncher(
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "windows_sidecar::tests::launcher_crash_helper",
                    "--nocapture",
                ])
                .env("FUSION_JOB_CRASH_FIXTURE", &marker)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        );
        let descendant = descendant_handle(&marker);
        launcher.0.kill().unwrap();
        launcher.0.wait().unwrap();
        assert_eq!(
            unsafe { WaitForSingleObject(descendant.as_raw_handle(), 10000) },
            0,
            "private noninherited job handle must kill descendant on launcher crash"
        );
    }

    #[tokio::test]
    async fn graceful_exit_preserves_final_output_and_releases_job() {
        let (mut rx, mut child) = spawn(powershell(
            "$null=[Console]::ReadLine(); [Console]::WriteLine('final-status')",
        ))
        .unwrap();
        let job = child.job();
        child.write(b"stop\n").unwrap();
        let mut lines = Vec::new();
        tokio::time::timeout(Duration::from_secs(10), async {
            while let Some(event) = rx.recv().await {
                match event {
                    CommandEvent::Stdout(bytes) => {
                        lines.push(String::from_utf8_lossy(&bytes).into_owned())
                    }
                    CommandEvent::Terminated(_) => break,
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert!(lines.iter().any(|line| line.trim() == "final-status"));
        assert!(job.is_empty().unwrap());
    }
}
