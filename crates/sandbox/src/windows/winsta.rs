//! The session's window station, where cursor rights live.
//!
//! SDL confines and warps the cursor for the game window, which takes
//! `WINSTA_WRITEATTRIBUTES`; an AppContainer token does not have it. The
//! grant is for the container's SID only and lives in this logon session.

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
	DENY_ACCESS, EXPLICIT_ACCESS_W, GRANT_ACCESS, GetSecurityInfo, REVOKE_ACCESS, SE_WINDOW_OBJECT,
	SetEntriesInAclW, SetSecurityInfo,
};
use windows::Win32::Security::{
	ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID,
};
use windows::Win32::System::StationsAndDesktops::GetProcessWindowStation;

use super::acl::trustee;

// winuser.h window-station rights.
const WINSTA_ACCESSCLIPBOARD: u32 = 0x0004;
const WINSTA_WRITEATTRIBUTES: u32 = 0x0010;
const WINSTA_READSCREEN: u32 = 0x0200;

/// Lets the container confine and warp the cursor, and denies it the
/// clipboard and the screen contents; `false` removes every entry for it.
pub(crate) fn set(sid: PSID, grant: bool) -> Result<(), String> {
	let entry = |access: u32, mode| EXPLICIT_ACCESS_W {
		grfAccessPermissions: access,
		grfAccessMode: mode,
		grfInheritance: NO_INHERITANCE,
		Trustee: trustee(sid),
	};
	let entries = if grant {
		vec![
			entry(WINSTA_WRITEATTRIBUTES, GRANT_ACCESS),
			entry(WINSTA_READSCREEN | WINSTA_ACCESSCLIPBOARD, DENY_ACCESS),
		]
	} else {
		vec![entry(0, REVOKE_ACCESS)]
	};
	let failed =
		|what: &str, err: windows::core::Error| format!("{what} the window station: {err}");
	unsafe {
		let station = HANDLE(
			GetProcessWindowStation()
				.map_err(|err| failed("opening", err))?
				.0,
		);
		let mut old: *mut ACL = std::ptr::null_mut();
		let mut descriptor = PSECURITY_DESCRIPTOR::default();
		GetSecurityInfo(
			station,
			SE_WINDOW_OBJECT,
			DACL_SECURITY_INFORMATION,
			None,
			None,
			Some(&mut old),
			None,
			Some(&mut descriptor),
		)
		.ok()
		.map_err(|err| failed("reading", err))?;
		let mut new: *mut ACL = std::ptr::null_mut();
		let result = SetEntriesInAclW(Some(&entries), Some(old), &mut new)
			.ok()
			.map_err(|err| failed("changing", err))
			.and_then(|()| {
				SetSecurityInfo(
					station,
					SE_WINDOW_OBJECT,
					DACL_SECURITY_INFORMATION,
					None,
					None,
					Some(new),
					None,
				)
				.ok()
				.map_err(|err| failed("writing", err))
			});
		if !new.is_null() {
			LocalFree(Some(HLOCAL(new as _)));
		}
		LocalFree(Some(HLOCAL(descriptor.0)));
		result
	}
}
