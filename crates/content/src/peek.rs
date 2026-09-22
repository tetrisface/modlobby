//! One file out of a zip on a web server, read with range requests rather
//! than downloaded: a zip's index sits at its end, so a game's `modinfo.lua`
//! costs a few requests and about a megabyte of an 800 MB `.sdz`
//! (SplinterFaction 0.1.86, 2026-09-22). What a release calls itself is then
//! known before the release is fetched.

use std::io::Read;

/// The most of a zip's tail read to find its end record: the record and the
/// longest comment a zip may carry.
const TAIL: u64 = 22 + 65_535;

/// The most of an index read: a game of tens of thousands of files.
const MOST_INDEX: u64 = 16 << 20;

/// The most one peeked file may be, packed or not.
const MOST_FILE: u64 = 1 << 20;

/// `wanted` at the root of the zip at `url`, compared without case. An error
/// for a server that does not serve ranges, or for anything that is not a
/// zip it can read.
pub async fn root_file(http: &reqwest::Client, url: &str, wanted: &str) -> Result<Vec<u8>, String> {
	let (tail, size) = suffix(http, url, TAIL).await?;
	let end = find_last(&tail, b"PK\x05\x06").ok_or("no zip end record")?;
	let record = &tail[end..];
	let (mut index_size, mut index_at) =
		(u64::from(le32(record, 12)?), u64::from(le32(record, 16)?));
	if index_size == u64::from(u32::MAX) || index_at == u64::from(u32::MAX) {
		// Zip64: the locator just before the end record says where the larger
		// end record is.
		let locator = end.checked_sub(20).ok_or("no zip64 locator")?;
		if tail.get(locator..locator + 4) != Some(b"PK\x06\x07") {
			return Err("no zip64 locator".into());
		}
		let at = le64(&tail, locator + 8)?;
		let big = range(http, url, at, 56).await?;
		if big.get(..4) != Some(b"PK\x06\x06") {
			return Err("no zip64 end record".into());
		}
		index_size = le64(&big, 40)?;
		index_at = le64(&big, 48)?;
	}
	if index_size > MOST_INDEX || index_at + index_size > size {
		return Err("an index larger than a game's".into());
	}
	let index = range(http, url, index_at, index_size).await?;
	let entry = find_entry(&index, wanted)?.ok_or_else(|| format!("no {wanted} at its root"))?;
	if entry.packed > MOST_FILE || entry.size > MOST_FILE {
		return Err(format!("{wanted} is larger than one would be"));
	}
	let header = range(http, url, entry.at, 30).await?;
	if header.get(..4) != Some(b"PK\x03\x04") {
		return Err("no local header where the index says".into());
	}
	let skip = 30 + u64::from(le16(&header, 26)?) + u64::from(le16(&header, 28)?);
	let packed = range(http, url, entry.at + skip, entry.packed).await?;
	match entry.method {
		0 => Ok(packed),
		8 => {
			let mut out = Vec::new();
			flate2::read::DeflateDecoder::new(packed.as_slice())
				.take(MOST_FILE)
				.read_to_end(&mut out)
				.map_err(|err| err.to_string())?;
			Ok(out)
		}
		other => Err(format!(
			"{wanted} is packed a way this does not read ({other})"
		)),
	}
}

/// One file as the index lists it.
struct Entry {
	method: u16,
	packed: u64,
	size: u64,
	at: u64,
}

/// `wanted` among the index's entries, its sizes and place read from the
/// zip64 extra field where the plain ones are full.
fn find_entry(index: &[u8], wanted: &str) -> Result<Option<Entry>, String> {
	let mut at = 0;
	while index.get(at..at + 4) == Some(b"PK\x01\x02") {
		let name_len = usize::from(le16(index, at + 28)?);
		let extra_len = usize::from(le16(index, at + 30)?);
		let comment_len = usize::from(le16(index, at + 32)?);
		let name = index
			.get(at + 46..at + 46 + name_len)
			.ok_or("an index cut short")?;
		let name = String::from_utf8_lossy(name);
		if name.trim_start_matches('/').eq_ignore_ascii_case(wanted) {
			let mut entry = Entry {
				method: le16(index, at + 10)?,
				packed: u64::from(le32(index, at + 20)?),
				size: u64::from(le32(index, at + 24)?),
				at: u64::from(le32(index, at + 42)?),
			};
			let extra = index
				.get(at + 46 + name_len..at + 46 + name_len + extra_len)
				.ok_or("an index cut short")?;
			zip64_sizes(extra, &mut entry)?;
			return Ok(Some(entry));
		}
		at += 46 + name_len + extra_len + comment_len;
	}
	Ok(None)
}

/// The zip64 extra field (id 1) holds, in order, whichever of the size, the
/// packed size and the offset did not fit in 32 bits.
fn zip64_sizes(mut extra: &[u8], entry: &mut Entry) -> Result<(), String> {
	while extra.len() >= 4 {
		let (id, len) = (le16(extra, 0)?, usize::from(le16(extra, 2)?));
		let body = extra.get(4..4 + len).ok_or("an extra field cut short")?;
		if id == 1 {
			let mut values = body.chunks_exact(8).map(|eight| le64(eight, 0));
			let full = u64::from(u32::MAX);
			for field in [&mut entry.size, &mut entry.packed, &mut entry.at] {
				if *field == full {
					*field = values.next().ok_or("a zip64 field missing")??;
				}
			}
		}
		extra = &extra[4 + len..];
	}
	Ok(())
}

/// The last `len` bytes and the whole file's size. The size is asked for
/// with the first byte, since GitHub's release host refuses a suffix range
/// (`bytes=-N`, 501 as of 2026-09-22) and serves an explicit one.
async fn suffix(http: &reqwest::Client, url: &str, len: u64) -> Result<(Vec<u8>, u64), String> {
	let first = ranged(http, url, "bytes=0-0".into()).await?;
	let size: u64 = first
		.headers()
		.get(reqwest::header::CONTENT_RANGE)
		.and_then(|value| value.to_str().ok())
		.and_then(|value| value.rsplit('/').next()?.parse().ok())
		.ok_or("no size in the answer")?;
	drop(first);
	let at = size.saturating_sub(len);
	Ok((range(http, url, at, size - at).await?, size))
}

/// `len` bytes from `at`.
async fn range(http: &reqwest::Client, url: &str, at: u64, len: u64) -> Result<Vec<u8>, String> {
	if len == 0 {
		return Ok(Vec::new());
	}
	let response = ranged(http, url, format!("bytes={at}-{}", at + len - 1)).await?;
	let body = response.bytes().await.map_err(|err| err.to_string())?;
	(body.len() as u64 == len)
		.then(|| body.to_vec())
		.ok_or_else(|| "the server sent another range".into())
}

/// A range request, answered as one: a server that sends the whole file
/// instead is let go before any of it is read.
async fn ranged(
	http: &reqwest::Client,
	url: &str,
	range: String,
) -> Result<reqwest::Response, String> {
	let response = http
		.get(url)
		.header(reqwest::header::RANGE, range)
		.header(reqwest::header::ACCEPT_ENCODING, "identity")
		.send()
		.await
		.map_err(|err| err.to_string())?;
	if response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
		return Err(format!("no ranges served ({})", response.status()));
	}
	Ok(response)
}

fn find_last(haystack: &[u8], needle: &[u8]) -> Option<usize> {
	haystack
		.windows(needle.len())
		.rposition(|window| window == needle)
}

fn le16(bytes: &[u8], at: usize) -> Result<u16, String> {
	let two = bytes.get(at..at + 2).ok_or("a record cut short")?;
	Ok(u16::from_le_bytes([two[0], two[1]]))
}

fn le32(bytes: &[u8], at: usize) -> Result<u32, String> {
	let four = bytes.get(at..at + 4).ok_or("a record cut short")?;
	Ok(u32::from_le_bytes(four.try_into().expect("four bytes")))
}

fn le64(bytes: &[u8], at: usize) -> Result<u64, String> {
	let eight = bytes.get(at..at + 8).ok_or("a record cut short")?;
	Ok(u64::from_le_bytes(eight.try_into().expect("eight bytes")))
}

#[cfg(test)]
pub(crate) mod tests {
	use std::io::Write;

	use wiremock::matchers::method;
	use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

	use super::*;

	/// A file served as a server with ranges serves it.
	pub(crate) struct Ranged(pub Vec<u8>);

	impl Respond for Ranged {
		fn respond(&self, request: &Request) -> ResponseTemplate {
			let whole = &self.0;
			let Some(range) = request
				.headers
				.get("range")
				.and_then(|value| value.to_str().ok())
				.and_then(|value| value.strip_prefix("bytes="))
			else {
				return ResponseTemplate::new(200).set_body_bytes(whole.clone());
			};
			let len = whole.len();
			let (start, end) = match range.split_once('-').unwrap() {
				("", last) => (len.saturating_sub(last.parse().unwrap()), len - 1),
				(start, "") => (start.parse().unwrap(), len - 1),
				(start, end) => (
					start.parse().unwrap(),
					end.parse::<usize>().unwrap().min(len - 1),
				),
			};
			ResponseTemplate::new(206)
				.insert_header(
					"content-range",
					format!("bytes {start}-{end}/{len}").as_str(),
				)
				.set_body_bytes(whole[start..=end].to_vec())
		}
	}

	pub(crate) fn game_zip(modinfo: &str, deflated: bool) -> Vec<u8> {
		let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
		let method = if deflated {
			zip::CompressionMethod::Deflated
		} else {
			zip::CompressionMethod::Stored
		};
		let options = zip::write::SimpleFileOptions::default().compression_method(method);
		zip.start_file("units/big.lua", options).unwrap();
		zip.write_all(&vec![b'x'; 200_000]).unwrap();
		zip.start_file("modinfo.lua", options).unwrap();
		zip.write_all(modinfo.as_bytes()).unwrap();
		zip.set_comment("a comment, as some packers leave");
		zip.finish().unwrap().into_inner()
	}

	#[tokio::test]
	async fn a_root_file_is_read_out_of_a_zip_by_ranges() {
		let http = crate::http::client("test");
		for deflated in [true, false] {
			let server = MockServer::start().await;
			let modinfo = "name = 'SplinterFaction'\nversion = '0.1.86'\n";
			Mock::given(method("GET"))
				.respond_with(Ranged(game_zip(modinfo, deflated)))
				.mount(&server)
				.await;
			let url = format!("{}/SF.sdz", server.uri());
			assert_eq!(
				root_file(&http, &url, "modinfo.lua").await.unwrap(),
				modinfo.as_bytes()
			);
			assert!(root_file(&http, &url, "mapinfo.lua").await.is_err());
		}
	}

	#[tokio::test]
	async fn a_server_without_ranges_is_let_go() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(200).set_body_bytes(game_zip("x", true)))
			.mount(&server)
			.await;
		let http = crate::http::client("test");
		let said = root_file(&http, &format!("{}/SF.sdz", server.uri()), "modinfo.lua")
			.await
			.unwrap_err();
		assert!(said.starts_with("no ranges served"), "{said}");
	}
}
