//! The engine's view of the compositor on Linux, without the clipboard.
//!
//! A Wayland client reads the clipboard through the globals the compositor
//! advertises: the data device manager, and the primary-selection and
//! data-control managers. The focused window is offered the selection, and a
//! game has the focus while it is played, so mod Lua could read whatever was
//! last copied through `Spring.GetClipboard`. The engine is handed a socket
//! of ours instead of the compositor's; [`Filter`] reads every message that
//! crosses it and drops those globals from the registry, so the engine never
//! learns they exist, and ends the connection of a client that binds one by
//! guessing its name.

use std::collections::HashSet;

/// The globals a game's window, input and GPU buffers need, by interface
/// name: everything else a compositor offers -- the clipboard, screen
/// capture, virtual keyboards and pointers, other windows' text input and
/// toplevels -- is hidden.
pub const ALLOWED: &[&str] = &[
	"wl_compositor",
	"wl_subcompositor",
	"wl_shm",
	"wl_seat",
	"wl_output",
	"wl_drm",
	"xdg_wm_base",
	"zxdg_decoration_manager_v1",
	"org_kde_kwin_server_decoration_manager",
	"zxdg_output_manager_v1",
	"zwp_linux_dmabuf_v1",
	"wp_linux_drm_syncobj_manager_v1",
	"wp_viewporter",
	"wp_fractional_scale_manager_v1",
	"wp_presentation",
	"wp_fifo_manager_v1",
	"wp_commit_timing_manager_v1",
	"wp_tearing_control_manager_v1",
	"wp_content_type_manager_v1",
	"wp_cursor_shape_manager_v1",
	"wp_single_pixel_buffer_manager_v1",
	"zwp_relative_pointer_manager_v1",
	"zwp_pointer_constraints_v1",
	"zwp_pointer_gestures_v1",
	"zwp_idle_inhibit_manager_v1",
	"zwp_keyboard_shortcuts_inhibit_manager_v1",
	"zwp_text_input_manager_v3",
	"zwp_tablet_manager_v2",
	"xdg_activation_v1",
	"xdg_toplevel_icon_manager_v1",
];

/// The globals that hand out the clipboard, by interface name: never on
/// [`ALLOWED`], and what the probe asks the registry for.
pub const HIDDEN: &[&str] = &[
	"wl_data_device_manager",
	"zwp_primary_selection_device_manager_v1",
	"gtk_primary_selection_device_manager",
	"zwlr_data_control_manager_v1",
	"ext_data_control_manager_v1",
];

/// One connection's view of the protocol: the bytes of a message not yet
/// whole, each way, the registry objects the client made, and the globals
/// hidden from them, by name and by interface.
#[derive(Debug, Default)]
pub struct Filter {
	from_client: Vec<u8>,
	from_server: Vec<u8>,
	registries: HashSet<u32>,
	hidden: HashSet<u32>,
	/// The interfaces hidden so far: what the proxy's log says, so the first
	/// game that misses one shows which.
	pub dropped: Vec<String>,
}

impl Filter {
	pub fn new() -> Self {
		Self::default()
	}

	/// The client's bytes to pass on: every whole message so far, as sent.
	/// `Err` when the client binds a hidden global, or sends what is not
	/// Wayland; the connection ends.
	pub fn from_client(&mut self, bytes: &[u8]) -> Result<Vec<u8>, String> {
		self.from_client.extend_from_slice(bytes);
		let mut out = Vec::new();
		let mut at = 0;
		while let Some((object, opcode, size)) = next(&self.from_client[at..])? {
			let message = &self.from_client[at..at + size];
			let args = &message[8..];
			if object == DISPLAY && opcode == GET_REGISTRY {
				self.registries.insert(uint_at(args, 0)?);
			}
			if opcode == BIND && self.registries.contains(&object) {
				let name = uint_at(args, 0)?;
				let (interface, _) = string_at(args, 4)?;
				if self.hidden.contains(&name) || !ALLOWED.contains(&interface.as_str()) {
					return Err(format!(
						"the game bound global {name} ({interface}), which is hidden from it"
					));
				}
			}
			out.extend_from_slice(message);
			at += size;
		}
		self.from_client.drain(..at);
		Ok(out)
	}

	/// The compositor's bytes to pass on: every whole message so far, less
	/// the hidden globals' announcements and removals.
	pub fn from_server(&mut self, bytes: &[u8]) -> Result<Vec<u8>, String> {
		self.from_server.extend_from_slice(bytes);
		let mut out = Vec::new();
		let mut at = 0;
		while let Some((object, opcode, size)) = next(&self.from_server[at..])? {
			let message = &self.from_server[at..at + size];
			let args = &message[8..];
			at += size;
			if self.registries.contains(&object) {
				match opcode {
					GLOBAL => {
						let name = uint_at(args, 0)?;
						let (interface, _) = string_at(args, 4)?;
						if !ALLOWED.contains(&interface.as_str()) {
							self.hidden.insert(name);
							self.dropped.push(interface);
							continue;
						}
					}
					GLOBAL_REMOVE if self.hidden.contains(&uint_at(args, 0)?) => continue,
					_ => {}
				}
			}
			if object == DISPLAY && opcode == DELETE_ID {
				self.registries.remove(&uint_at(args, 0)?);
			}
			out.extend_from_slice(message);
		}
		self.from_server.drain(..at);
		Ok(out)
	}
}

/// `wl_display`, always object 1.
const DISPLAY: u32 = 1;
/// `wl_display.get_registry(new_id)`, a request.
const GET_REGISTRY: u16 = 1;
/// `wl_display.delete_id(id)`, an event.
const DELETE_ID: u16 = 1;
/// `wl_registry.bind(name, interface, version, new_id)`, a request.
const BIND: u16 = 0;
/// `wl_registry.global(name, interface, version)`, an event.
const GLOBAL: u16 = 0;
/// `wl_registry.global_remove(name)`, an event.
const GLOBAL_REMOVE: u16 = 1;

/// The whole message at the front of `bytes`, when there is one: its
/// sender, opcode and size. A size no message can have is not Wayland.
fn next(bytes: &[u8]) -> Result<Option<(u32, u16, usize)>, String> {
	if bytes.len() < 8 {
		return Ok(None);
	}
	let object = uint_at(bytes, 0)?;
	let word = uint_at(bytes, 4)?;
	let size = (word >> 16) as usize;
	if size < 8 || !size.is_multiple_of(4) {
		return Err(format!("a message of {size} bytes is not Wayland"));
	}
	if bytes.len() < size {
		return Ok(None);
	}
	Ok(Some((object, (word & 0xffff) as u16, size)))
}

fn uint_at(bytes: &[u8], at: usize) -> Result<u32, String> {
	bytes
		.get(at..at + 4)
		.and_then(|word| word.try_into().ok())
		.map(u32::from_ne_bytes)
		.ok_or_else(|| "a message shorter than its arguments".to_owned())
}

/// The string at `at` and where the next argument starts: its length with
/// the NUL, then it, padded to four bytes.
fn string_at(bytes: &[u8], at: usize) -> Result<(String, usize), String> {
	let len = uint_at(bytes, at)? as usize;
	let start = at + 4;
	if len == 0 {
		return Ok((String::new(), start));
	}
	let text = bytes
		.get(start..start + len - 1)
		.ok_or_else(|| "a string longer than its message".to_owned())?;
	let end = start + ((len + 3) & !3);
	if end > bytes.len() {
		return Err("a string longer than its message".into());
	}
	Ok((String::from_utf8_lossy(text).into_owned(), end))
}

/// How long the proxy waits for the game's first connection.
#[cfg(target_os = "linux")]
const FIRST_PATIENCE: std::time::Duration = std::time::Duration::from_secs(120);

/// How long the proxy outlives its last connection: SDL connects once to
/// see whether Wayland is there and again for the window, and the game must
/// find the proxy still listening in between.
#[cfg(target_os = "linux")]
const LAST_PATIENCE: std::time::Duration = std::time::Duration::from_secs(30);

/// Serves the compositor at `upstream` on `listen` through a [`Filter`] per
/// connection, until the last connection has been gone [`LAST_PATIENCE`],
/// or none came in [`FIRST_PATIENCE`]. Runs as its own process, so a lobby
/// restart leaves a running game its display.
#[cfg(target_os = "linux")]
pub fn serve(listen: &std::path::Path, upstream: &std::path::Path) -> std::io::Result<()> {
	use std::os::unix::net::{UnixListener, UnixStream};
	use std::sync::Arc;
	use std::sync::atomic::{AtomicUsize, Ordering};
	use std::time::Instant;

	let _ = std::fs::remove_file(listen);
	let listener = UnixListener::bind(listen)?;
	listener.set_nonblocking(true)?;
	let live = Arc::new(AtomicUsize::new(0));
	let mut idle_since = Instant::now();
	let mut seen = false;
	loop {
		match listener.accept() {
			Ok((client, _)) => {
				client.set_nonblocking(false)?;
				let server = match UnixStream::connect(upstream) {
					Ok(server) => server,
					Err(err) => {
						eprintln!("the compositor at {} refused: {err}", upstream.display());
						continue;
					}
				};
				seen = true;
				live.fetch_add(1, Ordering::SeqCst);
				let live = Arc::clone(&live);
				std::thread::spawn(move || {
					relay(client, server);
					live.fetch_sub(1, Ordering::SeqCst);
				});
			}
			Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
				if live.load(Ordering::SeqCst) > 0 {
					idle_since = Instant::now();
				} else {
					let patience = if seen { LAST_PATIENCE } else { FIRST_PATIENCE };
					if idle_since.elapsed() > patience {
						let _ = std::fs::remove_file(listen);
						return Ok(());
					}
				}
				std::thread::sleep(std::time::Duration::from_millis(100));
			}
			Err(err) => return Err(err),
		}
	}
}

/// One connection, both ways, until either side ends it or the filter
/// does.
#[cfg(target_os = "linux")]
fn relay(client: std::os::unix::net::UnixStream, server: std::os::unix::net::UnixStream) {
	use std::sync::{Arc, Mutex};

	let filter = Arc::new(Mutex::new(Filter::new()));
	let (Ok(client_back), Ok(server_back)) = (client.try_clone(), server.try_clone()) else {
		return;
	};
	let up = {
		let filter = Arc::clone(&filter);
		std::thread::spawn(move || {
			pump(&client, &server, |bytes| {
				filter
					.lock()
					.map_err(|_| "poisoned".to_owned())?
					.from_client(bytes)
			})
		})
	};
	pump(&server_back, &client_back, |bytes| {
		filter
			.lock()
			.map_err(|_| "poisoned".to_owned())?
			.from_server(bytes)
	});
	let _ = up.join();
	if let Ok(filter) = filter.lock()
		&& !filter.dropped.is_empty()
	{
		eprintln!("hidden from the game: {}", filter.dropped.join(", "));
	}
}

/// Moves what `from` sends to `to` through `filter`, with the descriptors
/// that came with it: held until the bytes they belong with go out, since a
/// stream socket carries descriptors only beside bytes. Ends both sockets
/// when either ends or the filter refuses.
#[cfg(target_os = "linux")]
fn pump(
	from: &std::os::unix::net::UnixStream,
	to: &std::os::unix::net::UnixStream,
	mut filter: impl FnMut(&[u8]) -> Result<Vec<u8>, String>,
) {
	use std::os::fd::OwnedFd;

	let mut held: Vec<OwnedFd> = Vec::new();
	loop {
		let (bytes, fds) = match fds::recv(from) {
			Ok((bytes, fds)) if bytes.is_empty() && fds.is_empty() => break,
			Ok(read) => read,
			Err(_) => break,
		};
		held.extend(fds);
		let passed = match filter(&bytes) {
			Ok(passed) => passed,
			Err(why) => {
				eprintln!("{why}");
				break;
			}
		};
		if passed.is_empty() {
			continue;
		}
		if fds::send(to, &passed, &held).is_err() {
			break;
		}
		// Sent: the other side holds its own copies now.
		held.clear();
	}
	let _ = from.shutdown(std::net::Shutdown::Both);
	let _ = to.shutdown(std::net::Shutdown::Both);
}

/// `recvmsg` and `sendmsg` with `SCM_RIGHTS`, which std has no stable way to
/// do.
#[cfg(target_os = "linux")]
pub(crate) mod fds {
	use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
	use std::os::unix::net::UnixStream;

	/// The most descriptors one read takes: libwayland sends at most 28 per
	/// message, and several messages may arrive at once.
	const MOST: usize = 252;

	fn space(count: usize) -> usize {
		// SAFETY: CMSG_SPACE is arithmetic.
		unsafe { libc::CMSG_SPACE((count * std::mem::size_of::<RawFd>()) as u32) as usize }
	}

	pub fn recv(socket: &UnixStream) -> std::io::Result<(Vec<u8>, Vec<OwnedFd>)> {
		let mut bytes = vec![0u8; 16 * 1024];
		let mut control = vec![0u8; space(MOST)];
		let mut iov = libc::iovec {
			iov_base: bytes.as_mut_ptr().cast(),
			iov_len: bytes.len(),
		};
		// SAFETY: msghdr is plain data; every pointer set below outlives the call.
		let mut header: libc::msghdr = unsafe { std::mem::zeroed() };
		header.msg_iov = &mut iov;
		header.msg_iovlen = 1;
		header.msg_control = control.as_mut_ptr().cast();
		header.msg_controllen = control.len();
		let read = loop {
			// SAFETY: the buffers are as long as the header says.
			let read =
				unsafe { libc::recvmsg(socket.as_raw_fd(), &mut header, libc::MSG_CMSG_CLOEXEC) };
			if read >= 0 {
				break read;
			}
			let err = std::io::Error::last_os_error();
			if err.kind() != std::io::ErrorKind::Interrupted {
				return Err(err);
			}
		};
		bytes.truncate(read as usize);
		let mut fds = Vec::new();
		// SAFETY: the CMSG_* walk stays inside the control buffer the kernel filled.
		unsafe {
			let mut cmsg = libc::CMSG_FIRSTHDR(&header);
			while !cmsg.is_null() {
				if (*cmsg).cmsg_level == libc::SOL_SOCKET && (*cmsg).cmsg_type == libc::SCM_RIGHTS {
					let data = libc::CMSG_DATA(cmsg).cast::<RawFd>();
					let len = (*cmsg).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
					for at in 0..len / std::mem::size_of::<RawFd>() {
						fds.push(OwnedFd::from_raw_fd(data.add(at).read_unaligned()));
					}
				}
				cmsg = libc::CMSG_NXTHDR(&header, cmsg);
			}
		}
		Ok((bytes, fds))
	}

	pub fn send(socket: &UnixStream, bytes: &[u8], fds: &[OwnedFd]) -> std::io::Result<()> {
		let mut sent = 0;
		let mut with_fds = !fds.is_empty();
		while sent < bytes.len() {
			let rest = &bytes[sent..];
			let mut iov = libc::iovec {
				iov_base: rest.as_ptr().cast_mut().cast(),
				iov_len: rest.len(),
			};
			let mut control = vec![0u8; if with_fds { space(fds.len()) } else { 0 }];
			// SAFETY: plain data; the pointers outlive the call.
			let mut header: libc::msghdr = unsafe { std::mem::zeroed() };
			header.msg_iov = &mut iov;
			header.msg_iovlen = 1;
			if with_fds {
				header.msg_control = control.as_mut_ptr().cast();
				header.msg_controllen = control.len();
				// SAFETY: the control buffer has room for one SCM_RIGHTS of fds.len().
				unsafe {
					let cmsg = libc::CMSG_FIRSTHDR(&header);
					(*cmsg).cmsg_level = libc::SOL_SOCKET;
					(*cmsg).cmsg_type = libc::SCM_RIGHTS;
					(*cmsg).cmsg_len =
						libc::CMSG_LEN((fds.len() * std::mem::size_of::<RawFd>()) as u32) as _;
					let data = libc::CMSG_DATA(cmsg).cast::<RawFd>();
					for (at, fd) in fds.iter().enumerate() {
						data.add(at).write_unaligned(fd.as_raw_fd());
					}
				}
			}
			// SAFETY: as above.
			let wrote = unsafe { libc::sendmsg(socket.as_raw_fd(), &header, libc::MSG_NOSIGNAL) };
			if wrote < 0 {
				let err = std::io::Error::last_os_error();
				if err.kind() == std::io::ErrorKind::Interrupted {
					continue;
				}
				return Err(err);
			}
			sent += wrote as usize;
			with_fds = false;
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A message as the wire carries it: sender, size and opcode, then the
	/// arguments already encoded.
	fn message(object: u32, opcode: u16, args: &[u8]) -> Vec<u8> {
		let size = (8 + args.len()) as u32;
		let mut out = Vec::new();
		out.extend(object.to_ne_bytes());
		out.extend(((size << 16) | u32::from(opcode)).to_ne_bytes());
		out.extend(args);
		out
	}

	fn uint(value: u32) -> Vec<u8> {
		value.to_ne_bytes().to_vec()
	}

	/// A string as Wayland writes one: its length with the NUL, then it,
	/// padded to four bytes.
	fn string(text: &str) -> Vec<u8> {
		let mut out = uint(text.len() as u32 + 1);
		out.extend(text.as_bytes());
		out.push(0);
		while !out.len().is_multiple_of(4) {
			out.push(0);
		}
		out
	}

	const REGISTRY: u32 = 2;

	fn get_registry() -> Vec<u8> {
		message(1, 1, &uint(REGISTRY))
	}

	fn global(name: u32, interface: &str, version: u32) -> Vec<u8> {
		let mut args = uint(name);
		args.extend(string(interface));
		args.extend(uint(version));
		message(REGISTRY, 0, &args)
	}

	fn global_remove(name: u32) -> Vec<u8> {
		message(REGISTRY, 1, &uint(name))
	}

	fn bind(name: u32, interface: &str, version: u32, id: u32) -> Vec<u8> {
		let mut args = uint(name);
		args.extend(string(interface));
		args.extend(uint(version));
		args.extend(uint(id));
		message(REGISTRY, 0, &args)
	}

	/// The globals a desktop advertises, the clipboard's among them.
	fn globals() -> Vec<Vec<u8>> {
		vec![
			global(1, "wl_compositor", 6),
			global(2, "wl_data_device_manager", 3),
			global(3, "zwp_primary_selection_device_manager_v1", 1),
			global(4, "wl_seat", 9),
			global(5, "ext_data_control_manager_v1", 1),
		]
	}

	/// A filter that has seen the client ask for the registry and the
	/// compositor answer.
	fn introduced() -> (Filter, Vec<u8>) {
		let mut filter = Filter::new();
		assert_eq!(filter.from_client(&get_registry()).unwrap(), get_registry());
		let passed = filter.from_server(&globals().concat()).unwrap();
		(filter, passed)
	}

	#[test]
	fn clipboard_globals_are_hidden_and_the_rest_pass() {
		let (_, passed) = introduced();
		assert_eq!(
			passed,
			[global(1, "wl_compositor", 6), global(4, "wl_seat", 9)].concat()
		);
	}

	#[test]
	fn binding_a_hidden_global_ends_the_connection() {
		let (mut filter, _) = introduced();
		let seat = bind(4, "wl_seat", 9, 7);
		assert_eq!(filter.from_client(&seat).unwrap(), seat);
		let err = filter
			.from_client(&bind(2, "wl_data_device_manager", 3, 8))
			.unwrap_err();
		assert!(
			err.contains("wl_data_device_manager") || err.contains("hidden"),
			"{err}"
		);
		// A guess under another interface's name is the same guess.
		let (mut filter, _) = introduced();
		assert!(filter.from_client(&bind(5, "wl_seat", 1, 8)).is_err());
	}

	#[test]
	fn a_message_split_across_reads_is_framed() {
		let mut filter = Filter::new();
		let mut passed = Vec::new();
		for byte in get_registry() {
			passed.extend(filter.from_client(&[byte]).unwrap());
		}
		assert_eq!(passed, get_registry());
		let mut passed = Vec::new();
		for byte in globals().concat() {
			passed.extend(filter.from_server(&[byte]).unwrap());
		}
		assert_eq!(
			passed,
			[global(1, "wl_compositor", 6), global(4, "wl_seat", 9)].concat()
		);
	}

	#[test]
	fn a_hidden_globals_removal_is_hidden_too() {
		let (mut filter, _) = introduced();
		assert!(filter.from_server(&global_remove(2)).unwrap().is_empty());
		assert_eq!(
			filter.from_server(&global_remove(4)).unwrap(),
			global_remove(4)
		);
	}

	/// Events of other objects pass whatever their opcode, and so do a
	/// registry's from before the filter knew it was one.
	#[test]
	fn other_objects_pass_untouched() {
		let mut filter = Filter::new();
		let done = message(3, 0, &uint(42));
		assert_eq!(filter.from_server(&done).unwrap(), done);
	}

	#[test]
	fn what_is_not_wayland_ends_the_connection() {
		let mut filter = Filter::new();
		// A size smaller than the header itself.
		let mut broken = uint(1);
		broken.extend(((4u32 << 16) | 1).to_ne_bytes());
		assert!(filter.from_client(&broken).is_err());
		let (mut filter, _) = introduced();
		// A global whose string runs past its message.
		let mut args = uint(9);
		args.extend(uint(400));
		args.extend(b"wl_d");
		assert!(filter.from_server(&message(REGISTRY, 0, &args)).is_err());
	}

	/// Through the relay, a message with a descriptor reaches the compositor
	/// with that descriptor, and the compositor's globals come back
	/// filtered.
	#[cfg(target_os = "linux")]
	#[test]
	fn fds_travel_with_their_bytes() {
		use std::io::{Read as _, Seek as _, Write as _};
		use std::os::fd::OwnedFd;
		use std::os::unix::net::UnixStream;

		let (game, proxy_client) = UnixStream::pair().unwrap();
		let (proxy_server, compositor) = UnixStream::pair().unwrap();
		let relayed = std::thread::spawn(move || relay(proxy_client, proxy_server));

		let mut shared = tempfile::tempfile().unwrap();
		shared.write_all(b"pool").unwrap();
		let fd: OwnedFd = shared.try_clone().unwrap().into();
		let asked = [get_registry(), message(5, 2, &uint(7))].concat();
		fds::send(&game, &asked, &[fd]).unwrap();
		let (bytes, fds) = fds::recv(&compositor).unwrap();
		assert_eq!(bytes, asked);
		assert_eq!(fds.len(), 1);
		let mut through = std::fs::File::from(fds.into_iter().next().unwrap());
		through.rewind().unwrap();
		let mut held = String::new();
		through.read_to_string(&mut held).unwrap();
		assert_eq!(held, "pool", "the descriptor is the same file");

		fds::send(&compositor, &globals().concat(), &[]).unwrap();
		let mut back = Vec::new();
		let want = [global(1, "wl_compositor", 6), global(4, "wl_seat", 9)].concat();
		while back.len() < want.len() {
			let (bytes, _) = fds::recv(&game).unwrap();
			assert!(!bytes.is_empty(), "the relay closed early");
			back.extend(bytes);
		}
		assert_eq!(back, want);
		drop(game);
		drop(compositor);
		relayed.join().unwrap();
	}

	/// Only what a game's window needs is offered: a global off the allowlist
	/// -- screen capture, a virtual keyboard, another window's text input --
	/// is hidden like the clipboard, and binding it by name ends the
	/// connection.
	#[test]
	fn a_global_off_the_allowlist_is_hidden_too() {
		let mut filter = Filter::new();
		filter.from_client(&get_registry()).unwrap();
		let offered = [
			global(1, "wl_compositor", 6),
			global(6, "zwlr_screencopy_manager_v1", 3),
			global(7, "zwp_virtual_keyboard_manager_v1", 1),
			global(8, "xdg_wm_base", 6),
			global(9, "zwp_input_method_manager_v2", 1),
		]
		.concat();
		assert_eq!(
			filter.from_server(&offered).unwrap(),
			[global(1, "wl_compositor", 6), global(8, "xdg_wm_base", 6)].concat()
		);
		assert!(
			filter
				.from_client(&bind(6, "zwlr_screencopy_manager_v1", 3, 9))
				.is_err()
		);
		assert!(
			ALLOWED.iter().all(|allowed| !HIDDEN.contains(allowed)),
			"no clipboard global is allowed"
		);
	}
}
