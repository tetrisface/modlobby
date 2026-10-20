//! The job object the engine runs in: what it may not do with the desktop,
//! and that it dies with the job.

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
	AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_UILIMIT_DESKTOP,
	JOB_OBJECT_UILIMIT_EXITWINDOWS, JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES,
	JOB_OBJECT_UILIMIT_READCLIPBOARD, JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS,
	JOB_OBJECT_UILIMIT_WRITECLIPBOARD, JOBOBJECT_BASIC_UI_RESTRICTIONS,
	JobObjectBasicUIRestrictions, SetInformationJobObject, TerminateJobObject,
};
use windows::core::PCWSTR;

pub(crate) struct Job(HANDLE);

// A job handle is a kernel object; the pointer is an opaque id.
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
	/// No clipboard either way, no desktop switching, no logging off, no
	/// system parameters, no global atoms, and no USER handles of processes
	/// outside the job (other windows' titles stay unread). The limits last
	/// as long as the process does; the job does not end it when modlobby
	/// goes, so a game outlives a restart and is adopted by the next run.
	pub(crate) fn new() -> Result<Self, String> {
		let failed = |what: &str, err: windows::core::Error| format!("{what} the job: {err}");
		unsafe {
			let handle =
				CreateJobObjectW(None, PCWSTR::null()).map_err(|err| failed("creating", err))?;
			let job = Self(handle);
			let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
				UIRestrictionsClass: JOB_OBJECT_UILIMIT_READCLIPBOARD
					| JOB_OBJECT_UILIMIT_WRITECLIPBOARD
					| JOB_OBJECT_UILIMIT_DESKTOP
					| JOB_OBJECT_UILIMIT_EXITWINDOWS
					| JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
					| JOB_OBJECT_UILIMIT_GLOBALATOMS
					| JOB_OBJECT_UILIMIT_HANDLES,
			};
			SetInformationJobObject(
				handle,
				JobObjectBasicUIRestrictions,
				&ui as *const _ as _,
				size_of_val(&ui) as u32,
			)
			.map_err(|err| failed("restricting", err))?;
			Ok(job)
		}
	}

	pub(crate) fn assign(&self, process: HANDLE) -> Result<(), String> {
		unsafe { AssignProcessToJobObject(self.0, process) }
			.map_err(|err| format!("putting the engine in its job: {err}"))
	}

	/// Ends everything in the job. Whether the call was accepted.
	pub(crate) fn terminate(&self) -> bool {
		unsafe { TerminateJobObject(self.0, 1) }.is_ok()
	}
}

impl Drop for Job {
	fn drop(&mut self) {
		unsafe {
			let _ = CloseHandle(self.0);
		}
	}
}
