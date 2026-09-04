/**
 * Map pictures and spring names, from BAR's published map index.
 *
 * The picture URL is not derivable from a map's name — it points into an
 * imagor bucket keyed by the map's photo reference — so the index is the only
 * way to find it. Rust keeps the index (`content::map_index`): fetched with
 * the lobby's own User-Agent, cached on disk, and asked for again with its
 * ETag once a day, which the server answers with a bodiless 304.
 *
 * The webview never loads a picture from the CDN itself. It asks Rust for one
 * at the size it will draw (`mapThumb`), and Rust fetches the published
 * picture — the same 1024px transform the official lobby asks for, so it comes
 * out of a shared CDN cache — once, keeps it, and resizes from that copy.
 *
 * A lobby has to work with no network at all: every failure here returns
 * nothing and the room falls back to the start-box schematic, which needs
 * nothing.
 */

import { convertFileSrc } from '@tauri-apps/api/core'
import type { MapIndex } from '../ipc/bindings/MapIndex'
import type { Tile } from '../ipc/bindings/Tile'
import { api } from '../ipc/client'

/**
 * The boxes map pictures are drawn in, in CSS pixels, where the box is fixed
 * by the stylesheet. Named here so that warming ahead asks for exactly what
 * drawing will. Change these with the CSS they mirror.
 */
export const TILES = {
  /** Inside `.col-thumb` in the battle list: 52×34 less a 1px border. */
  list: { width: 50, height: 32 },
  /** `.minimap` in the room card's 132px column, less a 1px border. */
  minimap: { width: 130, height: 130 },
} as const satisfies Record<string, Tile>

/** Where an earlier version kept its own copy; shed once, then never seen. */
const OLD_CACHE_KEY = 'modlobby.mapImages'

let pending: Promise<MapIndex> | null = null

function load(): Promise<MapIndex> {
  try {
    localStorage.removeItem(OLD_CACHE_KEY)
  } catch {
    // Storage the webview refuses; nothing to shed.
  }
  return api.mapIndex()
}

/** One in-flight load, however many callers ask at once. */
async function index(): Promise<MapIndex | null> {
  try {
    pending ??= load()
    return await pending
  } catch {
    // Rust could not be reached; the next caller asks again.
    pending = null
    return null
  }
}

/**
 * Archive file name (without extension) to the map's spring name.
 *
 * A start script needs the spring name, and nothing on disk records the
 * capitalisation the engine expects — only this index does.
 */
export async function mapNames(): Promise<MapIndex['names']> {
  return (await index())?.names ?? {}
}

/**
 * The picture for a spring map name at the size it is drawn, as a URL the
 * webview loads like any other image. Rust resizes the published picture with
 * a real filter and keeps the result (`content::map_thumb`), because a webview
 * scaling a 1024px picture into a 50px tile aliases. A name with no picture
 * answers 404, which reaches the `<img>` as an `error` event.
 *
 * `width` and `height` are CSS pixels. The picture is asked for in device
 * pixels, so that it is drawn one to one and nothing is scaled again.
 */
export function mapThumb(
  springName: string,
  width: number,
  height: number,
): string | null {
  if (!springName) return null
  const tile = devicePixels({ width, height })
  return convertFileSrc(`${tile.width}x${tile.height}/${springName}`, 'thumb')
}

/** A CSS-pixel box in the device pixels it is drawn with. */
function devicePixels(tile: Tile): Tile {
  const scale = window.devicePixelRatio || 1
  return {
    width: Math.round(tile.width * scale),
    height: Math.round(tile.height * scale),
  }
}

/**
 * Asks Rust to make the pictures of `springNames`, in that order, at every
 * fixed size the lobby draws, so that joining a room from the list shows its
 * map at once. Nothing is scheduled: it runs when the list changes, on one
 * worker in the lobby process, and the newest list replaces what was queued.
 */
export async function warmMapPictures(springNames: string[]): Promise<void> {
  if (springNames.length === 0) return
  const tiles = Object.values(TILES).map(devicePixels)
  try {
    await api.warmMapPictures(springNames, tiles)
  } catch {
    // Rust could not be reached; the pictures are made on demand instead.
  }
}
