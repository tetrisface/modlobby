//! `thumb://localhost/<width>x<height>/<spring name>`: a map's picture at the
//! size a tile shows it, made and kept by `content::map_thumb`;
//! `thumb://localhost/full/<spring name>`, the picture as published, for a
//! box to show while its size is cut; and
//! `thumb://localhost/news/<width>x<height>/<guid>`, the banner a news item
//! published, which the webview cannot fetch itself; and
//! `thumb://localhost/widget/<width>x<height>/<n>/<usage key>`, picture `n` of
//! a widget from the usage document.
//!
//! A widget's small tile and its enlarged view are two sizes cut from one
//! download: the published picture is fetched once and kept, so opening the
//! larger one costs a resize and no request.
//!
//! A URI scheme rather than a command, so the picture stays an `<img src>`:
//! the webview loads it when the row scrolls into view, keeps it, and reports
//! a failure as the `error` event the tile already handles. On Windows the
//! webview spells the scheme `http://thumb.localhost/`, which is why the CSP
//! names both forms.
//!
//! Every form names a *key*, never a URL, and the key is looked up in
//! something the app fetched itself — the map index, or the news feed. That is
//! what keeps this from being an open image proxy for anything a page cares to
//! ask for.

use content::map_thumb::{self, Tile};
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::state::App;

pub const SCHEME: &str = "thumb";

/// Which store the rest of the path names a picture in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// A map, by its spring name.
    Map,
    /// A news item, by the permalink that identifies it.
    News,
    /// Picture `n` of a widget or one of its forks, by its key in the usage
    /// document. An index into the list the document published, so still
    /// never a URL a caller chose.
    Widget(usize),
}

/// The most pictures a widget publishes; an index past it is refused outright
/// rather than looked up.
const MAX_WIDGET_PICTURES: usize = 8;

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
    let Some((source, tile, key)) = parse(path) else {
        return status(StatusCode::BAD_REQUEST);
    };
    let state = app.state::<App>();
    let url = match source {
        Source::Map => state.map_index().await.images.get(&key).cloned(),
        Source::News => state
            .news()
            .await
            .into_iter()
            .find(|item| item.id == key)
            .and_then(|item| item.image),
        Source::Widget(index) => state
            .widget_usage()
            .await
            .and_then(|usage| usage.image(&key, index).map(str::to_owned)),
    };
    let Some(url) = url else {
        return status(StatusCode::NOT_FOUND);
    };
    let made = match tile {
        // A map's tile is cut to fill it — a bigger row shows more of the
        // middle of the terrain. A widget's picture is a screenshot, and what
        // it is a screenshot of is often at an edge, so it comes whole and the
        // tile pads it.
        Some(tile) => match source {
            Source::Widget(_) => state.thumbs.get_whole(&url, tile).await,
            _ => state.thumbs.get(&url, tile).await,
        }
        .map(|png| ("image/png", png)),
        None => state
            .thumbs
            .published(&url)
            .await
            .map(|picture| (map_thumb::mime(&picture), picture)),
    };
    match made {
        Ok((mime, body)) => Response::builder()
            .header(header::CONTENT_TYPE, mime)
            // The webview may keep it as long as whatever named it is trusted;
            // a map's picture changes about as often as the index does, and a
            // story's banner does not change at all once it is published.
            .header(header::CACHE_CONTROL, "private, max-age=86400")
            .body(body)
            .expect("a response from static headers"),
        Err(err) => {
            tracing::debug!(picture = %key, %err, "no thumbnail");
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
/// picture as published, `None` here — and `/news/<width>x<height>/<guid>` for
/// a story's banner. Percent-encoded as the webview sends it: the JS side
/// encodes the whole path, slash included, so the split comes after decoding.
/// A map name may itself contain a slash and a guid is a whole URL, so only
/// the first slash of each segment counts.
///
/// The two are told apart by the first segment, which for a map is always its
/// size: no size is spelled `news`, so a map called `news` still reads as one.
fn parse(path: &str) -> Option<(Source, Option<Tile>, String)> {
    let decoded = percent_encoding::percent_decode_str(path.trim_start_matches('/'))
        .decode_utf8()
        .ok()?;
    let (head, rest) = decoded.split_once('/')?;
    if head == "news" || head == "widget" {
        let (size, id) = rest.split_once('/')?;
        let tile = tile(size)?;
        let (source, id) = if head == "news" {
            (Source::News, id)
        } else {
            // `widget/<size>/<index>/<key>`: the index before the key, because a
            // key may itself contain slashes and only the first one counts.
            let (index, key) = id.split_once('/')?;
            let index: usize = index.parse().ok()?;
            (index < MAX_WIDGET_PICTURES).then_some(())?;
            (Source::Widget(index), key)
        };
        return (!id.is_empty()).then(|| (source, Some(tile), id.to_owned()));
    }
    let tile = match head {
        "full" => None,
        _ => Some(tile(head)?),
    };
    (!rest.is_empty()).then(|| (Source::Map, tile, rest.to_owned()))
}

fn tile(size: &str) -> Option<Tile> {
    let (width, height) = size.split_once('x')?;
    Tile::new(width.parse().ok()?, height.parse().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_names_a_tile_and_a_map() {
        let (source, tile, name) = parse("/50x32/AcidicQuarry%205.17").unwrap();
        let tile = tile.unwrap();
        assert_eq!(source, Source::Map);
        assert_eq!((tile.width, tile.height), (50, 32));
        assert_eq!(name, "AcidicQuarry 5.17");
    }

    #[test]
    fn full_is_the_picture_as_published() {
        let (source, tile, name) = parse("/full/AcidicQuarry%205.17").unwrap();
        assert_eq!(source, Source::Map);
        assert!(tile.is_none());
        assert_eq!(name, "AcidicQuarry 5.17");
    }

    #[test]
    fn the_slash_may_arrive_encoded_too() {
        let (_, _, name) = parse("/50x32%2FAcidicQuarry%205.17").unwrap();
        assert_eq!(name, "AcidicQuarry 5.17");
    }

    #[test]
    fn a_name_keeps_its_own_slashes() {
        let (_, _, name) = parse("/50x32/Odd%2FName%201").unwrap();
        assert_eq!(name, "Odd/Name 1");
    }

    #[test]
    fn a_news_path_names_a_tile_and_the_story_that_published_the_picture() {
        let (source, tile, id) =
            parse("/news/320x180/https%3A%2F%2Fwww.beyondallreason.info%2Fnews%2Fthe-lore")
                .unwrap();
        let tile = tile.unwrap();
        assert_eq!(source, Source::News);
        assert_eq!((tile.width, tile.height), (320, 180));
        // The whole permalink, slashes and all: it is the id, not a path.
        assert_eq!(id, "https://www.beyondallreason.info/news/the-lore");
    }

    #[test]
    fn a_widget_path_names_a_tile_a_picture_and_the_usage_key() {
        let (source, tile, key) = parse("/widget/96x60/2/widget%3Agui_fire").unwrap();
        assert_eq!(source, Source::Widget(2));
        assert_eq!(tile, Tile::new(96, 60));
        assert_eq!(key, "widget:gui_fire");
    }

    #[test]
    fn a_fork_key_keeps_its_own_slashes() {
        let (source, _, key) =
            parse("/widget/96x60/0/github%3Aowner%2Frepo%3APing%20Wheel").unwrap();
        assert_eq!(source, Source::Widget(0));
        assert_eq!(key, "github:owner/repo:Ping Wheel");
    }

    #[test]
    fn a_widget_path_without_a_size_is_refused() {
        // No "full" form: a widget picture is only ever served cut to a box,
        // so a caller cannot ask the lobby to relay an original of any size.
        assert!(parse("/widget/full/0/widget:gui_fire").is_none());
    }

    #[test]
    fn a_widget_path_without_a_valid_picture_index_is_refused() {
        assert!(
            parse("/widget/96x60/widget:gui_fire").is_none(),
            "no index at all"
        );
        assert!(
            parse("/widget/96x60/x/widget:gui_fire").is_none(),
            "not a number"
        );
        assert!(
            parse("/widget/96x60/8/widget:gui_fire").is_none(),
            "past the most published"
        );
    }

    #[test]
    fn a_map_named_news_is_still_a_map() {
        let (source, _, name) = parse("/50x32/news").unwrap();
        assert_eq!(source, Source::Map);
        assert_eq!(name, "news");
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
            // A news path with no size, or no story after it.
            "/news/https%3A%2F%2Fx",
            "/news/320x180/",
            "/news/full/https%3A%2F%2Fx",
        ] {
            assert!(parse(path).is_none(), "{path}");
        }
    }
}
