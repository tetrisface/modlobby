//! `thumb://localhost/<width>x<height>/<spring name>`: a map's picture at the
//! size a tile shows it, made and kept by `content::map_thumb`; and
//! `thumb://localhost/full/<spring name>`, the picture as published, for a
//! box to show while its size is cut.
//!
//! A URI scheme rather than a command, so the picture stays an `<img src>`:
//! the webview loads it when the row scrolls into view, keeps it, and reports
//! a failure as the `error` event the tile already handles. On Windows the
//! webview spells the scheme `http://thumb.localhost/`, which is why the CSP
//! names both forms.

use content::map_thumb::{self, Tile};
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::state::App;

pub const SCHEME: &str = "thumb";

/// The handler Tauri calls per request; the work is on the async runtime so
/// the webview's thread is not held while a picture is fetched.
pub fn serve<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        responder.respond(respond(&app, request.uri().path()).await);
    });
}

async fn respond<R: Runtime>(app: &tauri::AppHandle<R>, path: &str) -> Response<Vec<u8>> {
    let Some((tile, name)) = parse(path) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let state = app.state::<App>();
    let index = state.map_index().await;
    let Some(url) = index.images.get(&name) else {
        return status(StatusCode::NOT_FOUND);
    };
    let made = match tile {
        Some(tile) => state
            .thumbs
            .get(url, tile)
            .await
            .map(|png| ("image/png", png)),
        None => state
            .thumbs
            .published(url)
            .await
            .map(|picture| (map_thumb::mime(&picture), picture)),
    };
    match made {
        Ok((mime, body)) => Response::builder()
            .header(header::CONTENT_TYPE, mime)
            // The webview may keep it as long as the index is trusted; the
            // picture behind a name changes about as often as the index does.
            .header(header::CACHE_CONTROL, "private, max-age=86400")
            .body(body)
            .expect("a response from static headers"),
        Err(err) => {
            tracing::debug!(map = %name, %err, "no thumbnail");
            status(StatusCode::BAD_GATEWAY)
        }
    }
}

fn status(code: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(code)
        .body(Vec::new())
        .expect("a response from a status")
}

/// `/<width>x<height>/<spring name>` — or `/full/<spring name>` for the
/// picture as published, `None` here — percent-encoded as the webview sends
/// it: the JS side encodes the whole path, slash included, so the split comes
/// after decoding. A name may itself contain a slash, so only the first one
/// counts.
fn parse(path: &str) -> Option<(Option<Tile>, String)> {
    let decoded = percent_encoding::percent_decode_str(path.trim_start_matches('/'))
        .decode_utf8()
        .ok()?;
    let (size, name) = decoded.split_once('/')?;
    let tile = match size {
        "full" => None,
        _ => {
            let (width, height) = size.split_once('x')?;
            Some(Tile::new(width.parse().ok()?, height.parse().ok()?)?)
        }
    };
    (!name.is_empty()).then(|| (tile, name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_names_a_tile_and_a_map() {
        let (tile, name) = parse("/50x32/AcidicQuarry%205.17").unwrap();
        let tile = tile.unwrap();
        assert_eq!((tile.width, tile.height), (50, 32));
        assert_eq!(name, "AcidicQuarry 5.17");
    }

    #[test]
    fn full_is_the_picture_as_published() {
        let (tile, name) = parse("/full/AcidicQuarry%205.17").unwrap();
        assert!(tile.is_none());
        assert_eq!(name, "AcidicQuarry 5.17");
    }

    #[test]
    fn the_slash_may_arrive_encoded_too() {
        let (_, name) = parse("/50x32%2FAcidicQuarry%205.17").unwrap();
        assert_eq!(name, "AcidicQuarry 5.17");
    }

    #[test]
    fn a_name_keeps_its_own_slashes() {
        let (_, name) = parse("/50x32/Odd%2FName%201").unwrap();
        assert_eq!(name, "Odd/Name 1");
    }

    #[test]
    fn anything_else_is_refused() {
        for path in [
            "/",
            "/AcidicQuarry%205.17",
            "/50x32/",
            "/full/",
            "/0x32/Map",
            "/50x9999/Map",
            "/50/Map",
            "/ax32/Map",
            "/50x32%FF/Map",
        ] {
            assert!(parse(path).is_none(), "{path}");
        }
    }
}
