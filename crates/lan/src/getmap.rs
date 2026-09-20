//! Fetching the room's map from the host, when no search on the internet had
//! it.
//!
//! The last resort, and the only one that can work for a map nobody
//! publishes: the host is playing it, so the host has the file. The request
//! names no file — it says who is asking and proves it with the script
//! password that member gave at `JOINBATTLE` — and the host answers with
//! whatever its room is hosting. See [`crate::serve`] for the other end.
//!
//! What arrives is written beside the maps under a `.part` name and put in
//! place only once every byte the header promised has landed, so a dropped
//! connection leaves nothing the engine would try to read.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

/// The most a map may be. BAR's largest is a few hundred megabytes; past this
/// it is not a map, and a host that says so is not one to keep reading from.
const MOST: u64 = 2 << 30;

/// How much the header may be, so a host that answers with an endless line
/// cannot be read forever.
const MOST_HEADER: u64 = 4 * 1024;

/// Told as it arrives: bytes so far, and what the host said there would be.
pub type Progress = dyn Fn(u64, u64) + Send + Sync;

/// Fetches the room's map from `host` into `maps_dir`, answering with the
/// file name it wrote.
///
/// `name` and `script_password` are ours in that room; a host that does not
/// know them both answers `NOMAP` and this says so.
pub async fn fetch(
	host: std::net::SocketAddr,
	name: &str,
	script_password: &str,
	maps_dir: &Path,
	say: &Progress,
) -> Result<String, String> {
	let stream = TcpStream::connect(host)
		.await
		.map_err(|err| format!("{host}: {err}"))?;
	let _ = stream.set_nodelay(true);
	let (read, mut write) = stream.into_split();
	let mut reader = BufReader::new(read);

	// The host greets every connection before it hears anything; ours is
	// asked for and then read past.
	let mut greeting = String::new();
	read_line(&mut reader, &mut greeting).await?;
	write
		.write_all(format!("{} {name} {script_password}\n", crate::serve::GET_MAP_FILE).as_bytes())
		.await
		.map_err(|err| format!("asking for the map: {err}"))?;

	let mut header = String::new();
	read_line(&mut reader, &mut header).await?;
	let (size, archive) = parse_header(&header)?;

	let path = maps_dir.join(&archive);
	let part = maps_dir.join(format!("{archive}.part"));
	if let Some(parent) = part.parent() {
		tokio::fs::create_dir_all(parent)
			.await
			.map_err(|err| format!("{}: {err}", parent.display()))?;
	}
	match take(&mut reader, &part, size, say).await {
		Ok(()) => {}
		Err(err) => {
			// Half a map is worse than none: the engine would read it.
			let _ = tokio::fs::remove_file(&part).await;
			return Err(err);
		}
	}
	tokio::fs::rename(&part, &path)
		.await
		.map_err(|err| format!("{}: {err}", path.display()))?;
	Ok(archive)
}

/// `MAPFILE <bytes> <archive name>`, or `NOMAP <reason>`.
///
/// The name is checked here rather than trusted: it decides what is written
/// to the disk, and it came off the network. A plain file name with a map's
/// extension, or nothing.
fn parse_header(line: &str) -> Result<(u64, String), String> {
	let mut parts = line.trim_end().splitn(3, ' ');
	match (parts.next(), parts.next(), parts.next()) {
		(Some("MAPFILE"), Some(size), Some(archive)) => {
			let size: u64 = size
				.parse()
				.map_err(|_| format!("the host offered {size} bytes"))?;
			if size == 0 || size > MOST {
				return Err(format!("the host offered {size} bytes"));
			}
			if !plain_archive(archive) {
				return Err(format!("the host offered a file called {archive}"));
			}
			Ok((size, archive.to_owned()))
		}
		(Some("NOMAP"), reason, rest) => Err(format!(
			"the host will not send it: {}",
			[reason, rest]
				.into_iter()
				.flatten()
				.collect::<Vec<_>>()
				.join(" ")
		)),
		_ => Err(format!("the host answered {line:?}")),
	}
}

/// A file name and nothing else: no directory of its own to climb into, no
/// drive, no name the file system would read as somewhere else.
fn plain_archive(name: &str) -> bool {
	let lower = name.to_ascii_lowercase();
	Path::new(name).file_name().and_then(|n| n.to_str()) == Some(name)
		&& !name.contains(['/', '\\', ':'])
		&& name != ".."
		&& (lower.ends_with(".sd7") || lower.ends_with(".sdz"))
}

/// Exactly `size` bytes into `part`, and a short answer is a failure: the
/// length is the only thing that says the file arrived whole.
async fn take(
	reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
	part: &PathBuf,
	size: u64,
	say: &Progress,
) -> Result<(), String> {
	let mut file = tokio::fs::File::create(part)
		.await
		.map_err(|err| format!("{}: {err}", part.display()))?;
	let mut left = size;
	let mut chunk = vec![0_u8; 64 * 1024];
	while left > 0 {
		let want = chunk.len().min(left as usize);
		let read = reader
			.read(&mut chunk[..want])
			.await
			.map_err(|err| format!("reading the map: {err}"))?;
		if read == 0 {
			return Err(format!(
				"the map stopped after {} of {size} bytes",
				size - left
			));
		}
		file.write_all(&chunk[..read])
			.await
			.map_err(|err| format!("{}: {err}", part.display()))?;
		left -= read as u64;
		say(size - left, size);
	}
	file.flush()
		.await
		.map_err(|err| format!("{}: {err}", part.display()))
}

async fn read_line(
	reader: &mut BufReader<tokio::net::tcp::OwnedReadHalf>,
	into: &mut String,
) -> Result<(), String> {
	let read = reader
		.take(MOST_HEADER)
		.read_line(into)
		.await
		.map_err(|err| format!("reading the host's answer: {err}"))?;
	if read == 0 {
		return Err("the host said nothing".into());
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_header_is_read_and_a_refusal_is_said_as_one() {
		assert_eq!(
			parse_header("MAPFILE 1234 frostycove_v1.13.sd7\n").unwrap(),
			(1234, "frostycove_v1.13.sd7".to_owned())
		);
		let refused = parse_header("NOMAP not in this room\n").unwrap_err();
		assert!(refused.contains("not in this room"), "{refused}");
		assert!(parse_header("TASSERVER 0.38").is_err());
	}

	/// The name off the wire decides what is written to the disk, so it is
	/// the one thing here that must not be taken at its word.
	#[test]
	fn a_name_that_is_not_a_plain_map_file_is_refused() {
		for bad in [
			"../../evil.sd7",
			"..\\evil.sd7",
			"C:\\windows\\evil.sd7",
			"/etc/evil.sd7",
			"maps/evil.sd7",
			"evil.exe",
			"evil.sd7.exe",
			"..",
		] {
			assert!(!plain_archive(bad), "{bad} was allowed");
			assert!(parse_header(&format!("MAPFILE 10 {bad}")).is_err(), "{bad}");
		}
		assert!(plain_archive("frostycove_v1.13.sd7"));
		assert!(plain_archive("Some Map V2.sdz"));
		// Nothing at all, and more than a map could be.
		assert!(parse_header("MAPFILE 0 fine.sd7").is_err());
		assert!(parse_header("MAPFILE 999999999999 fine.sd7").is_err());
	}
}
