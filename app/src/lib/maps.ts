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

type Cache = { fetchedAt: number; images: Record<string, string> }
type IndexEntry = { springName?: string; images?: { preview?: string } }

let pending: Promise<Record<string, string>> | null = null

function readCache(): Record<string, string> | null {
  try {
    const raw = localStorage.getItem(CACHE_KEY)
    if (!raw) return null
    const cache = JSON.parse(raw) as Cache
    const age = Date.now() - cache.fetchedAt
    if (age > CACHE_DAYS * 86_400_000) return null
    return cache.images
  } catch {
    // A private window, cleared site data, or storage the webview refuses.
    return null
  }
}

function writeCache(images: Record<string, string>) {
  try {
    localStorage.setItem(
      CACHE_KEY,
      JSON.stringify({ fetchedAt: Date.now(), images } satisfies Cache),
    )
  } catch {
    // Losing the cache costs one fetch, so it is not worth reporting.
  }
}

async function load(): Promise<Record<string, string>> {
  const cached = readCache()
  if (cached) return cached

  const response = await fetch(INDEX_URL)
  if (!response.ok) throw new Error(`map index: ${response.status}`)

  const entries = (await response.json()) as IndexEntry[]
  const images: Record<string, string> = {}
  for (const entry of entries) {
    const name = entry.springName
    const preview = entry.images?.preview
    if (name && preview) images[name] = preview
  }

  writeCache(images)
  return images
}

/** The preview URL for a spring map name, or null if we cannot find one. */
export async function mapImage(springName: string): Promise<string | null> {
  if (!springName) return null
  try {
    // One in-flight load, however many rooms ask at once.
    pending ??= load()
    return (await pending)[springName] ?? null
  } catch {
    // Offline, blocked, or the index moved: the schematic still draws.
    pending = null
    return null
  }
}
