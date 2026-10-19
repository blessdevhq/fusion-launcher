//! Private Windows process group. Job assignment is atomic with process birth.
//! No process can execute outside the kill-on-close job, including descendants.

use std::ffi::{c_void, OsStr};
use std::fs::File;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::process::Command;

use super::super::errors::{ErrorCode, FusionError};

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

fn error(context: &str) -> FusionError {
    FusionError::new(ErrorCode::ProcessSupervisionFailed, context)
        .with_detail("osError", std::io::Error::last_os_error().to_string())
}

pub(super) struct WindowsJob(OwnedHandle);
impl WindowsJob {
    pub(super) fn new() -> Result<Self, FusionError> {
        // SAFETY: unnamed, noninheritable job handle is uniquely owned.
        let raw = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
        if raw.is_null() {
            return Err(error("Cannot create process job."));
        }
        let job = Self(unsafe { OwnedHandle::from_raw_handle(raw) });
        let mut limits: ExtendedLimits = unsafe { std::mem::zeroed() };
        limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE; no breakaway.
        if unsafe {
            SetInformationJobObject(
                raw,
                9,
                (&limits as *const ExtendedLimits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(error("Cannot configure process job."));
        }
        Ok(job)
    }

    pub(super) fn terminate(&self) -> Result<(), FusionError> {
        if unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } == 0 {
            return Err(error("Cannot terminate process group."));
        }
        Ok(())
    }

    pub(super) fn is_empty(&self) -> Result<bool, FusionError> {
        let mut accounting = Accounting::default();
        if unsafe {
            QueryInformationJobObject(
                self.0.as_raw_handle(),
                1,
                (&mut accounting as *mut Accounting).cast(),
                std::mem::size_of_val(&accounting) as u32,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(error("Cannot inspect process group."));
        }
        Ok(accounting.active == 0)
    }

    pub(super) fn spawn(&self, command: &Command) -> Result<WindowsProcess, FusionError> {
        let (stdin, child_stdin) = pipe(false)?;
        let (stdout, child_stdout) = pipe(true)?;
        let (stderr, child_stderr) = pipe(true)?;
        let mut inherited = [
            child_stdin.as_raw_handle(),
            child_stdout.as_raw_handle(),
            child_stderr.as_raw_handle(),
        ];
        let mut jobs = [self.0.as_raw_handle()];
        let mut attributes = Attributes::new()?;
        attributes.set(0x20002, &mut inherited)?; // HANDLE_LIST: only explicit pipes inherited.
        attributes.set(0x2000d, &mut jobs)?; // JOB_LIST: assigned before any user code executes.
        let application = wide(command.get_program())?;
        let mut line = Vec::new();
        quote(command.get_program(), &mut line)?;
        for argument in command.get_args() {
            line.push(b' ' as u16);
            quote(argument, &mut line)?;
        }
        line.push(0);
        let directory = command
            .get_current_dir()
            .map(|p| wide(p.as_os_str()))
            .transpose()?;
        // Command was env_clear()'d by the supervisor. Supplying an explicit
        // UTF-16 block prevents inheriting private launcher environment values.
        let mut values: Vec<_> = command
            .get_envs()
            .filter_map(|(k, v)| v.map(|v| (k, v)))
            .collect();
        values.sort_by(|(a, _), (b, _)| {
            a.to_string_lossy()
                .to_uppercase()
                .cmp(&b.to_string_lossy().to_uppercase())
        });
        let mut environment = Vec::new();
        for (key, value) in values {
            let key = wide(key)?;
            let value = wide(value)?;
            environment.extend_from_slice(&key[..key.len() - 1]);
            environment.push(b'=' as u16);
            environment.extend_from_slice(&value[..value.len() - 1]);
            environment.push(0);
        }
        environment.push(0);
        if environment.len() == 1 {
            environment.push(0);
        }
        let mut startup: StartupInfoEx = unsafe { std::mem::zeroed() };
        startup.startup.size = std::mem::size_of::<StartupInfoEx>() as u32;
        startup.startup.flags = 0x100;
        startup.startup.stdin = inherited[0];
        startup.startup.stdout = inherited[1];
        startup.startup.stderr = inherited[2];
        startup.attributes = attributes.ptr();
        let mut info: ProcessInfo = unsafe { std::mem::zeroed() };
        if unsafe {
            CreateProcessW(
                application.as_ptr(),
                line.as_mut_ptr(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                1,
                0x08000000 | 0x00080000 | 0x00000400,
                environment.as_mut_ptr().cast(),
                directory.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
                &mut startup.startup,
                &mut info,
            )
        } == 0
        {
            return Err(FusionError::new(
                ErrorCode::ProcessLaunchFailed,
                "Cannot create job-bound process (Windows 10+ required).",
            )
            .with_detail("osError", std::io::Error::last_os_error().to_string()));
        }
        let process = unsafe { OwnedHandle::from_raw_handle(info.process) };
        let thread = unsafe { OwnedHandle::from_raw_handle(info.thread) };
        drop(thread);
        drop((stdin, child_stdin, child_stdout, child_stderr));
        Ok(WindowsProcess {
            process,
            stdout: Some(File::from(stdout)),
            stderr: Some(File::from(stderr)),
        })
    }
}

pub(super) struct WindowsProcess {
    process: OwnedHandle,
    pub stdout: Option<File>,
    pub stderr: Option<File>,
}
impl WindowsProcess {
    pub(super) fn try_wait(&self) -> Result<Option<i32>, FusionError> {
        match unsafe { WaitForSingleObject(self.process.as_raw_handle(), 0) } {
            258 => Ok(None),
            0 => {
                let mut code = 0;
                if unsafe { GetExitCodeProcess(self.process.as_raw_handle(), &mut code) } == 0 {
                    return Err(error("Cannot obtain process exit code."));
                }
                Ok(Some(code as i32))
            }
            _ => Err(error("Cannot wait for process.")),
        }
    }
}

struct Attributes {
    storage: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn new() -> Result<Self, FusionError> {
        let mut bytes = 0;
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 2, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(error("Cannot size process attributes."));
        }
        let mut this = Self {
            storage: vec![0; bytes.div_ceil(std::mem::size_of::<usize>())],
            initialized: false,
        };
        if unsafe { InitializeProcThreadAttributeList(this.ptr(), 2, 0, &mut bytes) } == 0 {
            return Err(error("Cannot initialize process attributes."));
        }
        this.initialized = true;
        Ok(this)
    }
    fn ptr(&mut self) -> *mut c_void {
        self.storage.as_mut_ptr().cast()
    }
    fn set(&mut self, key: usize, values: &mut [Handle]) -> Result<(), FusionError> {
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
            return Err(error("Cannot set process attributes."));
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

fn pipe(parent_reads: bool) -> Result<(OwnedHandle, OwnedHandle), FusionError> {
    let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
    let mut attributes = SecurityAttributes {
        size: std::mem::size_of::<SecurityAttributes>() as u32,
        descriptor: std::ptr::null_mut(),
        inherit: 1,
    };
    if unsafe { CreatePipe(&mut read, &mut write, &mut attributes, 0) } == 0 {
        return Err(error("Cannot create process pipe."));
    }
    let read = unsafe { OwnedHandle::from_raw_handle(read) };
    let write = unsafe { OwnedHandle::from_raw_handle(write) };
    let (parent, child) = if parent_reads {
        (read, write)
    } else {
        (write, read)
    };
    if unsafe { SetHandleInformation(parent.as_raw_handle(), 1, 0) } == 0 {
        return Err(error("Cannot restrict process pipe inheritance."));
    }
    Ok((parent, child))
}
fn wide(value: &OsStr) -> Result<Vec<u16>, FusionError> {
    let mut units: Vec<_> = value.encode_wide().collect();
    if units.contains(&0) {
        return Err(FusionError::new(
            ErrorCode::InvalidEnvironment,
            "NUL in process parameter.",
        ));
    }
    units.push(0);
    Ok(units)
}
fn quote(value: &OsStr, out: &mut Vec<u16>) -> Result<(), FusionError> {
    out.push(b'"' as u16);
    let mut slashes = 0;
    for unit in value.encode_wide() {
        if unit == 0 {
            return Err(FusionError::new(
                ErrorCode::InvalidEnvironment,
                "NUL in process argument.",
            ));
        }
        if unit == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        out.extend(std::iter::repeat_n(
            b'\\' as u16,
            if unit == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        slashes = 0;
        out.push(unit);
    }
    out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    out.push(b'"' as u16);
    Ok(())
}
