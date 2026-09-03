//! Map pictures at the size a tile shows them, resized here and kept on disk.
//!
//! The index publishes one picture per map: the 1024px transform the official
//! lobby asks for, which the CDN has cached and marks immutable. The battle
//! list shows it in a tile a twentieth of that size, and a webview scaling a
//! picture down that far aliases — terrain turns to noise. Asking the image
//! server for a smaller transform would make modlobby the only client of that
//! transform, and the picture arrives without CORS headers, so the webview
//! cannot redraw it on a canvas either. So the published picture is fetched
//! once, with the lobby's own client, cut and resized here with a proper
//! filter, and the result kept under `cache/map-thumbs/` for good. The file is
//! named after the URL, which changes when the picture does.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use image::imageops::FilterType;
use image::{ImageFormat, ImageReader};
use md5::{Digest, Md5};

/// Under the config directory's `cache/`.
pub const CACHE_DIR: &str = "map-thumbs";
/// The longest side served. The source is 1024 on its long side; anything
/// bigger would be an upscale, and a tile does not need one.
pub const MAX_SIDE: u32 = 1024;

/// The box a picture is made to fill, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

/// The pictures made this run, and where they are kept.
pub struct Service {
    client: reqwest::Client,
    dir: PathBuf,
    /// One lock per file, so a list showing the same map in twenty rooms
    /// fetches and resizes it once and the other nineteen wait for the file.
    /// Kept for the run: there are as many as maps seen, which is few.
    making: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
}

impl Service {
    pub fn new(client: reqwest::Client, cache_dir: &Path) -> Self {
        Self {
            client,
            dir: cache_dir.join(CACHE_DIR),
            making: Mutex::new(HashMap::new()),
        }
    }

    /// The picture at `url` filling `tile`, as PNG bytes: from disk when it
    /// has been made before, otherwise fetched, made and kept.
    pub async fn get(&self, url: &str, tile: Tile) -> Result<Vec<u8>, Error> {
        let path = self.path(url, tile);
        if let Ok(png) = tokio::fs::read(&path).await {
            return Ok(png);
        }
        let lock = self.lock_for(&path);
        let _making = lock.lock().await;
        // Made by whoever held the lock before us.
        if let Ok(png) = tokio::fs::read(&path).await {
            return Ok(png);
        }
        let png = self.fetch(url, tile).await?;
        write(&path, &png).await?;
        Ok(png)
    }

    fn path(&self, url: &str, tile: Tile) -> PathBuf {
        let hash: String = Md5::digest(url.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        self.dir
            .join(format!("{hash}-{}x{}.png", tile.width, tile.height))
    }

    fn lock_for(&self, path: &Path) -> Arc<tokio::sync::Mutex<()>> {
        self.making
            .lock()
            .expect("the lock map is never poisoned")
            .entry(path.to_owned())
            .or_default()
            .clone()
    }

    async fn fetch(&self, url: &str, tile: Tile) -> Result<Vec<u8>, Error> {
        let response = self.client.get(url).send().await?;
        if !response.status().is_success() {
            return Err(Error::Status(response.status().as_u16()));
        }
        let picture = response.bytes().await?;
        // Decoding and resizing a 1024px picture is tens of milliseconds of
        // CPU, which the async runtime should not sit through.
        let png = tokio::task::spawn_blocking(move || fill(&picture, tile))
            .await
            .expect("resizing does not panic")?;
        Ok(png)
    }
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
async fn write(path: &Path, png: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension("png.tmp");
    tokio::fs::write(&tmp, png).await?;
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

        let kept: Vec<_> = std::fs::read_dir(dir.path().join(CACHE_DIR))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(kept.len(), 1);
        assert!(kept[0].ends_with("-50x32.png"), "{kept:?}");
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
    async fn each_tile_size_is_its_own_file() {
        let (_server, url) = serving(halves(ImageFormat::Png), 2).await;
        let dir = tempfile::tempdir().unwrap();
        let thumbs = service(dir.path());

        thumbs.get(&url, tile(50, 32)).await.unwrap();
        thumbs.get(&url, tile(100, 64)).await.unwrap();
        assert_eq!(
            std::fs::read_dir(dir.path().join(CACHE_DIR))
                .unwrap()
                .count(),
            2
        );
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
    async fn a_body_that_is_not_a_picture_is_an_error() {
        let (_server, url) = serving(b"<html>maintenance</html>".to_vec(), 1).await;
        let dir = tempfile::tempdir().unwrap();
        let thumbs = service(dir.path());

        let err = thumbs.get(&url, tile(50, 32)).await.unwrap_err();
        assert!(matches!(err, Error::Image(_)), "{err}");
    }
}
