/**
 * Minimap images, from BAR's published map index.
 *
 * The image URL is not derivable from a map's name — it points into an imagor
 * bucket keyed by the map's photo reference — so the index is the only way to
 * find it (`maps-metadata/scripts/js/src/sync_to_webflow.ts`). Its `springName`
 * is exactly the name `BATTLEOPENED` reports, so the lookup is a plain match.
 *
 * The index is ~950 kB of JSON for 225 maps and we need two fields, so what is
 * kept is the trimmed pairs, in `localStorage`, for a day. A lobby has to work
 * with no network at all: every failure here returns null and the room falls
 * back to the start-box schematic, which needs nothing.
 */

const INDEX_URL =
  'https://maps-metadata.beyondallreason.dev/latest/lobby_maps.validated.json'
const CACHE_KEY = 'modlobby.mapImages'
const CACHE_DAYS = 1

type Cache = {
  fetchedAt: number
  images: Record<string, string>
  /** Archive file name without its extension, to the map's spring name. */
  names: Record<string, string>
}
type IndexEntry = {
  springName?: string
  filename?: string
  images?: { preview?: string }
}

type Index = { images: Record<string, string>; names: Record<string, string> }

let pending: Promise<Index> | null = null

function readCache(): Index | null {
  try {
    const raw = localStorage.getItem(CACHE_KEY)
    if (!raw) return null
    const cache = JSON.parse(raw) as Cache
    const age = Date.now() - cache.fetchedAt
    if (age > CACHE_DAYS * 86_400_000) return null
    if (!cache.names) return null
    return { images: cache.images, names: cache.names }
  } catch {
    // A private window, cleared site data, or storage the webview refuses.
    return null
  }
}

function writeCache(index: Index) {
  try {
    localStorage.setItem(
      CACHE_KEY,
      JSON.stringify({ fetchedAt: Date.now(), ...index } satisfies Cache),
    )
  } catch {
    // Losing the cache costs one fetch, so it is not worth reporting.
  }
}

async function load(): Promise<Index> {
  const cached = readCache()
  if (cached) return cached

  const response = await fetch(INDEX_URL)
  if (!response.ok) throw new Error(`map index: ${response.status}`)

  const entries = (await response.json()) as IndexEntry[]
  const index: Index = { images: {}, names: {} }
  for (const entry of entries) {
    const name = entry.springName
    if (!name) continue
    const preview = entry.images?.preview
    if (preview) index.images[name] = preview
    // `acidicquarry_5.17.sd7` is what sits in the maps directory; the stem is
    // what a caller listing that directory has to match on.
    const file = entry.filename?.replace(/\.sd[7z]$/i, '')
    if (file) index.names[file] = name
  }

  writeCache(index)
  return index
}

/**
 * Every preview URL by spring name, for a list that wants many at once.
 *
 * One shared load: asking per row would start a hundred identical fetches on
 * the first scroll. Returns empty when the index cannot be reached, which the
 * caller renders as no picture rather than as an error.
 */
/**
 * The same picture, asked for at the size it will be drawn.
 *
 * The published URL is an imagor transform with the size in its path — a
 * 1024px fit-in at quality 75 — and the server is unsigned, so the size can be
 * asked for. That is worth doing twice over: a battle-list thumbnail is 52px
 * wide, and letting the browser take 1024px down to that is a twenty-fold
 * downscale done with a fast filter, which is what makes a map look like
 * gravel. Asking imagor for it instead resamples with libvips, and the
 * download goes from 198 KiB to 27 KiB.
 *
 * A URL that is not shaped like that transform is returned untouched.
 */
export function sized(url: string, pixels: number): string {
  return url
    .replace(/\/fit-in\/\d+x\d+\//, `/fit-in/${pixels}x${pixels}/`)
    .replace(/quality\(\d+\)/, 'quality(90)')
}

export async function mapImages(): Promise<Record<string, string>> {
  try {
    pending ??= load()
    return (await pending).images
  } catch {
    pending = null
    return {}
  }
}

/**
 * Archive file name (without extension) to the map's spring name.
 *
 * A start script needs the spring name, and nothing on disk records the
 * capitalisation the engine expects — only this index does.
 */
export async function mapNames(): Promise<Record<string, string>> {
  try {
    pending ??= load()
    return (await pending).names
  } catch {
    pending = null
    return {}
  }
}

/** The preview URL for a spring map name, or null if we cannot find one. */
export async function mapImage(springName: string): Promise<string | null> {
  if (!springName) return null
  try {
    // One in-flight load, however many callers ask at once.
    pending ??= load()
    return (await pending).images[springName] ?? null
  } catch {
    // Offline, blocked, or the index moved: the schematic still draws.
    pending = null
    return null
  }
}
