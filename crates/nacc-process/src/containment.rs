//! Windows Job Object containment (master plan S13.4: "Use a Job Object per
//! run so a provider CLI that spawns children cannot leave orphans").
//!
//! A provider CLI is rarely a single process: Claude Code spawns shell
//! tools, Codex spawns its sandbox helper, and any of those may spawn
//! grandchildren. Killing only the direct child leaves the tree running --
//! the exact failure the master plan's completion criteria call out
//! ("cancellation kills the complete process tree"). A Job Object is the
//! Windows API for this: every process assigned to a job (and, by default,
//! every descendant it spawns) belongs to that job, and
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` makes the OS terminate the whole
//! job when the last handle to it closes -- including when *NACC itself*
//! dies, which is the "no orphaned provider processes after a crash"
//! property master plan S16 requires.
//!
//! **Verified locally, not assumed**: the integration test in
//! [`crate::supervisor`] spawns `cmd.exe` -> `powershell.exe` (a genuine
//! two-level tree), records the grandchild's PID, cancels the run, and then
//! asserts the grandchild is really gone. That test is Windows-only
//! (`#[cfg(windows)]`) because Job Objects are a Windows API -- which is
//! exactly why the non-Windows branch below is an honest "unsupported"
//! result rather than a silent no-op that would look like containment.

/// Failure creating or using a Job Object.
#[derive(Debug, thiserror::Error)]
pub enum ContainmentError {
    #[error("failed to create a Windows job object: {detail}")]
    CreateJob { detail: String },
    #[error("failed to configure the job object (kill-on-close): {detail}")]
    ConfigureJob { detail: String },
    #[error("failed to assign process {pid} to the job object: {detail}")]
    AssignProcess { pid: u32, detail: String },
    #[error("failed to terminate the job object: {detail}")]
    TerminateJob { detail: String },
    #[error("failed to terminate process {pid}: {detail}")]
    TerminateProcess { pid: u32, detail: String },
    #[error(
        "process containment is only implemented on Windows in this build; \
         this target has no Job Object API to use"
    )]
    UnsupportedPlatform,
}

pub type Result<T> = std::result::Result<T, ContainmentError>;

#[cfg(windows)]
mod platform {
    use super::{ContainmentError, Result};
    // `STILL_ACTIVE` lives in `Foundation` (as an `NTSTATUS`) in
    // windows-sys 0.61, not in `Threading` where the `windows` crate puts
    // it -- found by compiling against the real crate rather than by
    // assuming the two bindings mirror each other.
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, STILL_ACTIVE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_TERMINATE,
    };

    /// The Windows last-error code, attached to every failure below so a
    /// support bundle says *why* the OS refused the call instead of just
    /// that it did.
    fn last_error() -> String {
        let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        format!("Win32 error {code}")
    }

    /// One Job Object, owned by exactly one supervised process. Dropping it
    /// closes the handle, which -- because the job is configured with
    /// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` -- terminates every process
    /// still in the job. That is deliberate: a dropped supervisor handle
    /// must never leak a running tree.
    pub struct JobObject {
        handle: HANDLE,
    }

    // The handle is an opaque kernel handle; it is safe to move between
    // threads and to terminate from any thread (the Win32 calls used here
    // are documented as thread-agnostic for a job handle).
    unsafe impl Send for JobObject {}
    unsafe impl Sync for JobObject {}

    impl JobObject {
        pub fn create() -> Result<Self> {
            // Unnamed (`null` name): the handle itself is the only way to
            // reach this job, so no other process or thread can attach to
            // it by name. `std::ptr::null()` rather than a named type so
            // this crate does not need the whole `Win32_Security` feature
            // just to pass a null `SECURITY_ATTRIBUTES` pointer.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return Err(ContainmentError::CreateJob {
                    detail: last_error(),
                });
            }

            // KILL_ON_JOB_CLOSE is the entire orphan-prevention mechanism;
            // without it this is just a bookkeeping object.
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let configured = unsafe {
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            if configured == 0 {
                let detail = last_error();
                unsafe { CloseHandle(handle) };
                return Err(ContainmentError::ConfigureJob { detail });
            }

            Ok(Self { handle })
        }

        /// Assign an already-spawned process to this job. Descendants it
        /// spawns afterwards join the job too, which is what makes a single
        /// `terminate()` sufficient for a whole tree.
        ///
        /// `process_handle` is a raw Win32 handle (as returned by
        /// `CreateProcess`, i.e. `tokio::process::Child::raw_handle`).
        pub fn assign(&self, pid: u32, process_handle: usize) -> Result<()> {
            let assigned =
                unsafe { AssignProcessToJobObject(self.handle, process_handle as HANDLE) };
            if assigned == 0 {
                return Err(ContainmentError::AssignProcess {
                    pid,
                    detail: last_error(),
                });
            }
            Ok(())
        }

        /// Terminate every process currently in the job.
        pub fn terminate(&self, exit_code: u32) -> Result<()> {
            let terminated = unsafe { TerminateJobObject(self.handle, exit_code) };
            if terminated == 0 {
                return Err(ContainmentError::TerminateJob {
                    detail: last_error(),
                });
            }
            Ok(())
        }
    }

    impl Drop for JobObject {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    /// Whether `pid` currently exists (its exit code is still
    /// `STILL_ACTIVE`). Used by the run supervisor's crash-recovery
    /// reconciliation (master plan S16) to decide whether a recorded
    /// process is genuinely still running, and by this crate's own tests to
    /// prove a tree really died rather than merely that a call returned
    /// `Ok`.
    pub fn process_alive(pid: u32) -> bool {
        if pid == 0 {
            return false;
        }
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return false;
        }
        let mut code: u32 = 0;
        let ok = unsafe { GetExitCodeProcess(handle, &mut code) };
        unsafe { CloseHandle(handle) };
        ok != 0 && code == STILL_ACTIVE as u32
    }

    /// Terminate one process directly. Only used as a fallback when a Job
    /// Object could not be created (e.g. an exotic parent-job
    /// configuration); it cannot reach grandchildren, which is precisely
    /// why the job-object path is the primary one.
    pub fn terminate_process(pid: u32) -> Result<()> {
        if pid == 0 {
            return Ok(());
        }
        let handle = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
        if handle.is_null() {
            return Err(ContainmentError::TerminateProcess {
                pid,
                detail: last_error(),
            });
        }
        let ok = unsafe { TerminateProcess(handle, 1) };
        let detail = if ok == 0 { Some(last_error()) } else { None };
        unsafe { CloseHandle(handle) };
        match detail {
            Some(detail) => Err(ContainmentError::TerminateProcess { pid, detail }),
            None => Ok(()),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::{ContainmentError, Result};

    /// Non-Windows: there is no Job Object API, and this workspace's only
    /// supported target is Windows (master plan S4.1). Returning a typed
    /// `UnsupportedPlatform` error -- rather than silently pretending
    /// containment happened -- lets the supervisor fall back to a
    /// documented direct-child kill and log the degraded mode honestly.
    #[derive(Debug)]
    pub struct JobObject;

    impl JobObject {
        pub fn create() -> Result<Self> {
            Err(ContainmentError::UnsupportedPlatform)
        }

        pub fn assign(&self, _pid: u32, _process_handle: usize) -> Result<()> {
            Err(ContainmentError::UnsupportedPlatform)
        }

        pub fn terminate(&self, _exit_code: u32) -> Result<()> {
            Err(ContainmentError::UnsupportedPlatform)
        }
    }

    pub fn process_alive(pid: u32) -> bool {
        // No portable "is this PID alive" syscall here; the cross-platform
        // path for waiting on a child is the supervisor's own exit watch,
        // so this is only used for reconciliation of *recorded* pids, which
        // is Windows-scoped in this build.
        let _ = pid;
        false
    }

    pub fn terminate_process(_pid: u32) -> Result<()> {
        Err(ContainmentError::UnsupportedPlatform)
    }
}

pub use platform::{process_alive, terminate_process, JobObject};

#[cfg(test)]
mod tests {
    use super::*;

    /// On Windows this build must create a real job object -- a
    /// `UnsupportedPlatform` error here means containment silently stopped
    /// working, so the assertion is the platform split itself rather than
    /// a `cfg!` check clippy would (correctly) flag as constant.
    #[cfg(windows)]
    #[test]
    fn creating_a_job_object_succeeds_on_windows() {
        let job = JobObject::create().expect("this build must create real job objects on Windows");
        // Terminating an empty job is a valid no-op, not an error.
        job.terminate(0)
            .expect("terminating an empty job must succeed");
    }

    /// On non-Windows the honest behavior is a typed `UnsupportedPlatform`
    /// error, never a silent no-op that would look like containment.
    #[cfg(not(windows))]
    #[test]
    fn job_object_creation_reports_unsupported_off_windows() {
        assert!(matches!(
            JobObject::create(),
            Err(ContainmentError::UnsupportedPlatform)
        ));
    }

    #[test]
    fn pid_zero_is_never_alive() {
        // Guards the reconciliation path: a lease/store row with a
        // missing pid must never be mistaken for a live process.
        assert!(!process_alive(0));
    }

    /// Proves `process_alive` really queries the OS instead of always
    /// returning false (which would make every orphan-recovery decision
    /// silently wrong).
    #[cfg(windows)]
    #[test]
    fn the_current_process_reports_itself_alive() {
        assert!(
            process_alive(std::process::id()),
            "the running process must report as alive"
        );
    }
}
