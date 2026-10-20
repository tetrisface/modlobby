//! Starting the process suspended inside the container, with no children.

use std::ffi::OsString;
use std::path::Path;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{PSID, SECURITY_CAPABILITIES, SID_AND_ATTRIBUTES};
use windows::Win32::System::Threading::{
	CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
	EXTENDED_STARTUPINFO_PRESENT, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
	PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY, PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES,
	PROCESS_INFORMATION, ResumeThread, STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

/// `PROCESS_CREATION_CHILD_PROCESS_RESTRICTED`: the engine may start nothing,
/// so `Spring.Start` cannot relaunch it outside the sandbox.
const CHILD_PROCESS_RESTRICTED: u32 = 0x01;

/// Without these, `CreateProcessW` refuses an AppContainer process with
/// `ERROR_ENVVAR_NOT_FOUND`; Windows rewrites them to the container's own
/// folder.
const NEEDED: &[&str] = &["LOCALAPPDATA", "TEMP", "TMP"];

pub(crate) fn check_env(env: &[(OsString, OsString)]) -> Result<(), String> {
	let missing: Vec<&str> = NEEDED
		.iter()
		.copied()
		.filter(|needed| {
			!env.iter()
				.any(|(key, _)| key.to_string_lossy().eq_ignore_ascii_case(needed))
		})
		.collect();
	if missing.is_empty() {
		return Ok(());
	}
	Err(format!(
		"the environment lacks {}, which an AppContainer needs to start",
		missing.join(", ")
	))
}

/// A process created suspended; the caller puts it in its job and resumes it.
pub(crate) struct Suspended {
	pub process: HANDLE,
	pub thread: HANDLE,
	pub pid: u32,
}

impl Suspended {
	pub(crate) fn resume(&self) {
		unsafe {
			ResumeThread(self.thread);
			let _ = CloseHandle(self.thread);
		}
	}

	/// For a process that never got its job: ended before it ran a thing.
	pub(crate) fn abandon(&self) {
		unsafe {
			let _ = TerminateProcess(self.process, 1);
			let _ = CloseHandle(self.thread);
			let _ = CloseHandle(self.process);
		}
	}
}

pub(crate) fn create(
	sid: PSID,
	capabilities: &mut [SID_AND_ATTRIBUTES],
	line: &mut [u16],
	block: &[u16],
	cwd: Option<&Path>,
) -> Result<Suspended, String> {
	unsafe {
		let security = SECURITY_CAPABILITIES {
			AppContainerSid: sid,
			Capabilities: if capabilities.is_empty() {
				std::ptr::null_mut()
			} else {
				capabilities.as_mut_ptr()
			},
			CapabilityCount: capabilities.len() as u32,
			Reserved: 0,
		};
		let child_policy: u32 = CHILD_PROCESS_RESTRICTED;

		let mut size = 0usize;
		let _ = InitializeProcThreadAttributeList(None, 2, None, &mut size);
		let mut buffer = vec![0u8; size];
		let list = LPPROC_THREAD_ATTRIBUTE_LIST(buffer.as_mut_ptr() as _);
		InitializeProcThreadAttributeList(Some(list), 2, None, &mut size)
			.map_err(|err| format!("the process attribute list: {err}"))?;

		let outcome = (|| -> Result<Suspended, String> {
			UpdateProcThreadAttribute(
				list,
				0,
				PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES as usize,
				Some(&security as *const _ as _),
				size_of::<SECURITY_CAPABILITIES>(),
				None,
				None,
			)
			.map_err(|err| format!("the container attribute: {err}"))?;
			UpdateProcThreadAttribute(
				list,
				0,
				PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY as usize,
				Some(&child_policy as *const _ as _),
				size_of::<u32>(),
				None,
				None,
			)
			.map_err(|err| format!("the child-process attribute: {err}"))?;

			let mut startup = STARTUPINFOEXW::default();
			startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
			startup.lpAttributeList = list;
			let cwd = cwd.map(|dir| HSTRING::from(dir.as_os_str()));
			let mut info = PROCESS_INFORMATION::default();
			CreateProcessW(
				PCWSTR::null(),
				Some(PWSTR(line.as_mut_ptr())),
				None,
				None,
				false,
				EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | CREATE_SUSPENDED,
				Some(block.as_ptr() as _),
				cwd.as_ref()
					.map_or(PCWSTR::null(), |dir| PCWSTR(dir.as_ptr())),
				&startup.StartupInfo,
				&mut info,
			)
			.map_err(|err| format!("starting the engine in its container: {err}"))?;
			Ok(Suspended {
				process: info.hProcess,
				thread: info.hThread,
				pid: info.dwProcessId,
			})
		})();
		DeleteProcThreadAttributeList(list);
		outcome
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_launch_without_local_app_data_is_refused_before_create_process() {
		let err = check_env(&[(OsString::from("PATH"), OsString::from("x"))]).unwrap_err();
		assert!(err.contains("LOCALAPPDATA"), "{err}");
	}
}
