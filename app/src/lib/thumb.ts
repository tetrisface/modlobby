/**
 * Pictures the app fetches on the webview's behalf, over the `thumb:` scheme.
 *
 * The webview cannot reach the hosts these live on — the content policy names
 * no remote origin — so Rust fetches, resizes and keeps them, and the page
 * asks for one by a key it already has: a map's spring name, or the permalink
 * of the story that published a banner. They stay ordinary `<img src>` URLs,
 * so the browser loads them lazily, keeps them, and reports a miss as the
 * `error` event a tile already handles.
 */

import { convertFileSrc } from '@tauri-apps/api/core'
import type { Tile } from '../ipc/bindings/Tile'
import { uiScale } from '../store/settings'

/**
 * A URL for `path` under the scheme. On Windows the webview spells it
 * `http://thumb.localhost/`, which is why this goes through Tauri's helper
 * rather than building the string.
 */
export function thumbSrc(path: string): string {
	return convertFileSrc(path, 'thumb')
}

/**
 * A CSS-pixel box in the device pixels it is drawn with, so the picture comes
 * back at one to one and nothing is scaled a second time.
 *
 * Shared by every kind of picture the scheme serves: two of these would be two
 * answers to how the app scales, and one of them would eventually be wrong.
 *
 * Sizing the interface counts as well as the screen does: every box is drawn
 * in `rem`, so ctrl+wheel makes each one bigger without making its picture
 * bigger, and the tile is stretched. Asking for the larger size costs a local
 * decode and resize — Rust keeps the published picture and cuts every size
 * from that copy (`content::map_thumb`), so nothing is fetched again.
 */
export function devicePixels(tile: Tile): Tile {
	const scale = (window.devicePixelRatio || 1) * zoom()
	return {
		width: Math.round(tile.width * scale),
		height: Math.round(tile.height * scale),
	}
}

/**
 * The interface size as a factor, in quarters.
 *
 * Each distinct size is a picture of its own on disk, and the wheel moves in
 * tens of a percent — so the steps between are rounded together, and a tile
 * up to an eighth off is drawn by the box that already scales it anyway.
 */
function zoom(): number {
	return Math.round(uiScale() / 25) / 4 || 1
}
