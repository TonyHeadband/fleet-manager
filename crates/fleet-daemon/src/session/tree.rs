//! Ends a session's whole process tree, not just claude: its tools (shells, git, test runners) go with it.

/// What kills one session's tree. On Unix the PTY child leads its own session and process group; on Windows the
/// child is put in a Job object that kills everything in it when it is closed.
pub struct Tree {
    #[cfg(unix)]
    pgid: i32,
    #[cfg(windows)]
    job: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: the job handle is only used through kernel calls, which are thread-safe.
#[cfg(windows)]
unsafe impl Send for Tree {}
#[cfg(windows)]
unsafe impl Sync for Tree {}

impl Tree {
    #[cfg(unix)]
    pub fn adopt(pid: u32) -> std::io::Result<Tree> {
        Ok(Tree { pgid: pid as i32 })
    }

    #[cfg(windows)]
    pub fn adopt(pid: u32) -> std::io::Result<Tree> {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::JobObjects::*;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
        };
        // SAFETY: plain kernel object calls; every handle opened here is closed here or owned by `Tree`.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                let e = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(e);
            }
            let ok = AssignProcessToJobObject(job, process);
            let e = std::io::Error::last_os_error();
            CloseHandle(process);
            if ok == 0 {
                CloseHandle(job);
                return Err(e);
            }
            Ok(Tree { job })
        }
    }

    pub fn kill(&self) {
        #[cfg(unix)]
        // SAFETY: signals a process group this daemon started; a group already gone is ESRCH, ignored.
        unsafe {
            libc::killpg(self.pgid, libc::SIGKILL);
        }
        #[cfg(windows)]
        // SAFETY: the job handle is owned by `self` and still open.
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for Tree {
    fn drop(&mut self) {
        // SAFETY: closes the handle `adopt` opened; KILL_ON_JOB_CLOSE ends anything still in the job.
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}
