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

use std::collections::{HashMap, VecDeque};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

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

/// One map to make ahead of time: its published picture, at these sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub url: String,
    pub tiles: Vec<Tile>,
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
}

/// The pictures made this run, and where they are kept. Cheap to clone: a
/// handle on one shared state, like the HTTP client inside it.
#[derive(Clone)]
pub struct Service(Arc<Inner>);

struct Inner {
    client: reqwest::Client,
    dir: PathBuf,
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
            making: Mutex::new(HashMap::new()),
            queue: Mutex::new(VecDeque::new()),
            busy: watch::Sender::new(false),
        }))
    }

    /// The picture at `url` filling `tile`, as PNG bytes: from disk when it
    /// has been made before, otherwise made from the published picture —
    /// itself fetched only the first time — and kept.
    pub async fn get(&self, url: &str, tile: Tile) -> Result<Vec<u8>, Error> {
        let path = self
            .0
            .dir
            .join(format!("{}-{}x{}.png", hash(url), tile.width, tile.height));
        if let Ok(png) = tokio::fs::read(&path).await {
            return Ok(png);
        }
        let lock = self.lock_for(&path);
        let _making = lock.lock().await;
        // Made by whoever held the lock before us.
        if let Ok(png) = tokio::fs::read(&path).await {
            return Ok(png);
        }
        let picture = self.published(url).await?;
        // Decoding and resizing a 1024px picture is tens of milliseconds of
        // CPU, which the async runtime should not sit through.
        let png = tokio::task::spawn_blocking(move || fill(&picture, tile))
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
                if let Err(err) = self.get(&next.url, tile).await {
                    tracing::debug!(url = next.url, ?tile, %err, "not warmed");
                }
            }
        }
    }

    /// The picture as published, from disk after the first time.
    async fn published(&self, url: &str) -> Result<Vec<u8>, Error> {
        let path = self.0.dir.join(format!("{}.src", hash(url)));
        if let Ok(picture) = tokio::fs::read(&path).await {
            return Ok(picture);
        }
        let lock = self.lock_for(&path);
        let _fetching = lock.lock().await;
        if let Ok(picture) = tokio::fs::read(&path).await {
            return Ok(picture);
        }
        let picture = self.fetch(url).await?;
        write(&path, &picture).await?;
        Ok(picture)
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
        let response = self.0.client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(Error::Status(response.status().as_u16()));
        }
        let picture = response.bytes().await?;
        image::guess_format(&picture)?;
        Ok(picture.to_vec())
    }
}

/// What a picture's files are named after: the URL, which changes when the
/// picture does, and which has characters a file name cannot.
fn hash(url: &str) -> String {
    Md5::digest(url.as_bytes())
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

        let first = thumbs.get(&url, tile(50, 32)).await.unwrap();
        let again = thumbs.get(&url, tile(50, 32)).await.unwrap();
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

        let (one, two) = tokio::join!(
            thumbs.get(&url, tile(50, 32)),
            thumbs.get(&url, tile(50, 32))
        );
        assert_eq!(one.unwrap(), two.unwrap());
    }

    #[tokio::test]
    async fn a_second_size_is_made_from_the_kept_picture() {
        let (_server, url) = serving(halves(ImageFormat::Png), 1).await;
        let dir = tempfile::tempdir().unwrap();
        let thumbs = service(dir.path());

        thumbs.get(&url, tile(50, 32)).await.unwrap();
        let bigger = thumbs.get(&url, tile(100, 64)).await.unwrap();
        let small = image::load_from_memory(&bigger).unwrap();
        assert_eq!((small.width(), small.height()), (100, 64));
        assert_eq!(kept(dir.path()).len(), 3);
    }

    #[tokio::test]
    async fn two_sizes_asked_for_together_fetch_the_picture_once() {
        let (_server, url) = serving(halves(ImageFormat::Png), 1).await;
        let dir = tempfile::tempdir().unwrap();
        let thumbs = service(dir.path());

        let (one, two) = tokio::join!(
            thumbs.get(&url, tile(50, 32)),
            thumbs.get(&url, tile(100, 64))
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
            .get(&format!("{}/gone.png", server.uri()), tile(50, 32))
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

        let err = thumbs.get(&url, tile(50, 32)).await.unwrap_err();
        assert!(matches!(err, Error::Image(_)), "{err}");
        assert!(!dir.path().join(CACHE_DIR).exists());
    }

    #[tokio::test]
    async fn warming_makes_every_size_ahead_so_a_look_costs_no_fetch() {
        let (_server, url) = serving(halves(ImageFormat::Png), 1).await;
        let dir = tempfile::tempdir().unwrap();
        let thumbs = service(dir.path());

        thumbs.warm(vec![Job {
            url: url.clone(),
            tiles: vec![tile(50, 32), tile(130, 130)],
        }]);
        thumbs.settled().await;
        assert_eq!(kept(dir.path()).len(), 3);

        let room = thumbs.get(&url, tile(130, 130)).await.unwrap();
        let small = image::load_from_memory(&room).unwrap();
        assert_eq!((small.width(), small.height()), (130, 130));
    }

    #[tokio::test]
    async fn a_map_that_fails_does_not_stop_the_ones_after_it() {
        let (server, url) = serving(halves(ImageFormat::Png), 1).await;
        let dir = tempfile::tempdir().unwrap();
        let thumbs = service(dir.path());

        thumbs.warm(vec![
            Job {
                url: format!("{}/gone.png", server.uri()),
                tiles: vec![tile(50, 32)],
            },
            Job {
                url,
                tiles: vec![tile(50, 32)],
            },
        ]);
        thumbs.settled().await;
        assert_eq!(kept(dir.path()).len(), 2);
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
