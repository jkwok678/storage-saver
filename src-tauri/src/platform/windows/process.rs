//! Running the AI CLIs without a console window, and stopping them with everything they start.

use std::ffi::c_void;
use std::mem::size_of;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};
use std::ptr;
use std::sync::Arc;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, OpenProcess, PROCESS_TERMINATE, TerminateProcess,
};

pub(crate) fn prepare_command(cmd: &mut Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

/// An owned job object handle.
struct Job(HANDLE);

// SAFETY: a job object handle is only a token for a kernel object. Any thread may use it, and
// the calls made with it need no other synchronization.
unsafe impl Send for Job {}
// SAFETY: see above. Nothing is mutated through a shared reference.
unsafe impl Sync for Job {}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this value and closed only here. Closing the last
        // handle kills what is still in the job.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// What it takes to stop a running child later: the job holding it and its descendants, and
/// its process id for when the job could not be set up.
#[derive(Clone)]
pub(crate) struct ChildHandle {
    pid: u32,
    job: Option<Arc<Job>>,
}

impl PartialEq for ChildHandle {
    fn eq(&self, other: &Self) -> bool {
        self.pid == other.pid
            && match (&self.job, &other.job) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

/// A job that kills its processes once its last handle closes, so nothing outlives the app.
fn kill_on_close_job() -> Option<Job> {
    // SAFETY: no security attributes and no name.
    let handle = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if handle.is_null() {
        return None;
    }
    let job = Job(handle);
    let info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
        BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION {
            LimitFlags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: `info` is the structure JobObjectExtendedLimitInformation asks for, and its size is passed.
    let ok = unsafe {
        SetInformationJobObject(
            job.0,
            JobObjectExtendedLimitInformation,
            (&raw const info).cast::<c_void>(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    (ok != 0).then_some(job)
}

/// Puts the child in a new job. A child that cannot be put in one is still tracked by its id.
pub(crate) fn track_child(child: &Child) -> ChildHandle {
    let job = kill_on_close_job().filter(|job| {
        // SAFETY: both handles are valid, and the child's handle stays open while `child` lives.
        unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) != 0 }
    });
    ChildHandle {
        pid: child.id(),
        job: job.map(Arc::new),
    }
}

pub(crate) fn terminate(handle: &ChildHandle) {
    if let Some(job) = &handle.job {
        // SAFETY: the handle is open as long as `job` is alive.
        if unsafe { TerminateJobObject(job.0, 1) } != 0 {
            return;
        }
    }
    // SAFETY: a process opened for termination only is closed again right away. The id still
    // names our child because the handle is cleared before the child is reaped.
    unsafe {
        let process = OpenProcess(PROCESS_TERMINATE, 0, handle.pid);
        if !process.is_null() {
            TerminateProcess(process, 1);
            CloseHandle(process);
        }
    }
}
