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
 * The copy as it came is on offer too (`mapPicture`), for a box to show
 * while its size is being cut.
 *
 * A lobby has to work with no network at all: every failure here returns
 * nothing and the room falls back to the start-box schematic, which needs
 * nothing.
 */

import type { MapIndex } from '../ipc/bindings/MapIndex'
import type { Tile } from '../ipc/bindings/Tile'
import { api } from '../ipc/client'
import { devicePixels, thumbSrc } from './thumb'

/**
 * The boxes map pictures are drawn in, in CSS pixels, where the box is fixed
 * by the stylesheet. Named here so that warming ahead asks for exactly what
 * drawing will. Change these with the CSS they mirror. Each is cut to fill
 * its box; `LIST_TILE` is the one drawn whole.
 */
export const TILES = {
	/** `.minimap` in the room card's 132px column, less a 1px border. */
	minimap: { width: 130, height: 130 },
	/** `.nav-room-pic`, at the left of the room card in the nav. */
	nav: { width: 40, height: 28 },
} as const satisfies Record<string, Tile>

/**
 * Inside `.col-thumb` in the battle list: 50 wide, and as tall as a 3rem row
 * leaves it, both less the 1px border. Fitted whole inside the box rather
 * than cut to fill it — the map's shape is part of what the list says
 * about it — so a map that is not this shape is padded, never cropped.
 */
export const LIST_TILE = { width: 50, height: 41 } as const satisfies Tile

/**
 * `.map-card .map-pic` in the map picker's grid, which stretches from its
 * 150px minimum; asked for at the wide end so it is never scaled up much.
 *
 * Apart from `TILES` because that is the list of sizes warmed ahead, and a
 * thousand map cards are not worth making for a picker nobody has opened.
 * The grid asks as it scrolls instead.
 */
export const CARD_TILE = { width: 180, height: 180 } as const satisfies Tile

/** `.map-row .map-pic` in the picker's list layout: a thumbnail beside a name. */
export const ROW_TILE = { width: 44, height: 28 } as const satisfies Tile

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
 * A map's spring name, read back out of its archive's file name.
 *
 * For a map the published index has never heard of — installed by hand —
 * where there is nothing else to go on. The underscores were spaces, and the
 * engine resolves a name by splitting it on whitespace and looking for each
 * word case-folded; a name with the underscores left in is one word matching
 * nothing, and the game stops with `Dependent archive "..." not found`
 * before it opens a window. The mirror of Rust's `map_name_from_stem`.
 */
export function mapNameFromFile(file: string): string {
	return file.replace(/_/g, ' ')
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
 * What the map list shows about each map, by spring name: its author's name
 * for it, how big it is, how many it takes. Empty where the index could not
 * be read, which leaves a list of names and no columns.
 */
export async function mapFacts(): Promise<MapIndex['maps']> {
	return (await index())?.maps ?? {}
}

/** The version a map's name ends in: ` v3.4.4`, `_V4`, ` 1.3`. */
const VERSION = /[\s_-]+v?\d[\w.]*$/i

/** A map's name less its version, to tell versions of one map by:
 *  `Aurelia v4.1` and `Aurelia_V4` are both `aurelia`. */
function family(spring: string): string {
	return spring.replace(VERSION, '').replace(/_/g, ' ').trim().toLowerCase()
}

/**
 * What a list calls each of its maps, by spring name: the name less its
 * version — `Supreme Isthmus` for `Supreme Isthmus v1.8` — except where the
 * list holds two versions of one map, which keep their full names to be
 * told apart by.
 */
export function shownMapNames(
	names: readonly string[],
): ReadonlyMap<string, string> {
	const versions = new Map<string, Set<string>>()
	for (const name of names) {
		const kin = versions.get(family(name)) ?? new Set<string>()
		kin.add(name)
		versions.set(family(name), kin)
	}
	return new Map(
		names.map((name) => {
			if (versions.get(family(name))!.size > 1) return [name, name]
			return [name, name.replace(VERSION, '') || name]
		}),
	)
}

/**
 * The name beyondallreason.info lists a map under, which is all its search
 * matches: the author's name for it, with no version. A version the index
 * no longer lists goes by the name of one it does. `null` for a map that is
 * not BAR's, which the site has nothing on.
 */
export function mapSiteName(
	spring: string,
	facts: MapIndex['maps'],
): string | null {
	const exact = facts[spring]?.displayName
	if (exact) return exact
	const wanted = family(spring)
	const kin = Object.entries(facts).find(
		([name, about]) => about.displayName && family(name) === wanted,
	)
	return kin?.[1].displayName ?? null
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
 * `whole` asks for it fitted inside the box with nothing cut off, for a
 * box that pads it (`object-fit: contain`).
 */
export function mapThumb(
	springName: string,
	width: number,
	height: number,
	whole = false,
): string | null {
	if (!springName) return null
	const tile = devicePixels({ width, height })
	const size = `${tile.width}x${tile.height}/${springName}`
	return thumbSrc(whole ? `whole/${size}` : size)
}

/**
 * The picture as published, for a box to show — scaled by the webview, so
 * soft — while Rust cuts its size. A name with no picture answers 404 here
 * too.
 */
export function mapPicture(springName: string): string | null {
	if (!springName) return null
	return thumbSrc(`full/${springName}`)
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
		await api.warmMapPictures(springNames, tiles, [devicePixels(LIST_TILE)])
	} catch {
		// Rust could not be reached; the pictures are made on demand instead.
	}
}
