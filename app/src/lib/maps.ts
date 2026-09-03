/**
 * Minimap images and spring names, from BAR's published map index.
 *
 * The image URL is not derivable from a map's name — it points into an imagor
 * bucket keyed by the map's photo reference — so the index is the only way to
 * find it. Rust keeps the index (`content::map_index`): fetched with the
 * lobby's own User-Agent, cached on disk, and asked for again with its ETag
 * once a day, which the server answers with a bodiless 304.
 *
 * The URL is used exactly as published. It is the same 1024px transform the
 * official lobby asks for, so it comes out of a shared CDN cache, and the CDN
 * marks it immutable, so the webview keeps it for good. A size of our own
 * would be a transform only modlobby asks BAR's image server to compute. Where
 * a picture is shown far smaller than that, Rust resizes it (`mapThumb`).
 *
 * A lobby has to work with no network at all: every failure here returns
 * nothing and the room falls back to the start-box schematic, which needs
 * nothing.
 */

import { convertFileSrc } from '@tauri-apps/api/core'
import type { MapIndex } from '../ipc/bindings/MapIndex'
import { api } from '../ipc/client'

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

/** The preview URL for a spring map name, or null if we cannot find one. */
export async function mapImage(springName: string): Promise<string | null> {
  if (!springName) return null
  return (await index())?.images[springName] ?? null
}

/**
 * The picture for a spring map name at the size a tile shows it, as a URL the
 * webview loads like any other image. Rust fetches the published picture once,
 * resizes it with a real filter and keeps the result (`content::map_thumb`),
 * because a webview scaling a 1024px picture into a 50px tile aliases. A name
 * with no picture answers 404, which reaches the tile as an `error` event.
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
  const scale = window.devicePixelRatio || 1
  const tile = `${Math.round(width * scale)}x${Math.round(height * scale)}`
  return convertFileSrc(`${tile}/${springName}`, 'thumb')
}
