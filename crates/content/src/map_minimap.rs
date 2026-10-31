//! The picture every map carries: the minimap inside its `.smf`.
//!
//! BAR's index publishes a picture for the version of a map it lists now, and
//! a new version takes that row over. So an older version has no published
//! picture, and neither has a map nobody publishes. Both still have this one,
//! which also says exactly what that version of the map looks like.
//!
//! The minimap is always a 1024 square in DXT1 (a GPU texture format of four
//! bits a pixel), whatever shape the map is, so it is stretched back to that
//! shape here. It sits a few megabytes into the `.smf`, ahead of the tiles that
//! make up most of an archive, so only those first megabytes are decompressed.

use std::io::{self, Read};
use std::path::Path;

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{Pixel, Rgb, RgbImage};

/// A minimap's side in pixels: the `.smf` format fixes it.
const SIDE: u32 = 1024;
/// Four bits a pixel. Eight smaller levels follow, which nothing here wants.
const TOP_LEVEL: usize = (SIDE as usize * SIDE as usize) / 2;
/// The size of `SMFHeader` in the engine's `SMFFormat.h`, and where in it the
/// fields read here sit.
const HEADER: usize = 80;
const WIDTH: usize = 24;
const HEIGHT: usize = 28;
const MINIMAP_POINTER: usize = 64;
const MAGIC: &[u8; 16] = b"spring map file\0";
/// How much is decompressed looking for a minimap. Real ones are found within
/// megabytes, and a map is somebody else's file: this is what stops one built
/// to keep a core busy for as long as it likes.
const MOST_DECODED: u64 = 1 << 30;
/// Enough that resizing from it shows no blocks. A map's picture is kept for
/// good, and as a PNG it would take several times the room.
const JPEG_QUALITY: u8 = 90;

#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// The file could not be opened, which may be gone in a moment.
	#[error("{0}")]
	Io(#[from] std::io::Error),
	/// What the file holds is no use, and will not be until it changes.
	#[error("{0}")]
	Unreadable(String),
}

fn unreadable(why: impl ToString) -> Error {
	Error::Unreadable(why.to_string())
}

fn no_smf() -> Error {
	unreadable("not a map: no .smf inside")
}

/// The minimap in the map at `path` (`.sd7`, `.sdz` or an unpacked `.sdd`) as
/// a picture in the map's own shape.
pub fn picture_in(path: &Path) -> Result<Vec<u8>, Error> {
	let extension = path
		.extension()
		.and_then(|extension| extension.to_str())
		.unwrap_or_default()
		.to_ascii_lowercase();
	let minimap = match extension.as_str() {
		"sd7" => from_sd7(path),
		"sdz" => from_sdz(path),
		"sdd" => from_directory(path),
		_ => Err(no_smf()),
	}?;
	shaped(&minimap)
}

fn is_smf(name: &str) -> bool {
	crate::map_name::smf_name(name).is_some()
}

fn from_sd7(path: &Path) -> Result<Minimap, Error> {
	let mut source = std::fs::File::open(path)?;
	let password = sevenz_rust2::Password::empty();
	let archive = sevenz_rust2::Archive::open_with_password(path, &password).map_err(unreadable)?;
	let smf = archive
		.files
		.iter()
		.position(|file| is_smf(file.name()))
		.ok_or_else(no_smf)?;
	// Only the block it is in: a map packed a file at a time, as nearly all
	// are, costs what its own `.smf` costs. `ArchiveReader::for_each_entries`
	// would go on to set up every other block, even told to stop.
	let block = archive.stream_map.file_block_index[smf].ok_or_else(no_smf)?;
	let mut decoded = 0;
	let mut minimap = Err(no_smf());
	sevenz_rust2::BlockDecoder::new(1, block, &archive, &password, &mut source)
		.for_each_entries(&mut |entry, reader| {
			if is_smf(entry.name()) {
				minimap = from_smf(reader);
				return Ok(false);
			}
			// A solid block has nothing to seek to: the next entry starts
			// where this one's decoding stops.
			decoded += io::copy(&mut reader.take(MOST_DECODED - decoded), &mut io::sink())?;
			if decoded == MOST_DECODED {
				minimap = Err(unreadable("the minimap is further in than any real one"));
				return Ok(false);
			}
			Ok(true)
		})
		.map_err(unreadable)?;
	minimap
}

fn from_sdz(path: &Path) -> Result<Minimap, Error> {
	let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?).map_err(unreadable)?;
	let name = zip
		.file_names()
		.filter_map(Result::ok)
		.find(|name| is_smf(name))
		.ok_or_else(no_smf)?
		.into_owned();
	from_smf(zip.by_name(&name).map_err(unreadable)?)
}

/// An unpacked map is a directory, and its files are just files.
fn from_directory(path: &Path) -> Result<Minimap, Error> {
	let smf = std::fs::read_dir(path.join("maps"))?
		.filter_map(Result::ok)
		.find(|entry| entry.file_name().to_str().is_some_and(is_smf))
		.ok_or_else(no_smf)?;
	from_smf(std::fs::File::open(smf.path())?)
}

/// A minimap as stored, and the shape of the map it belongs to.
struct Minimap {
	width: u32,
	height: u32,
	top: Vec<u8>,
}

/// Reads as far into an `.smf` as its minimap and no further. Every number in
/// the header is checked, because all of them are whoever made the map's.
fn from_smf(mut smf: impl Read) -> Result<Minimap, Error> {
	let mut head = [0u8; HEADER];
	smf.read_exact(&mut head)
		.map_err(|_| unreadable("not a map: shorter than a header"))?;
	if !head.starts_with(MAGIC) {
		return Err(unreadable("not a map: the .smf has the wrong magic"));
	}
	let positive = |at: usize| {
		let bytes = head[at..at + 4].try_into().expect("four bytes");
		u32::try_from(i32::from_le_bytes(bytes))
			.ok()
			.filter(|value| *value > 0)
			.map(u64::from)
			.ok_or_else(|| unreadable("the .smf header says something impossible"))
	};
	let (width, height, at) = (
		positive(WIDTH)?,
		positive(HEIGHT)?,
		positive(MINIMAP_POINTER)?,
	);
	if at > MOST_DECODED {
		return Err(unreadable("the minimap is further in than any real one"));
	}
	let Some(gap) = at.checked_sub(HEADER as u64) else {
		return Err(unreadable("the .smf says its minimap is inside its header"));
	};
	io::copy(&mut smf.by_ref().take(gap), &mut io::sink()).map_err(unreadable)?;
	let mut top = vec![0; TOP_LEVEL];
	smf.read_exact(&mut top).map_err(unreadable)?;
	Ok(Minimap {
		width: width as u32,
		height: height as u32,
		top,
	})
}

/// A 5-6-5 colour, each channel scaled up to the full eight bits.
fn expand(colour: u16) -> Rgb<u8> {
	let scale = |value: u16, most: u32| (u32::from(value) * 255 / most) as u8;
	Rgb([
		scale(colour >> 11, 31),
		scale((colour >> 5) & 63, 63),
		scale(colour & 31, 31),
	])
}

fn blend(a: Rgb<u8>, b: Rgb<u8>, (of_a, of_b): (u16, u16)) -> Rgb<u8> {
	a.map2(&b, |x, y| {
		((u16::from(x) * of_a + u16::from(y) * of_b) / (of_a + of_b)) as u8
	})
}

/// The four colours a DXT1 block picks from. Which way round its two stored
/// colours are is what says how the other two are made.
fn palette(first: u16, second: u16) -> [Rgb<u8>; 4] {
	let (a, b) = (expand(first), expand(second));
	if first > second {
		[a, b, blend(a, b, (2, 1)), blend(a, b, (1, 2))]
	} else {
		[a, b, blend(a, b, (1, 1)), Rgb([0, 0, 0])]
	}
}

/// DXT1 as the engine draws it, opaque: a block of 8 bytes for each 4x4 pixels,
/// two colours and then two bits a pixel to pick from the palette they make.
fn decode(top: &[u8]) -> RgbImage {
	let mut image = RgbImage::new(SIDE, SIDE);
	let per_row = SIDE / 4;
	for (at, block) in (0u32..).zip(top.chunks_exact(8)) {
		let colours = palette(
			u16::from_le_bytes([block[0], block[1]]),
			u16::from_le_bytes([block[2], block[3]]),
		);
		let picks = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
		for pixel in 0..16 {
			let colour = colours[(picks >> (2 * pixel) & 3) as usize];
			image.put_pixel(
				at % per_row * 4 + pixel % 4,
				at / per_row * 4 + pixel / 4,
				colour,
			);
		}
	}
	image
}

/// The size of a picture of a map this shape: its long side as long as the
/// minimap's, and never so thin that nothing is left to draw.
fn shape(width: u32, height: u32) -> (u32, u32) {
	let long = u64::from(width.max(height));
	let side = |length: u32| (u64::from(length) * u64::from(SIDE) / long).max(1) as u32;
	(side(width), side(height))
}

fn shaped(minimap: &Minimap) -> Result<Vec<u8>, Error> {
	let (width, height) = shape(minimap.width, minimap.height);
	let picture =
		image::imageops::resize(&decode(&minimap.top), width, height, FilterType::Lanczos3);
	let mut jpeg = Vec::new();
	JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
		.encode_image(&picture)
		.map_err(unreadable)?;
	Ok(jpeg)
}

#[cfg(test)]
pub(crate) mod tests {
	use std::io::{Cursor, Write};

	use image::Rgb;

	use super::*;

	const RED: u16 = 0xF800;
	const BLUE: u16 = 0x001F;

	/// A block of one colour: both ends set to it, every pixel on the first.
	fn solid(colour: u16) -> [u8; 8] {
		let [low, high] = colour.to_le_bytes();
		[low, high, low, high, 0, 0, 0, 0]
	}

	/// The top level of a minimap, every block the one given.
	fn filled(block: [u8; 8]) -> Vec<u8> {
		block.repeat(TOP_LEVEL / 8)
	}

	/// An `.smf` with its minimap `gap` bytes past the end of the header.
	fn smf(squares: (i32, i32), gap: usize, top: &[u8]) -> Vec<u8> {
		let mut file = b"spring map file\0".to_vec();
		let field = |value: i32| value.to_le_bytes();
		for value in [1, 42, squares.0, squares.1, 8, 8, 32] {
			file.extend(field(value));
		}
		file.extend([0u8; 8]);
		let at = HEADER as i32 + gap as i32;
		for value in [0, 0, 0, at, 0, 0, 0] {
			file.extend(field(value));
		}
		file.extend(vec![7u8; gap]);
		file.extend(top);
		file
	}

	/// A map archive of `colour`, as a zip, for what tests a whole archive.
	pub(crate) fn sdz(path: &Path, colour: u16) {
		let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
		let stored = zip::write::SimpleFileOptions::default();
		zip.start_file("mapinfo.lua", stored).unwrap();
		zip.write_all(b"return {}").unwrap();
		zip.start_file("maps/test.smf", stored).unwrap();
		zip.write_all(&smf((1024, 1024), 16, &filled(solid(colour))))
			.unwrap();
		zip.finish().unwrap();
	}

	fn decoded(block: [u8; 8]) -> image::RgbImage {
		decode(&filled(block))
	}

	#[test]
	fn a_block_with_its_ends_in_order_mixes_two_colours_between_them() {
		// The two-bit indices for the top row of pixels: 0, 1, 2, 3.
		let [c0_low, c0_high] = RED.to_le_bytes();
		let [c1_low, c1_high] = BLUE.to_le_bytes();
		let image = decoded([c0_low, c0_high, c1_low, c1_high, 0b11_10_01_00, 0, 0, 0]);
		assert_eq!(*image.get_pixel(0, 0), Rgb([255, 0, 0]));
		assert_eq!(*image.get_pixel(1, 0), Rgb([0, 0, 255]));
		assert_eq!(*image.get_pixel(2, 0), Rgb([170, 0, 85]));
		assert_eq!(*image.get_pixel(3, 0), Rgb([85, 0, 170]));
	}

	#[test]
	fn a_block_with_its_ends_reversed_has_a_midpoint_and_black() {
		let [c0_low, c0_high] = BLUE.to_le_bytes();
		let [c1_low, c1_high] = RED.to_le_bytes();
		let image = decoded([c0_low, c0_high, c1_low, c1_high, 0b11_10_01_00, 0, 0, 0]);
		assert_eq!(*image.get_pixel(0, 0), Rgb([0, 0, 255]));
		assert_eq!(*image.get_pixel(1, 0), Rgb([255, 0, 0]));
		assert_eq!(*image.get_pixel(2, 0), Rgb([127, 0, 127]));
		assert_eq!(*image.get_pixel(3, 0), Rgb([0, 0, 0]));
	}

	#[test]
	fn the_indices_run_along_each_row_from_the_top() {
		let [low, high] = RED.to_le_bytes();
		let [c1_low, c1_high] = BLUE.to_le_bytes();
		// Only the last pixel of the last row is on the other colour.
		let image = decoded([low, high, c1_low, c1_high, 0, 0, 0, 0b01_00_00_00]);
		assert_eq!(*image.get_pixel(3, 3), Rgb([0, 0, 255]));
		assert_eq!(*image.get_pixel(0, 3), Rgb([255, 0, 0]));
		assert_eq!(*image.get_pixel(3, 0), Rgb([255, 0, 0]));
	}

	#[test]
	fn blocks_fill_the_picture_left_to_right_then_down() {
		let mut top = filled(solid(RED));
		let per_row = SIDE as usize / 4;
		top[8..16].copy_from_slice(&solid(BLUE));
		top[8 * per_row..8 * (per_row + 1)].copy_from_slice(&solid(BLUE));
		let image = decode(&top);
		assert_eq!((image.width(), image.height()), (SIDE, SIDE));
		assert_eq!(*image.get_pixel(3, 0), Rgb([255, 0, 0]));
		assert_eq!(*image.get_pixel(4, 0), Rgb([0, 0, 255]));
		assert_eq!(*image.get_pixel(8, 0), Rgb([255, 0, 0]));
		assert_eq!(*image.get_pixel(0, 4), Rgb([0, 0, 255]));
	}

	#[test]
	fn the_minimap_is_found_where_the_header_says() {
		let file = smf((1024, 2048), 3000, &filled(solid(BLUE)));
		let minimap = from_smf(Cursor::new(file)).unwrap();
		assert_eq!((minimap.width, minimap.height), (1024, 2048));
		assert_eq!(*decode(&minimap.top).get_pixel(0, 0), Rgb([0, 0, 255]));
	}

	fn refused(file: Vec<u8>) -> String {
		match from_smf(Cursor::new(file)) {
			Err(Error::Unreadable(why)) => why,
			Err(other) => panic!("the wrong kind of failure: {other}"),
			Ok(_) => panic!("read a minimap out of it"),
		}
	}

	#[test]
	fn a_file_that_is_not_a_map_is_refused() {
		let mut file = smf((1024, 1024), 0, &filled(solid(RED)));
		file[..6].copy_from_slice(b"spoing");
		assert!(refused(file).contains("not a map"));
	}

	#[test]
	fn a_map_ending_before_its_minimap_is_refused() {
		let mut file = smf((1024, 1024), 16, &filled(solid(RED)));
		file.truncate(file.len() - 1);
		refused(file);
		refused(b"spring map".to_vec());
	}

	#[test]
	fn a_header_that_points_nowhere_sensible_is_refused() {
		let write_at = |file: &mut Vec<u8>, at: usize, value: i32| {
			file[at..at + 4].copy_from_slice(&value.to_le_bytes());
		};
		let good = || smf((1024, 1024), 16, &filled(solid(RED)));

		for (at, value) in [
			(MINIMAP_POINTER, -1),
			(MINIMAP_POINTER, 8),
			(WIDTH, 0),
			(WIDTH, -1024),
			(HEIGHT, 0),
		] {
			let mut file = good();
			write_at(&mut file, at, value);
			refused(file);
		}
	}

	#[test]
	fn a_minimap_further_in_than_any_real_one_is_not_gone_looking_for() {
		// Nothing past the header is even there: what stops this is the
		// pointer, before a byte of a decompression bomb is asked for.
		let mut file = smf((1024, 1024), 0, &[]);
		file[MINIMAP_POINTER..MINIMAP_POINTER + 4]
			.copy_from_slice(&((MOST_DECODED + 1) as i32).to_le_bytes());
		assert!(refused(file).contains("further in"));
	}

	#[test]
	fn the_picture_takes_the_shape_of_the_map() {
		assert_eq!(shape(1024, 1024), (SIDE, SIDE));
		assert_eq!(shape(1024, 1280), (819, SIDE));
		assert_eq!(shape(2048, 1024), (SIDE, 512));
		// However long and thin, there is a picture left to draw.
		assert_eq!(shape(1, 1 << 30), (1, SIDE));
	}

	#[test]
	fn a_square_stored_minimap_comes_out_as_wide_as_the_map_is() {
		let picture = shaped(&Minimap {
			width: 1024,
			height: 2048,
			top: filled(solid(RED)),
		})
		.unwrap();
		let image = image::load_from_memory(&picture).unwrap().into_rgb8();
		assert_eq!((image.width(), image.height()), (SIDE / 2, SIDE));
		let Rgb([red, green, blue]) = *image.get_pixel(100, 100);
		assert!(red > 240 && green < 16 && blue < 16, "{red} {green} {blue}");
	}

	fn is_blue(picture: &[u8]) -> bool {
		let image = image::load_from_memory(picture).unwrap().into_rgb8();
		let Rgb([red, green, blue]) = *image.get_pixel(10, 10);
		blue > 240 && red < 16 && green < 16
	}

	#[test]
	fn a_zipped_map_is_read() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("zipped_1.0.sdz");
		sdz(&path, BLUE);
		assert!(is_blue(&picture_in(&path).unwrap()));
	}

	#[test]
	fn an_unpacked_map_is_read() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("unpacked.sdd");
		std::fs::create_dir_all(path.join("maps")).unwrap();
		std::fs::write(
			path.join("maps").join("Unpacked.SMF"),
			smf((1024, 1024), 16, &filled(solid(BLUE))),
		)
		.unwrap();
		assert!(is_blue(&picture_in(&path).unwrap()));
	}

	/// One block per file, as nearly every map is packed.
	#[test]
	fn a_map_packed_a_file_at_a_time_is_read() {
		use sevenz_rust2::{ArchiveEntry, ArchiveWriter};

		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("loose_1.0.sd7");
		let mut seven = ArchiveWriter::create(&path).unwrap();
		let smf_bytes = smf((1024, 1024), 16, &filled(solid(BLUE)));
		for (name, bytes) in [
			("mapinfo.lua", b"return {}".to_vec()),
			("maps/loose.smf", smf_bytes),
			("maps/loose.smt", vec![9u8; 4096]),
		] {
			seven
				.push_archive_entry(ArchiveEntry::new_file(name), Some(Cursor::new(bytes)))
				.unwrap();
		}
		seven.finish().unwrap();
		assert!(is_blue(&picture_in(&path).unwrap()));
	}

	/// One block for everything, as a few are: what comes before the `.smf`
	/// has to be decoded to reach it, and reading from where the last entry
	/// stopped would hand back someone else's bytes.
	#[test]
	fn a_map_packed_solid_is_read_past_what_comes_before_it() {
		use sevenz_rust2::{ArchiveEntry, ArchiveWriter, SourceReader};

		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("solid_1.0.sd7");
		let mut seven = ArchiveWriter::create(&path).unwrap();
		let entries = vec![
			ArchiveEntry::new_file("aaa_before.dat"),
			ArchiveEntry::new_file("maps/solid.smf"),
		];
		let readers: Vec<SourceReader<Cursor<Vec<u8>>>> = vec![
			Cursor::new(vec![9u8; 4096]).into(),
			Cursor::new(smf((1024, 1024), 16, &filled(solid(BLUE)))).into(),
		];
		seven.push_archive_entries(entries, readers).unwrap();
		seven.finish().unwrap();
		assert!(is_blue(&picture_in(&path).unwrap()));
	}

	#[test]
	fn an_archive_with_no_map_in_it_is_unreadable_for_good() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("empty_1.0.sdz");
		let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
		zip.start_file("mapinfo.lua", zip::write::SimpleFileOptions::default())
			.unwrap();
		zip.finish().unwrap();
		assert!(matches!(picture_in(&path), Err(Error::Unreadable(_))));
	}

	/// What half a download looks like, whose stamp changes when it finishes.
	#[test]
	fn an_archive_that_is_not_one_is_unreadable_not_a_panic() {
		let dir = tempfile::tempdir().unwrap();
		for name in ["half_1.0.sd7", "half_1.0.sdz"] {
			let path = dir.path().join(name);
			std::fs::write(&path, b"PK half a file").unwrap();
			assert!(
				matches!(picture_in(&path), Err(Error::Unreadable(_))),
				"{name}"
			);
		}
	}

	/// The one failure worth trying again: nothing was learned about the file.
	#[test]
	fn a_file_that_cannot_be_opened_is_an_error_of_the_moment() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("gone_1.0.sd7");
		assert!(matches!(picture_in(&path), Err(Error::Io(_))));
	}
}
