//! Map pictures at the size a tile shows them, resized here and kept on disk.
//!
//! The index publishes one picture per map: the 1024px transform the official
//! lobby asks for, which the CDN has cached and marks immutable. The battle
//! list shows it in a tile a twentieth of that size, and a webview scaling a
//! picture down that far aliases — terrain turns to noise. Asking the image
//! server for a smaller transform would make modlobby the only client of that
//! transform, and the picture arrives without CORS headers, so the webview
//! cannot redraw it on a canvas either. So the published picture is fetched
//! once, with the lobby's own client, and kept as it came; every size drawn
//! is cut and resized from that copy with a proper filter and kept beside it,
//! under `cache/map-thumbs/`, for good. The files are named after the URL,
//! which changes when the picture does.
//!
//! A cold cache shows: the first look at a room waits for a download and a
//! decode. So the list can ask for the maps it shows to be made ahead of time
//! ([`Service::warm`]), on one worker, so that at most one core is busy with
//! it and what the screen asks for now is never queued behind it.
//!
//! A picture that is not there is remembered as not being there. The URL names
//! the picture — it changes when the picture does — so a 404 is an answer about
//! that URL and not about the moment, and a list that repaints must not send
//! BAR's image server the same one every frame. What is about the moment
//! instead — a dropped connection, a 500, a 429, a login page dressed as a
//! picture — is not remembered, because the moment passes.
//!
//! Nothing past the first look at a picture is about maps in particular. A
//! picture is kept as it came and cut from there, whether it came from a URL --
//! the map index's, or the banner on a news card -- or out of a map's own archive
//! ([`Picture::Minimap`]), which is the only one a map the index no longer lists
//! has.

use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use image::imageops::FilterType;
use image::{ImageFormat, ImageReader};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use ts_rs::TS;

/// Under the config directory's `cache/`.
pub const CACHE_DIR: &str = "map-thumbs";
/// The longest side served. The source is 1024 on its long side; anything
/// bigger would be an upscale, and a tile does not need one.
pub const MAX_SIDE: u32 = 1024;
/// How many pictures are fetched at once. See [`Inner::gate`].
pub const AT_ONCE: usize = 6;

/// The box a picture is made to fill, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Tile {
	pub width: u32,
	pub height: u32,
}

impl Tile {
	/// `None` when a side is zero or beyond [`MAX_SIDE`].
	pub fn new(width: u32, height: u32) -> Option<Self> {
		let fits = |side: u32| (1..=MAX_SIDE).contains(&side);
		(fits(width) && fits(height)).then_some(Self { width, height })
	}
}

/// Where a picture comes from: what its files are named after, and what is
/// done the one time it is not on the disk yet.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Picture {
	/// Published at this URL, which changes when the picture does.
	Published(String),
	/// The minimap inside the map at `archive`. An archive's name says nothing
	/// about what is in it the way a URL does, so `stamp` does: a map replaced
	/// under the same name, or half a download finishing, is another picture.
	Minimap { archive: PathBuf, stamp: String },
}

impl Picture {
	/// The minimap of the map at `archive` as it stands now, if it stands.
	pub fn minimap(archive: PathBuf) -> Option<Self> {
		let about = std::fs::metadata(&archive).ok()?;
		let modified = about.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
		let stamp = format!("{}-{}", about.len(), modified.as_nanos());
		Some(Self::Minimap { archive, stamp })
	}

	/// What this picture's files are named after and its refusals kept under.
	fn key(&self) -> String {
		match self {
			Self::Published(url) => url.clone(),
			Self::Minimap { archive, stamp } => {
				format!("minimap:{}:{stamp}", archive.display())
			}
		}
	}
}

/// What is wanted of a picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
	/// Cut to fill this box, as [`Service::get`] does.
	Fill(Tile),
	/// Fitted whole inside this box, as [`Service::get_whole`] does.
	Whole(Tile),
	/// As it came, as [`Service::original`] gives it.
	AsItCame,
}

/// One map to make ahead of time: its picture, at these sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
	pub picture: Picture,
	/// Cut to fill each box, as [`Service::get`] does.
	pub tiles: Vec<Tile>,
	/// Fitted whole inside each box, as [`Service::get_whole`] does.
	pub whole: Vec<Tile>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("{0}")]
	Request(#[from] reqwest::Error),
	#[error("the picture answered {0}")]
	Status(u16),
	#[error("the picture is not an image: {0}")]
	Image(#[from] image::ImageError),
	#[error("{0}")]
	Io(#[from] std::io::Error),
	/// A picture in a format this build does not decode: real, published, and
	/// no use here.
	#[error("the picture is a {0:?}, which this build cannot decode")]
	Format(image::ImageFormat),
	/// The answer this picture already got, given again without asking again.
	#[error("{0}")]
	Refused(String),
	#[error("{0}")]
	Minimap(#[from] crate::map_minimap::Error),
	/// Nothing says where this picture is.
	#[error("nowhere to get this picture from")]
	Nowhere,
}

impl Error {
	/// Whether this says something about the URL or something about the moment.
	///
	/// A 4xx is an answer about this URL, and the URL names the picture, so
	/// asking again can only get the same answer back -- except the two that
	/// exist to say "later": a 429 carries the server's own `Retry-After`, and
	/// a 408 is the connection's, not the picture's. A format this build does
	/// not decode is about the URL too; the picture will not change formats.
	/// An archive with no minimap to read is about the archive as it is now,
	/// and a [`Picture::Minimap`] names only that; one that could not be opened
	/// says nothing about it.
	/// Everything else is about now, and now passes: a dropped connection, a
	/// 5xx, a local IO failure, and a body that is not a picture at all, which
	/// is what a captive portal or a maintenance page looks like from here --
	/// every map asked for through a hotel's login page would otherwise stay
	/// blank until the lobby restarts.
	fn settled(&self) -> bool {
		match self {
			Self::Status(408 | 429) => false,
			Self::Status(code) => (400..500).contains(code),
			Self::Format(_) | Self::Minimap(crate::map_minimap::Error::Unreadable(_)) => true,
			Self::Minimap(crate::map_minimap::Error::Io(_))
			| Self::Request(_)
			| Self::Image(_)
			| Self::Io(_)
			| Self::Refused(_)
			| Self::Nowhere => false,
		}
	}
}

/// The pictures made this run, and where they are kept. Cheap to clone: a
/// handle on one shared state, like the HTTP client inside it.
#[derive(Clone)]
pub struct Service(Arc<Inner>);

struct Inner {
	client: reqwest::Client,
	dir: PathBuf,
	/// How many pictures may be in flight at once, fetched or read.
	///
	/// The per-file lock joins callers asking for the *same* picture; a list
	/// painting thirty rooms asks for thirty different ones in the same frame,
	/// and those it does not join. Six is what a browser allows itself per
	/// host, and the rest wait a moment rather than arriving all at once at
	/// somebody else's image server.
	gate: tokio::sync::Semaphore,
	/// Pictures that answered something about themselves, and what they
	/// answered, by [`Picture::key`].
	///
	/// Kept for the run rather than on disk: a picture missing because it was
	/// published late should come back on the next start, and a lobby that has
	/// been open for hours should not still be asking for it.
	refused: Mutex<HashMap<String, String>>,
	/// One lock per file — the picture as published and each size made from
	/// it — so a list showing the same map in twenty rooms fetches and
	/// resizes it once and the other nineteen wait for the file. Kept for the
	/// run: there are as many as files touched, which is few.
	making: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
	/// What [`Service::warm`] was last asked for and has not made yet, in the
	/// order asked.
	queue: Mutex<VecDeque<Job>>,
	/// Whether a worker is draining the queue. Changed only under `queue`'s
	/// lock, so a job cannot slip in between the worker's last look at the
	/// queue and its exit.
	busy: watch::Sender<bool>,
}

impl Service {
	pub fn new(client: reqwest::Client, cache_dir: &Path) -> Self {
		Self(Arc::new(Inner {
			client,
			dir: cache_dir.join(CACHE_DIR),
			gate: tokio::sync::Semaphore::new(AT_ONCE),
			refused: Mutex::new(HashMap::new()),
			making: Mutex::new(HashMap::new()),
			queue: Mutex::new(VecDeque::new()),
			busy: watch::Sender::new(false),
		}))
	}

	/// The first of `pictures` that can be had, cut as asked, with what to call
	/// it. One that cannot be gives way to the next, and the next is not looked
	/// for until then. With none to try, [`Error::Nowhere`]; with none that
	/// worked, what the last one said.
	pub async fn first_of(
		&self,
		pictures: impl IntoIterator<Item = Picture>,
		cut: Cut,
	) -> Result<(&'static str, Vec<u8>), Error> {
		let mut last = Error::Nowhere;
		for picture in pictures {
			let made = match cut {
				Cut::Fill(tile) => self.get(&picture, tile).await.map(|png| ("image/png", png)),
				Cut::Whole(tile) => self
					.get_whole(&picture, tile)
					.await
					.map(|png| ("image/png", png)),
				Cut::AsItCame => self
					.original(&picture)
					.await
					.map(|kept| (mime(&kept), kept)),
			};
			match made {
				Ok(found) => return Ok(found),
				Err(err) => {
					tracing::debug!(?picture, %err, "no picture here");
					last = err;
				}
			}
		}
		Err(last)
	}

	/// `picture` filling `tile`, as PNG bytes: from disk when it has been made
	/// before, otherwise made from the picture as it came — itself fetched or
	/// read only the first time — and kept.
	pub async fn get(&self, picture: &Picture, tile: Tile) -> Result<Vec<u8>, Error> {
		self.make(picture, tile, "", fill).await
	}

	/// `picture` whole inside `tile`: scaled to fit, never cropped,
	/// so the box it is drawn in pads it instead of cutting it. What a
	/// screenshot of an interface wants — its edges are the part that says
	/// which interface it is.
	pub async fn get_whole(&self, picture: &Picture, tile: Tile) -> Result<Vec<u8>, Error> {
		self.make(picture, tile, "-whole", fit).await
	}

	async fn make(
		&self,
		from: &Picture,
		tile: Tile,
		kind: &str,
		resize: fn(&[u8], Tile) -> Result<Vec<u8>, image::ImageError>,
	) -> Result<Vec<u8>, Error> {
		let path = self.0.dir.join(format!(
			"{}-{}x{}{kind}.png",
			hash(&from.key()),
			tile.width,
			tile.height
		));
		if let Ok(png) = tokio::fs::read(&path).await {
			return Ok(png);
		}
		let lock = self.lock_for(&path);
		let _making = lock.lock().await;
		// Made by whoever held the lock before us.
		if let Ok(png) = tokio::fs::read(&path).await {
			return Ok(png);
		}
		let picture = self.original(from).await?;
		// Decoding and resizing a 1024px picture is tens of milliseconds of
		// CPU, which the async runtime should not sit through.
		let png = tokio::task::spawn_blocking(move || resize(&picture, tile))
			.await
			.expect("resizing does not panic")?;
		write(&path, &png).await?;
		Ok(png)
	}

	/// Makes `jobs` ahead of time, in order, on one worker: the pictures at
	/// the top of the list are the ones about to be looked at. What was
	/// queued by an earlier call and not yet made is dropped — the newest
	/// list is what is on screen. Anything that fails is skipped; the screen
	/// will ask for it itself and see the error then.
	///
	/// Must be called on the async runtime: the worker is spawned on it.
	pub fn warm(&self, jobs: Vec<Job>) {
		let mut queue = self.0.queue.lock().expect("the queue is never poisoned");
		*queue = jobs.into();
		if self.0.busy.send_replace(true) {
			return;
		}
		let worker = self.clone();
		tokio::spawn(worker.drain());
	}

	/// Resolves once no worker is running: what was warmed is on disk.
	pub async fn settled(&self) {
		let mut busy = self.0.busy.subscribe();
		// A closed channel means the service is gone, which is settled too.
		let _ = busy.wait_for(|busy| !busy).await;
	}

	async fn drain(self) {
		loop {
			let next = {
				let mut queue = self.0.queue.lock().expect("the queue is never poisoned");
				match queue.pop_front() {
					Some(job) => job,
					None => {
						self.0.busy.send_replace(false);
						return;
					}
				}
			};
			for tile in next.tiles {
				if let Err(err) = self.get(&next.picture, tile).await {
					tracing::debug!(picture = ?next.picture, ?tile, %err, "not warmed");
				}
			}
			for tile in next.whole {
				if let Err(err) = self.get_whole(&next.picture, tile).await {
					tracing::debug!(picture = ?next.picture, ?tile, %err, "not warmed whole");
				}
			}
		}
	}

	/// The picture as it came, from disk after the first time — and from the
	/// refusal it already got, if it got one.
	pub async fn original(&self, picture: &Picture) -> Result<Vec<u8>, Error> {
		let key = picture.key();
		let path = self.0.dir.join(format!("{}.src", hash(&key)));
		if let Ok(kept) = tokio::fs::read(&path).await {
			return Ok(kept);
		}
		if let Some(refused) = self.refused(&key) {
			return Err(Error::Refused(refused));
		}
		let lock = self.lock_for(&path);
		let _fetching = lock.lock().await;
		// Either could have happened while somebody else held the lock asking
		// this same question, and both are answers.
		if let Ok(kept) = tokio::fs::read(&path).await {
			return Ok(kept);
		}
		if let Some(refused) = self.refused(&key) {
			return Err(Error::Refused(refused));
		}
		let kept = self
			.first_look(picture)
			.await
			.inspect_err(|err| self.refuse(&key, err))?;
		write(&path, &kept).await?;
		Ok(kept)
	}

	/// The one time a picture is not on the disk yet.
	async fn first_look(&self, picture: &Picture) -> Result<Vec<u8>, Error> {
		match picture {
			Picture::Published(url) => self.fetch(url).await,
			Picture::Minimap { archive, .. } => self.read_minimap(archive.clone()).await,
		}
	}

	/// Decompressing holds a dictionary of tens of megabytes for as long as it
	/// runs, and a list of old maps asks for them all in the same frame, so
	/// these wait their turn at the gate as a download does.
	async fn read_minimap(&self, archive: PathBuf) -> Result<Vec<u8>, Error> {
		let _turn = self
			.0
			.gate
			.acquire()
			.await
			.expect("the gate is never closed");
		off_the_runtime(move || crate::map_minimap::picture_in(&archive)).await
	}

	/// The answer this picture already got, when it got one worth keeping.
	fn refused(&self, key: &str) -> Option<String> {
		self.0
			.refused
			.lock()
			.expect("the refusal map is never poisoned")
			.get(key)
			.cloned()
	}

	/// Keeps an answer that was about the picture, so it is not asked for twice.
	fn refuse(&self, key: &str, err: &Error) {
		if !err.settled() {
			return;
		}
		tracing::debug!(key, %err, "picture refused; not asking again this run");
		self.0
			.refused
			.lock()
			.expect("the refusal map is never poisoned")
			.insert(key.to_owned(), err.to_string());
	}

	fn lock_for(&self, path: &Path) -> Arc<tokio::sync::Mutex<()>> {
		self.0
			.making
			.lock()
			.expect("the lock map is never poisoned")
			.entry(path.to_owned())
			.or_default()
			.clone()
	}

	/// A body that is a picture, by its magic bytes: a maintenance page or an
	/// error dressed as 200 must not be kept as if it were one.
	async fn fetch(&self, url: &str) -> Result<Vec<u8>, Error> {
		let _turn = self
			.0
			.gate
			.acquire()
			.await
			.expect("the gate is never closed");
		let response = self.0.client.get(url).send().await?;
		if !response.status().is_success() {
			return Err(Error::Status(response.status().as_u16()));
		}
		let picture = response.bytes().await?;
		// Two questions, not one: the magic bytes know some twenty formats,
		// the build decodes three. A real picture in one of the others must
		// not be kept as if it could be drawn, or it is decoded and fails on
		// every paint for good -- the files are kept forever, and this is the
		// one place that decides what gets in.
		let format = image::guess_format(&picture)?;
		if !format.reading_enabled() {
			return Err(Error::Format(format));
		}
		Ok(picture.to_vec())
	}
}

/// Reading an archive, away from the async runtime, which should not sit
/// through decompressing. The parsers down there are not ours and the archive
/// is anybody's, so a panic is an answer about that archive like any other
/// reason it cannot be read.
async fn off_the_runtime(
	work: impl FnOnce() -> Result<Vec<u8>, crate::map_minimap::Error> + Send + 'static,
) -> Result<Vec<u8>, Error> {
	let read = tokio::task::spawn_blocking(work)
		.await
		.map_err(|panicked| crate::map_minimap::Error::Unreadable(panicked.to_string()))?;
	Ok(read?)
}

/// What to call a picture kept as it came: its format, by the magic bytes.
pub fn mime(picture: &[u8]) -> &'static str {
	image::guess_format(picture)
		.map(|format| format.to_mime_type())
		.unwrap_or("application/octet-stream")
}

/// What a picture's files are named after: its key, which changes when the
/// picture does, and which has characters a file name cannot.
fn hash(key: &str) -> String {
	Md5::digest(key.as_bytes())
		.iter()
		.map(|byte| format!("{byte:02x}"))
		.collect()
}

/// `picture`, in any format the decoder knows, scaled to cover `tile` and
/// cropped to it about the centre — what CSS `object-fit: cover` would have
/// done — as a PNG of exactly that size. Pure.
pub fn fill(picture: &[u8], tile: Tile) -> Result<Vec<u8>, image::ImageError> {
	let image = ImageReader::new(Cursor::new(picture))
		.with_guessed_format()?
		.decode()?;
	let small = image.resize_to_fill(tile.width, tile.height, FilterType::Lanczos3);
	let mut png = Cursor::new(Vec::new());
	// A map photo has no alpha worth keeping, and RGB halves the file.
	small.into_rgb8().write_to(&mut png, ImageFormat::Png)?;
	Ok(png.into_inner())
}

/// `picture`, scaled to fit *inside* `tile` with its shape kept and nothing
/// cropped — what CSS `object-fit: contain` would have done — as a PNG no
/// bigger than the tile on either side. The padding is the box's, not the
/// picture's, so a tall screenshot and a wide one both arrive whole. Pure.
pub fn fit(picture: &[u8], tile: Tile) -> Result<Vec<u8>, image::ImageError> {
	let image = ImageReader::new(Cursor::new(picture))
		.with_guessed_format()?
		.decode()?;
	let small = image.resize(tile.width, tile.height, FilterType::Lanczos3);
	let mut png = Cursor::new(Vec::new());
	small.into_rgb8().write_to(&mut png, ImageFormat::Png)?;
	Ok(png.into_inner())
}

/// Temp file and rename, so a crash never leaves half a picture behind.
async fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
	if let Some(parent) = path.parent() {
		tokio::fs::create_dir_all(parent).await?;
	}
	let tmp = path.with_extension("tmp");
	tokio::fs::write(&tmp, bytes).await?;
	tokio::fs::rename(&tmp, path).await
}

#[cfg(test)]
mod tests {
	use super::*;
	use image::{Rgb, RgbImage};
	use wiremock::matchers::{method, path};
	use wiremock::{Mock, MockServer, ResponseTemplate};

	const RED: Rgb<u8> = Rgb([255, 0, 0]);
	const BLUE: Rgb<u8> = Rgb([0, 0, 255]);

	/// A 200×100 picture, red on the left and blue on the right, so a crop
	/// shows where it was taken from.
	fn halves(format: ImageFormat) -> Vec<u8> {
		let picture = RgbImage::from_fn(200, 100, |x, _| if x < 100 { RED } else { BLUE });
		let mut bytes = Cursor::new(Vec::new());
		picture.write_to(&mut bytes, format).unwrap();
		bytes.into_inner()
	}

	fn tile(width: u32, height: u32) -> Tile {
		Tile::new(width, height).unwrap()
	}

	fn at(url: &str) -> Picture {
		Picture::Published(url.to_owned())
	}

	async fn serving(picture: Vec<u8>, times: u64) -> (MockServer, String) {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/map.png"))
			.respond_with(ResponseTemplate::new(200).set_body_bytes(picture))
			.expect(times)
			.mount(&server)
			.await;
		let url = format!("{}/map.png", server.uri());
		(server, url)
	}

	fn service(dir: &Path) -> Service {
		Service::new(crate::http::client("test"), dir)
	}

	fn kept(dir: &Path) -> Vec<String> {
		let mut names: Vec<String> = std::fs::read_dir(dir.join(CACHE_DIR))
			.unwrap()
			.map(|entry| entry.unwrap().file_name().into_string().unwrap())
			.collect();
		names.sort();
		names
	}

	#[test]
	fn a_tile_has_sides_between_one_and_the_source() {
		assert!(Tile::new(50, 32).is_some());
		assert!(Tile::new(MAX_SIDE, 1).is_some());
		assert!(Tile::new(0, 32).is_none());
		assert!(Tile::new(50, MAX_SIDE + 1).is_none());
	}

	#[test]
	fn a_fill_covers_the_tile_and_crops_about_the_centre() {
		// 200×100 into 50×50: scaled to 100×50, then the middle 50 kept, so
		// the seam between the halves lands in the middle of the tile.
		let png = fill(&halves(ImageFormat::Png), tile(50, 50)).unwrap();
		let small = image::load_from_memory(&png).unwrap().into_rgb8();
		assert_eq!((small.width(), small.height()), (50, 50));
		assert_eq!(*small.get_pixel(5, 25), RED);
		assert_eq!(*small.get_pixel(44, 25), BLUE);
	}

	#[test]
	fn a_fit_keeps_the_whole_picture_and_its_shape() {
		// 200×100 into 50×50: 50×25, both halves still there and neither side
		// past the tile — the box pads what is left over.
		let png = fit(&halves(ImageFormat::Png), tile(50, 50)).unwrap();
		let small = image::load_from_memory(&png).unwrap().into_rgb8();
		assert_eq!((small.width(), small.height()), (50, 25));
		assert_eq!(*small.get_pixel(5, 12), RED);
		assert_eq!(*small.get_pixel(44, 12), BLUE);
	}

	#[test]
	fn a_webp_is_decoded() {
		let png = fill(&halves(ImageFormat::WebP), tile(20, 10)).unwrap();
		let small = image::load_from_memory(&png).unwrap();
		assert_eq!((small.width(), small.height()), (20, 10));
	}

	#[test]
	fn bytes_that_are_not_a_picture_are_refused() {
		assert!(fill(b"<html>not found</html>", tile(20, 10)).is_err());
	}

	#[tokio::test]
	async fn a_picture_is_fetched_once_and_then_read_from_disk() {
		let (_server, url) = serving(halves(ImageFormat::Png), 1).await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		let first = thumbs.get(&at(&url), tile(50, 32)).await.unwrap();
		let again = thumbs.get(&at(&url), tile(50, 32)).await.unwrap();
		assert_eq!(first, again);
		let small = image::load_from_memory(&first).unwrap();
		assert_eq!((small.width(), small.height()), (50, 32));

		// The picture as published, and the one size made from it.
		let files = kept(dir.path());
		assert_eq!(files.len(), 2, "{files:?}");
		assert!(files[0].ends_with("-50x32.png"), "{files:?}");
		assert!(files[1].ends_with(".src"), "{files:?}");
	}

	#[tokio::test]
	async fn the_same_picture_asked_for_together_is_fetched_once() {
		let (_server, url) = serving(halves(ImageFormat::Png), 1).await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		let picture = at(&url);
		let (one, two) = tokio::join!(
			thumbs.get(&picture, tile(50, 32)),
			thumbs.get(&picture, tile(50, 32))
		);
		assert_eq!(one.unwrap(), two.unwrap());
	}

	#[tokio::test]
	async fn a_second_size_is_made_from_the_kept_picture() {
		let (_server, url) = serving(halves(ImageFormat::Png), 1).await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		thumbs.get(&at(&url), tile(50, 32)).await.unwrap();
		let bigger = thumbs.get(&at(&url), tile(100, 64)).await.unwrap();
		let small = image::load_from_memory(&bigger).unwrap();
		assert_eq!((small.width(), small.height()), (100, 64));
		assert_eq!(kept(dir.path()).len(), 3);
	}

	#[tokio::test]
	async fn two_sizes_asked_for_together_fetch_the_picture_once() {
		let (_server, url) = serving(halves(ImageFormat::Png), 1).await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		let picture = at(&url);
		let (one, two) = tokio::join!(
			thumbs.get(&picture, tile(50, 32)),
			thumbs.get(&picture, tile(100, 64))
		);
		one.unwrap();
		two.unwrap();
	}

	#[tokio::test]
	async fn a_missing_picture_is_an_error_and_leaves_nothing_behind() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.respond_with(ResponseTemplate::new(404))
			.mount(&server)
			.await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		let err = thumbs
			.get(&at(&format!("{}/gone.png", server.uri())), tile(50, 32))
			.await
			.unwrap_err();
		assert!(matches!(err, Error::Status(404)), "{err}");
		assert!(!dir.path().join(CACHE_DIR).exists());
	}

	#[tokio::test]
	async fn a_body_that_is_not_a_picture_is_an_error_and_is_not_kept() {
		let (_server, url) = serving(b"<html>maintenance</html>".to_vec(), 1).await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		let err = thumbs.get(&at(&url), tile(50, 32)).await.unwrap_err();
		assert!(matches!(err, Error::Image(_)), "{err}");
		assert!(!dir.path().join(CACHE_DIR).exists());
	}

	/// A room whose map has no published picture sits in the list like any
	/// other, and the list repaints — on a new room, a filter, a scroll. Every
	/// one of those used to be another 404 for BAR's image server.
	#[tokio::test]
	async fn a_picture_that_is_not_there_is_asked_for_once() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/gone.png"))
			.respond_with(ResponseTemplate::new(404))
			.expect(1)
			.mount(&server)
			.await;
		let url = format!("{}/gone.png", server.uri());
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		// The first answer is the server's, and every one after it is the
		// same answer without the asking.
		let first = thumbs.get(&at(&url), tile(50, 32)).await.unwrap_err();
		assert!(matches!(first, Error::Status(404)), "{first}");
		for _ in 0..4 {
			let again = thumbs.get(&at(&url), tile(50, 32)).await.unwrap_err();
			assert!(matches!(again, Error::Refused(_)), "{again}");
			assert_eq!(again.to_string(), first.to_string());
		}
		// A size never asked for before goes through the same refusal rather
		// than fetching the picture it would be cut from.
		let other = thumbs.get(&at(&url), tile(130, 130)).await.unwrap_err();
		assert!(matches!(other, Error::Refused(_)), "{other}");

		// Warming is the other way the same URL comes round again.
		thumbs.warm(vec![Job {
			picture: at(&url),
			tiles: vec![tile(50, 32)],
			whole: vec![],
		}]);
		thumbs.settled().await;

		// The mock's own expectation is the assertion: one request, not seven.
		server.verify().await;
	}

	/// The magic-byte check knows some twenty formats and the build decodes
	/// three. A picture in one of the others used to be kept as published and
	/// then fail to decode on every paint, forever, since kept files are for
	/// good. Now it is an answer about the URL: asked once, and nothing kept.
	#[tokio::test]
	async fn a_format_this_build_cannot_draw_is_asked_for_once_and_not_kept() {
		let server = MockServer::start().await;
		Mock::given(method("GET"))
			.and(path("/anim.gif"))
			.respond_with(
				ResponseTemplate::new(200).set_body_bytes(b"GIF89a\x01\x00\x01\x00\x00".to_vec()),
			)
			.expect(1)
			.mount(&server)
			.await;
		let url = format!("{}/anim.gif", server.uri());
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		let first = thumbs.get(&at(&url), tile(50, 32)).await.unwrap_err();
		assert!(
			matches!(first, Error::Format(image::ImageFormat::Gif)),
			"{first}"
		);
		let again = thumbs.get(&at(&url), tile(50, 32)).await.unwrap_err();
		assert!(matches!(again, Error::Refused(_)), "{again}");
		assert!(!dir.path().join(CACHE_DIR).exists());
		server.verify().await;
	}

	/// The other half of the rule: a server having a bad minute is asked again,
	/// or one 500 would cost the picture until the lobby is restarted. So is
	/// one that said "later" in so many words -- a 429 is the one 4xx that is
	/// about the moment by definition -- and so is a login page dressed as a
	/// picture, which is what every URL looks like from behind a hotel's Wi-Fi.
	#[tokio::test]
	async fn a_moment_of_bad_luck_is_not_held_against_the_url() {
		let server = MockServer::start().await;
		for (name, answer) in [
			("/later.png", ResponseTemplate::new(503)),
			("/slower.png", ResponseTemplate::new(429)),
			(
				"/portal.png",
				ResponseTemplate::new(200).set_body_bytes(b"<html>sign in</html>".to_vec()),
			),
		] {
			Mock::given(method("GET"))
				.and(path(name))
				.respond_with(answer)
				.expect(3)
				.mount(&server)
				.await;
		}
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		for name in ["/later.png", "/slower.png", "/portal.png"] {
			let url = format!("{}{name}", server.uri());
			for _ in 0..3 {
				let err = thumbs.get(&at(&url), tile(50, 32)).await.unwrap_err();
				assert!(!matches!(err, Error::Refused(_)), "{name}: {err}");
			}
		}
		server.verify().await;
	}

	#[tokio::test]
	async fn warming_makes_every_size_ahead_so_a_look_costs_no_fetch() {
		let (_server, url) = serving(halves(ImageFormat::Png), 1).await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		thumbs.warm(vec![Job {
			picture: at(&url),
			tiles: vec![tile(50, 32), tile(130, 130)],
			whole: vec![tile(50, 41)],
		}]);
		thumbs.settled().await;
		assert_eq!(kept(dir.path()).len(), 4);

		let room = thumbs.get(&at(&url), tile(130, 130)).await.unwrap();
		let small = image::load_from_memory(&room).unwrap();
		assert_eq!((small.width(), small.height()), (130, 130));
		// The whole one is made ahead too, at the picture's own shape.
		let list = thumbs.get_whole(&at(&url), tile(50, 41)).await.unwrap();
		let small = image::load_from_memory(&list).unwrap();
		assert_eq!((small.width(), small.height()), (50, 25));
	}

	#[tokio::test]
	async fn a_map_that_fails_does_not_stop_the_ones_after_it() {
		let (server, url) = serving(halves(ImageFormat::Png), 1).await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		thumbs.warm(vec![
			Job {
				picture: at(&format!("{}/gone.png", server.uri())),
				tiles: vec![tile(50, 32)],
				whole: vec![],
			},
			Job {
				picture: at(&url),
				tiles: vec![tile(50, 32)],
				whole: vec![],
			},
		]);
		thumbs.settled().await;
		assert_eq!(kept(dir.path()).len(), 2);
	}

	const BLUE_565: u16 = 0x001F;
	const RED_565: u16 = 0xF800;

	fn map_archive(dir: &Path, colour: u16) -> (PathBuf, Picture) {
		let archive = dir.join("old_map_1.0.sdz");
		crate::map_minimap::tests::sdz(&archive, colour);
		let picture = Picture::minimap(archive.clone()).unwrap();
		(archive, picture)
	}

	fn is_blue(png: &[u8]) -> bool {
		let image = image::load_from_memory(png).unwrap().into_rgb8();
		let Rgb([red, green, blue]) = *image.get_pixel(1, 1);
		blue > 240 && red < 16 && green < 16
	}

	#[test]
	fn a_map_that_is_not_there_has_no_minimap_to_read() {
		let dir = tempfile::tempdir().unwrap();
		assert_eq!(Picture::minimap(dir.path().join("gone_1.0.sd7")), None);
	}

	#[tokio::test]
	async fn a_minimap_is_read_once_and_cut_from_what_was_kept() {
		let dir = tempfile::tempdir().unwrap();
		let (archive, picture) = map_archive(dir.path(), BLUE_565);
		let thumbs = service(dir.path());

		let first = thumbs.get(&picture, tile(50, 32)).await.unwrap();
		assert!(is_blue(&first));
		// The archive is no longer asked: what came out of it was kept.
		std::fs::remove_file(&archive).unwrap();
		assert_eq!(thumbs.get(&picture, tile(50, 32)).await.unwrap(), first);
		let bigger = thumbs.get(&picture, tile(100, 64)).await.unwrap();
		assert!(is_blue(&bigger));
		assert_eq!(kept(dir.path()).len(), 3);
	}

	#[tokio::test]
	async fn a_map_replaced_under_its_own_name_is_a_new_picture() {
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());
		let (_, red) = map_archive(dir.path(), RED_565);
		assert!(!is_blue(&thumbs.get(&red, tile(50, 32)).await.unwrap()));

		let (_, blue) = map_archive(dir.path(), BLUE_565);
		assert_ne!(red, blue);
		assert!(is_blue(&thumbs.get(&blue, tile(50, 32)).await.unwrap()));
	}

	/// Half a download is an archive that cannot be read, right up until the
	/// moment it can. A repaint must not decode it again meanwhile, and the
	/// finished file must not be held to what the half of it said.
	#[tokio::test]
	async fn a_broken_archive_is_read_once_for_as_long_as_it_stays_broken() {
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());
		let archive = dir.path().join("old_map_1.0.sdz");
		std::fs::write(&archive, b"PK half a download").unwrap();
		let half = Picture::minimap(archive.clone()).unwrap();

		let first = thumbs.original(&half).await.unwrap_err();
		assert!(matches!(first, Error::Minimap(_)), "{first}");
		let again = thumbs.original(&half).await.unwrap_err();
		assert!(matches!(again, Error::Refused(_)), "{again}");

		let (_, finished) = map_archive(dir.path(), BLUE_565);
		thumbs.get(&finished, tile(50, 32)).await.unwrap();
	}

	/// An antivirus holding the file says nothing about the map.
	#[tokio::test]
	async fn an_archive_that_could_not_be_opened_is_tried_again() {
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());
		let (archive, picture) = map_archive(dir.path(), BLUE_565);
		std::fs::remove_file(&archive).unwrap();

		for _ in 0..2 {
			let err = thumbs.original(&picture).await.unwrap_err();
			assert!(
				matches!(err, Error::Minimap(crate::map_minimap::Error::Io(_))),
				"{err}"
			);
		}
	}

	/// A panic would leave an `<img>` waiting for ever and the warming worker
	/// marked busy for the rest of the run.
	#[tokio::test]
	async fn a_parser_that_panics_is_an_answer_about_that_archive() {
		let err = off_the_runtime(|| panic!("a parser met what it never expected"))
			.await
			.unwrap_err();
		assert!(err.settled(), "{err}");
		assert!(err.to_string().contains("never expected"), "{err}");
	}

	#[tokio::test]
	async fn warming_reads_a_minimap_too() {
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());
		let (_, picture) = map_archive(dir.path(), BLUE_565);

		thumbs.warm(vec![Job {
			picture,
			tiles: vec![tile(50, 32)],
			whole: vec![tile(50, 41)],
		}]);
		thumbs.settled().await;
		assert_eq!(kept(dir.path()).len(), 3);
	}

	#[tokio::test]
	async fn a_picture_that_cannot_be_had_gives_way_to_the_next() {
		let (server, url) = serving(halves(ImageFormat::Png), 1).await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());
		let gone = at(&format!("{}/gone.png", server.uri()));

		let (mime, png) = thumbs
			.first_of([gone, at(&url)], Cut::Fill(tile(50, 32)))
			.await
			.unwrap();
		assert_eq!(mime, "image/png");
		let small = image::load_from_memory(&png).unwrap();
		assert_eq!((small.width(), small.height()), (50, 32));
	}

	/// The next is costly to look for, which is why it is an iterator.
	#[tokio::test]
	async fn the_next_is_not_looked_for_while_the_first_will_do() {
		let (_server, url) = serving(halves(ImageFormat::Png), 1).await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());
		let pictures = std::iter::once(at(&url)).chain(std::iter::once_with(|| {
			panic!("looked further than it had to")
		}));
		thumbs
			.first_of(pictures, Cut::Fill(tile(50, 32)))
			.await
			.unwrap();
	}

	#[tokio::test]
	async fn nothing_to_try_is_its_own_answer() {
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());
		let err = thumbs.first_of([], Cut::AsItCame).await.unwrap_err();
		assert!(matches!(err, Error::Nowhere), "{err}");
	}

	#[tokio::test]
	async fn with_nothing_left_to_try_the_last_failure_is_what_is_said() {
		let server = MockServer::start().await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());
		let gone = at(&format!("{}/gone.png", server.uri()));
		let err = thumbs
			.first_of([gone], Cut::Fill(tile(50, 32)))
			.await
			.unwrap_err();
		assert!(matches!(err, Error::Status(404)), "{err}");
	}

	#[tokio::test]
	async fn each_cut_is_made_as_its_own_method_makes_it() {
		let (_server, url) = serving(halves(ImageFormat::WebP), 1).await;
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());

		let (mime, whole) = thumbs
			.first_of([at(&url)], Cut::Whole(tile(50, 50)))
			.await
			.unwrap();
		assert_eq!(mime, "image/png");
		let small = image::load_from_memory(&whole).unwrap();
		assert_eq!((small.width(), small.height()), (50, 25));

		let (mime, came) = thumbs.first_of([at(&url)], Cut::AsItCame).await.unwrap();
		assert_eq!(mime, "image/webp");
		assert_eq!(came, thumbs.original(&at(&url)).await.unwrap());
	}

	#[tokio::test]
	async fn nothing_to_warm_is_settled_already() {
		let dir = tempfile::tempdir().unwrap();
		let thumbs = service(dir.path());
		thumbs.warm(Vec::new());
		thumbs.settled().await;
		assert!(!dir.path().join(CACHE_DIR).exists());
	}
}
