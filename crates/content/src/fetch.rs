//! A large file to disk: streamed, resumed where a previous attempt stopped,
//! and checked once it has arrived. The engine and games both come this way.

use std::path::Path;
use std::time::Duration;

use md5::Digest;
use tokio::io::AsyncWriteExt;

/// How much has to arrive before progress is told again.
///
/// Chunks arrive in tens of kilobytes, so one report each would be tens of
/// thousands of IPC messages for one engine — a progress bar nobody can see
/// moving that fast, at the cost of the UI thread that has to drain them.
pub const REPORT_EVERY: u64 = 4 * 1024 * 1024;

/// The most one download may be. An address from a list is anyone's to
/// write, and one that never stops answering would otherwise fill the disk;
/// the largest real engine or game is a fraction of this.
pub const MOST: u64 = 8 << 30;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// Nothing, or not all of it, came back.
	#[error("{0}")]
	Network(String),
	#[error("{0}")]
	Io(String),
	/// The server answered, and not with the file.
	#[error("the server answered {0}")]
	Status(reqwest::StatusCode),
}

/// Streams `url` to `into`, picking up where a previous attempt left off.
///
/// Written to disk chunk by chunk rather than collected first: an engine is a
/// few hundred megabytes and a game can be most of a gigabyte. What is already
/// on disk is asked for with `Range`; a server that answers 206 continues it,
/// one that answers 200 did not understand and starts over. `report` hears
/// `(got, total)` every [`REPORT_EVERY`] bytes and once at the end.
pub async fn resumable(
	http: &reqwest::Client,
	url: &str,
	into: &Path,
	expected: u64,
	mut report: impl FnMut(u64, u64),
) -> Result<(), Error> {
	let have = tokio::fs::metadata(into)
		.await
		.map(|meta| meta.len())
		.unwrap_or(0);
	// The file as it is on the server, byte for byte: the offset resumed
	// from, the size quoted and the checksum given are all of the stored
	// file, and the client otherwise asks for gzip on every request. A server
	// that compressed the answer would make a resume append the wrong bytes
	// at the wrong offset, and the checksum would then reject the whole
	// download rather than the server.
	let mut request = http
		.get(url)
		.header(reqwest::header::ACCEPT_ENCODING, "identity");
	if have > 0 {
		request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
	}
	let mut response = request
		.send()
		.await
		.map_err(|err| Error::Network(format!("fetching {url}: {err}")))?;

	let status = response.status();
	let io = |err| Error::Io(format!("opening {}: {err}", into.display()));
	let (mut file, mut got) = if status == reqwest::StatusCode::PARTIAL_CONTENT && have > 0 {
		tracing::info!(have, path = %into.display(), "resuming a download");
		let file = tokio::fs::OpenOptions::new()
			.append(true)
			.open(into)
			.await
			.map_err(io)?;
		(file, have)
	} else if status.is_success() {
		(tokio::fs::File::create(into).await.map_err(io)?, 0)
	} else {
		return Err(Error::Status(status));
	};

	// The size quoted beforehand, but the response's own is the one that
	// matches what is arriving — and after a resume it counts only the rest.
	let total = match response.content_length() {
		Some(remaining) => got + remaining,
		None => expected.max(got),
	};
	let mut reported = got;

	// `chunk` rather than a stream, so reqwest needs no extra feature and this
	// needs no futures crate for one loop.
	while let Some(chunk) = response
		.chunk()
		.await
		.map_err(|err| Error::Network(format!("the download stopped: {err}")))?
	{
		file.write_all(&chunk)
			.await
			.map_err(|err| Error::Io(format!("writing {}: {err}", into.display())))?;
		got += chunk.len() as u64;
		if got > MOST {
			return Err(Error::Network(format!(
				"{url} sent more than {} GiB; stopped",
				MOST >> 30
			)));
		}
		if got - reported >= REPORT_EVERY {
			reported = got;
			report(got, total);
		}
	}

	file.flush()
		.await
		.map_err(|err| Error::Io(format!("finishing {}: {err}", into.display())))?;

	// The bar should read full before whatever comes next starts, whatever
	// the last reporting threshold happened to land on.
	report(got, total);
	Ok(())
}

/// A file's digest in hex, read in chunks.
pub fn hash_file<D: Digest>(path: &Path) -> std::io::Result<String> {
	use std::io::Read;
	let mut file = std::fs::File::open(path)?;
	let mut hasher = D::new();
	let mut chunk = [0_u8; 64 * 1024];
	loop {
		let read = file.read(&mut chunk)?;
		if read == 0 {
			break;
		}
		hasher.update(&chunk[..read]);
	}
	Ok(hex(&hasher.finalize()))
}

pub fn hex(bytes: &[u8]) -> String {
	bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Removes staging files and folders (`.<name>.part`) older than `after`, so
/// a download or a build abandoned long ago does not sit in `dir` forever. A
/// recent one is left for resuming.
pub fn sweep_stale_parts(dir: &Path, after: Duration) {
	let Ok(entries) = std::fs::read_dir(dir) else {
		return;
	};
	for entry in entries.filter_map(std::result::Result::ok) {
		let path = entry.path();
		let name = entry.file_name();
		let name = name.to_string_lossy();
		if !(name.starts_with('.') && name.ends_with(".part")) {
			continue;
		}
		let stale = entry
			.metadata()
			.and_then(|meta| meta.modified())
			.ok()
			.and_then(|modified| modified.elapsed().ok())
			.is_some_and(|age| age > after);
		if stale {
			tracing::info!(path = %path.display(), "removing an abandoned download");
			let _ = if path.is_dir() {
				std::fs::remove_dir_all(&path)
			} else {
				std::fs::remove_file(&path)
			};
		}
	}
}
