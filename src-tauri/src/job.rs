//! Process lifetime binding via Windows Job Objects.
//!
//! When the launcher process crashes or is forcefully terminated (e.g. via Task Manager),
//! child processes (like the libtorrent sidecar) can become orphaned.
//! By assigning child processes to a Job Object configured with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, the Windows kernel automatically terminates
//! all associated child processes when the launcher's Job Object handle is closed.

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::Mutex;

    type HANDLE = *mut c_void;
    type BOOL = i32;
    type DWORD = u32;

    const FALSE: BOOL = 0;

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: DWORD = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: DWORD = 9;
    const PROCESS_SET_QUOTA: DWORD = 0x0100;
    const PROCESS_TERMINATE: DWORD = 0x0001;

    #[repr(C)]
    struct IOCounters {
        read_operation_count: u64,
        write_operation_count: u64,
        other_operation_count: u64,
        read_transfer_count: u64,
        write_transfer_count: u64,
        other_transfer_count: u64,
    }

    #[repr(C)]
    struct JobObjectBasicLimitInformation {
        per_process_user_time_limit: i64,
        per_job_user_time_limit: i64,
        limit_flags: DWORD,
        minimum_working_set_size: usize,
        maximum_working_set_size: usize,
        active_process_limit: DWORD,
        affinity: usize,
        priority_class: DWORD,
        scheduling_class: DWORD,
    }

    #[repr(C)]
    struct JobObjectExtendedLimitInformation {
        basic_limit_information: JobObjectBasicLimitInformation,
        io_info: IOCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory_limit: usize,
        peak_job_memory_limit: usize,
    }

    extern "system" {
        fn CreateJobObjectW(lp_job_attributes: *mut c_void, lp_name: *const u16) -> HANDLE;
        fn SetInformationJobObject(
            h_job: HANDLE,
            job_object_information_class: DWORD,
            lp_job_object_information: *const c_void,
            cb_job_object_information_length: DWORD,
        ) -> BOOL;
        fn AssignProcessToJobObject(h_job: HANDLE, h_process: HANDLE) -> BOOL;
        fn OpenProcess(
            dw_desired_access: DWORD,
            b_inherit_handle: BOOL,
            dw_process_id: DWORD,
        ) -> HANDLE;
        fn CloseHandle(h_object: HANDLE) -> BOOL;
    }

    struct JobHandle(HANDLE);
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}

    static LAUNCHER_JOB: Mutex<Option<JobHandle>> = Mutex::new(None);

    fn get_launcher_job() -> Result<HANDLE, String> {
        let mut guard = LAUNCHER_JOB
            .lock()
            .map_err(|error| format!("Mutex poisoned while accessing launcher job: {error}"))?;
        if let Some(handle) = guard.as_ref() {
            return Ok(handle.0);
        }

        unsafe {
            let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
            if job.is_null() {
                return Err("Failed to create Windows Job Object for launcher lifetime".to_string());
            }

            let mut info = std::mem::zeroed::<JobObjectExtendedLimitInformation>();
            info.basic_limit_information.limit_flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            let res = SetInformationJobObject(
                job,
                JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                &info as *const _ as *const c_void,
                std::mem::size_of::<JobObjectExtendedLimitInformation>() as DWORD,
            );

            if res == FALSE {
                CloseHandle(job);
                return Err("Failed to configure JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE on Job Object".to_string());
            }

            *guard = Some(JobHandle(job));
            Ok(job)
        }
    }

    pub fn bind_to_launcher_lifetime(pid: u32) -> Result<(), String> {
        let job = get_launcher_job()?;

        unsafe {
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, FALSE, pid);
            if process.is_null() {
                return Err(format!("OpenProcess failed for pid {pid}"));
            }

            let res = AssignProcessToJobObject(job, process);
            CloseHandle(process);

            if res == FALSE {
                return Err(format!("AssignProcessToJobObject failed for pid {pid}"));
            }
            tracing::debug!("Bound child process {pid} to Windows Job Object");
            Ok(())
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn bind_to_launcher_lifetime(_pid: u32) -> Result<(), String> {
        Ok(())
    }
}

pub fn bind_to_launcher_lifetime(pid: u32) -> Result<(), String> {
    imp::bind_to_launcher_lifetime(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bind_current_process_or_noop() {
        // Test that calling bind on current process or arbitrary pid does not crash
        let current_pid = std::process::id();
        let _ = bind_to_launcher_lifetime(current_pid);
    }
}
