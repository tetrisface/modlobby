//! The container's identity, and the one capability it may be given.

use windows::Win32::Security::Isolation::{
	CreateAppContainerProfile, DeriveAppContainerSidFromAppContainerName,
};
use windows::Win32::Security::{
	CreateWellKnownSid, FreeSid, GetLengthSid, PSID, WinCapabilityInternetClientSid,
};
use windows::core::HSTRING;

/// A SID held as its bytes, so it can be cloned and sent across threads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sid(Vec<u8>);

impl Sid {
	/// The profile `name`'s SID, creating the profile the first time. The SID
	/// is a hash of the name, so it is the same on every machine.
	pub(crate) fn profile(name: &str) -> Result<Self, String> {
		let wide = HSTRING::from(name);
		let description = HSTRING::from("The sandbox modlobby runs modded games in");
		unsafe {
			let psid = match CreateAppContainerProfile(&wide, &wide, &description, None) {
				Ok(psid) => psid,
				Err(_) => DeriveAppContainerSidFromAppContainerName(&wide)
					.map_err(|err| format!("the AppContainer profile {name}: {err}"))?,
			};
			let bytes = copy(psid);
			FreeSid(psid);
			Ok(Self(bytes))
		}
	}

	/// A pointer into the bytes, valid while `self` is.
	pub(crate) fn as_psid(&self) -> PSID {
		PSID(self.0.as_ptr() as *mut _)
	}
}

unsafe fn copy(psid: PSID) -> Vec<u8> {
	let length = unsafe { GetLengthSid(psid) } as usize;
	unsafe { std::slice::from_raw_parts(psid.0 as *const u8, length) }.to_vec()
}

/// `internetClient`: outbound to the internet, nothing inbound. What a game
/// on a server needs, and no more.
pub(crate) fn internet_client() -> Result<Vec<u8>, String> {
	let mut buffer = vec![0u8; 68];
	let mut size = buffer.len() as u32;
	unsafe {
		CreateWellKnownSid(
			WinCapabilityInternetClientSid,
			None,
			Some(PSID(buffer.as_mut_ptr() as _)),
			&mut size,
		)
		.map_err(|err| format!("the internetClient capability: {err}"))?;
	}
	buffer.truncate(size as usize);
	Ok(buffer)
}
