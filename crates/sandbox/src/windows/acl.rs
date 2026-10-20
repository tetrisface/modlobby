//! Grants on files and directories for the container's SID.

use std::path::Path;

use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
	EXPLICIT_ACCESS_W, GRANT_ACCESS, GetNamedSecurityInfoW, NO_MULTIPLE_TRUSTEE, REVOKE_ACCESS,
	SE_FILE_OBJECT, SetEntriesInAclW, SetNamedSecurityInfoW, TRUSTEE_IS_SID,
	TRUSTEE_IS_WELL_KNOWN_GROUP, TRUSTEE_W,
};
use windows::Win32::Security::{
	ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE, PSECURITY_DESCRIPTOR, PSID,
	SUB_CONTAINERS_AND_OBJECTS_INHERIT,
};
use windows::core::{HSTRING, PWSTR};

/// `FILE_GENERIC_READ`: on a directory, listing it; on a file, reading it.
pub(crate) const READ: u32 = 0x0012_0089;
/// `FILE_GENERIC_EXECUTE`, for the engine's own directory.
pub(crate) const EXECUTE: u32 = 0x0012_00A0;
/// `FILE_ALL_ACCESS`, for the one directory the engine writes.
pub(crate) const ALL: u32 = 0x001F_01FF;

pub(crate) fn trustee(sid: PSID) -> TRUSTEE_W {
	TRUSTEE_W {
		pMultipleTrustee: std::ptr::null_mut(),
		MultipleTrusteeOperation: NO_MULTIPLE_TRUSTEE,
		TrusteeForm: TRUSTEE_IS_SID,
		TrusteeType: TRUSTEE_IS_WELL_KNOWN_GROUP,
		ptstrName: PWSTR(sid.0 as *mut u16),
	}
}

/// Adds `access` for `sid` on `path`. With `inherit`, everything under a
/// directory gets it too; without, the directory object alone does, which
/// lets the engine list a data root without reading what is in it.
pub(crate) fn grant(path: &Path, sid: PSID, access: u32, inherit: bool) -> Result<(), String> {
	let entry = EXPLICIT_ACCESS_W {
		grfAccessPermissions: access,
		grfAccessMode: GRANT_ACCESS,
		grfInheritance: if inherit {
			SUB_CONTAINERS_AND_OBJECTS_INHERIT
		} else {
			NO_INHERITANCE
		},
		Trustee: trustee(sid),
	};
	apply(path, entry)
}

/// Removes every entry for `sid` on `path`.
pub(crate) fn revoke(path: &Path, sid: PSID) -> Result<(), String> {
	let entry = EXPLICIT_ACCESS_W {
		grfAccessPermissions: 0,
		grfAccessMode: REVOKE_ACCESS,
		grfInheritance: NO_INHERITANCE,
		Trustee: trustee(sid),
	};
	apply(path, entry)
}

fn apply(path: &Path, entry: EXPLICIT_ACCESS_W) -> Result<(), String> {
	let name = HSTRING::from(path.as_os_str());
	let failed =
		|what: &str, err: windows::core::Error| format!("{what} {}: {err}", path.display());
	unsafe {
		let mut old: *mut ACL = std::ptr::null_mut();
		let mut descriptor = PSECURITY_DESCRIPTOR::default();
		GetNamedSecurityInfoW(
			&name,
			SE_FILE_OBJECT,
			DACL_SECURITY_INFORMATION,
			None,
			None,
			Some(&mut old),
			None,
			&mut descriptor,
		)
		.ok()
		.map_err(|err| failed("reading the permissions of", err))?;
		let mut new: *mut ACL = std::ptr::null_mut();
		let result = SetEntriesInAclW(Some(&[entry]), Some(old), &mut new)
			.ok()
			.map_err(|err| failed("changing the permissions of", err))
			.and_then(|()| {
				SetNamedSecurityInfoW(
					&name,
					SE_FILE_OBJECT,
					DACL_SECURITY_INFORMATION,
					None,
					None,
					Some(new),
					None,
				)
				.ok()
				.map_err(|err| failed("writing the permissions of", err))
			});
		if !new.is_null() {
			LocalFree(Some(HLOCAL(new as _)));
		}
		LocalFree(Some(HLOCAL(descriptor.0)));
		result
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use windows::Win32::Security::Authorization::GetEffectiveRightsFromAclW;

	/// What the ACL on `path` grants `sid`, as the system computes it.
	fn effective(path: &Path, sid: PSID) -> u32 {
		let name = HSTRING::from(path.as_os_str());
		unsafe {
			let mut dacl: *mut ACL = std::ptr::null_mut();
			let mut descriptor = PSECURITY_DESCRIPTOR::default();
			GetNamedSecurityInfoW(
				&name,
				SE_FILE_OBJECT,
				DACL_SECURITY_INFORMATION,
				None,
				None,
				Some(&mut dacl),
				None,
				&mut descriptor,
			)
			.ok()
			.unwrap();
			let mut rights = 0u32;
			let trustee = trustee(sid);
			GetEffectiveRightsFromAclW(dacl, &trustee, &mut rights)
				.ok()
				.unwrap();
			LocalFree(Some(HLOCAL(descriptor.0)));
			rights
		}
	}

	#[test]
	fn a_root_with_non_ascii_in_its_path_is_granted_and_listed() {
		let sid = super::super::sid::Sid::profile(super::super::PROFILE).unwrap();
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().join("spiel ü");
		std::fs::create_dir(&root).unwrap();
		assert_eq!(
			effective(&root, sid.as_psid()) & READ,
			0,
			"nothing before the grant"
		);
		grant(&root, sid.as_psid(), READ, false).unwrap();
		assert_eq!(effective(&root, sid.as_psid()) & READ, READ);
		revoke(&root, sid.as_psid()).unwrap();
		assert_eq!(
			effective(&root, sid.as_psid()) & READ,
			0,
			"nothing after the revoke"
		);
	}
}
