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
 * would be a transform only modlobby asks BAR's image server to compute.
 *
 * A lobby has to work with no network at all: every failure here returns
 * nothing and the room falls back to the start-box schematic, which needs
 * nothing.
 */

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
 * Every preview URL by spring name, for a list that wants many at once.
 * Empty when the index cannot be had, which the caller renders as no picture.
 */
export async function mapImages(): Promise<MapIndex['images']> {
  return (await index())?.images ?? {}
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
