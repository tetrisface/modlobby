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
 */
export function devicePixels(tile: Tile): Tile {
  const scale = window.devicePixelRatio || 1
  return {
    width: Math.round(tile.width * scale),
    height: Math.round(tile.height * scale),
  }
}
