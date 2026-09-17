import { createSignal } from 'solid-js'
import { api } from '../ipc/client'
import type { Install } from '../ipc/bindings/Install'
import type { Usage } from '../ipc/bindings/Usage'
import type { WidgetState } from '../ipc/bindings/WidgetState'
import type { WidgetStatus } from '../ipc/bindings/WidgetStatus'
import type { WidgetUsage } from '../ipc/bindings/WidgetUsage'
import type { WindowStats } from '../ipc/bindings/WindowStats'

/**
 * What BAR players actually run, from pve.bar's weekly projection.
 *
 * Fetched once per run and held: the document is rebuilt weekly, so refetching
 * on every visit would spend a request to learn the same thing.
 *
 * `null` means it could not be read. Usage decorates a widget list rather than
 * carrying it, so the page renders without the numbers rather than refusing to
 * render — the same reasoning the Rust side uses in returning an option.
 */
const [usage, setUsage] = createSignal<Usage | null>(null)
const [loaded, setLoaded] = createSignal(false)

/**
 * What modlobby has installed, and what BAR's own config says.
 *
 * Refetched after every action rather than patched in place: the config is a
 * file the game also writes, so what this thinks it did and what is on disk can
 * disagree. Asking is cheap and always right.
 */
const [status, setStatus] = createSignal<WidgetStatus | null>(null)

export { usage, loaded, status }

/** One request, however many callers arrive together. */
let pending: Promise<Usage | null> | null = null

export async function loadWidgetUsage(): Promise<void> {
  if (loaded()) return
  try {
    pending ??= api.widgetUsage()
    const document = await pending
    setUsage(document)
    if (document) {
      setLoaded(true)
      return
    }
    // A document is latched; a failure is not. Rust holds its own failure for
    // as long as the service asked to be left alone, so opening the page again
    // costs a call into Rust and no request at all — and once the service is
    // back, the numbers turn up without the app being restarted.
    pending = null
  } catch {
    // Rust could not be reached at all; the next caller asks again.
    pending = null
  }
}

/** What is installed right now. Cheap, and the only honest source. */
export async function refreshInstalled(): Promise<void> {
  try {
    setStatus(await api.widgetInstalled())
  } catch {
    setStatus(null)
  }
}

/**
 * The windows the pipeline publishes, narrowest first.
 *
 * Mirrors `WINDOWS` in the `widgets` crate. Declared rather than derived from
 * the document's key order, so the fallback below picks the widest window on
 * purpose instead of by whatever order a map happened to serialise in.
 */
export const WINDOW_ORDER = ['7d', '30d', '90d', '365d', 'all'] as const

export const DEFAULT_WINDOW = '30d'

/**
 * Player-versus-what, mirroring the battles list's own filter.
 *
 * `all` is always published and always complete. The split can be absent while
 * the pipeline is still re-reading history under new decoding rules — it
 * withholds a half-answered split rather than publishing one whose coverage
 * figure would be wrong.
 */
export const AUDIENCE_ORDER = ['all', 'pve', 'pvp'] as const
export type Audience = (typeof AUDIENCE_ORDER)[number]

export const DEFAULT_AUDIENCE: Audience = 'all'

/** The windows the document actually carries. */
export function windows(): string[] {
  return usage()?.windows ?? []
}

/** The audiences the document actually carries. */
export function audiences(): string[] {
  return usage()?.audiences ?? [DEFAULT_AUDIENCE]
}

/**
 * Widgets ranked for one audience and window, most used first.
 *
 * A widget missing from a window is absent, not zero: the k-anonymity floor is
 * applied inside each window, so a widget with six players this year and two
 * this week is withheld from the week rather than shown with a two. The same
 * holds across the audience split, which is why it cannot be derived here from
 * a combined total.
 */
export function ranked(audience: string, window: string): WidgetUsage[] {
  const document = usage()
  if (!document) return []
  const withRank = document.widgets.flatMap((widget) => {
    const stats = widget.windows[audience]?.[window]
    return stats ? [{ widget, rank: stats.rank }] : []
  })
  return withRank.sort((a, b) => a.rank - b.rank).map((entry) => entry.widget)
}

/** The stats to show for a widget, falling back when the window withheld it. */
export function statsFor(
  widget: WidgetUsage,
  audience: string,
  window: string,
): { window: string; stats: WindowStats } | null {
  const inAudience =
    widget.windows[audience] ?? widget.windows[DEFAULT_AUDIENCE]
  if (!inAudience) return null
  const wanted = inAudience[window]
  if (wanted) return { window, stats: wanted }
  for (const name of [...WINDOW_ORDER].reverse()) {
    const stats = inAudience[name]
    if (stats) return { window: name, stats }
  }
  return null
}

/**
 * Whether a window holds enough of its days to be presented as that window.
 *
 * The pipeline backfills history a slice at a time, so a year window can
 * legitimately hold a fortnight. Saying so beats implying a year of evidence.
 */
export function isRepresentative(stats: WindowStats): boolean {
  return stats.coverage >= 0.5
}

/** Players who reported a widget but never had it enabled. */
export function disabledOnly(stats: WindowStats): number {
  return Math.max(0, stats.players - stats.players_active)
}

/**
 * Free-text search over what a reader would recognise a widget by.
 *
 * Name, author and description — not the key or the hash, which are ours
 * rather than theirs. Matching is on every word given, in any order and any
 * field, so "ping errrr" finds Errrrrrr's Ping Wheel.
 */
export function matches(widget: WidgetUsage, query: string): boolean {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (terms.length === 0) return true
  const haystack =
    `${widget.name} ${widget.author} ${widget.description}`.toLowerCase()
  return terms.every((term) => haystack.includes(term))
}

/** What a row can do about a widget right now. */
export type Action = 'install' | 'update' | 'disable' | 'enable' | 'delete'

/** Whether modlobby put this widget on disk itself. */
export function installedEntry(widget: WidgetUsage) {
  return status()?.installed.find((entry) => entry.key === widget.key) ?? null
}

/** What BAR's config says about a widget, whoever installed it. */
export function configuredState(widget: WidgetUsage): WidgetState | null {
  return (
    status()?.configured.find((state) => state.name === widget.name) ?? null
  )
}

/**
 * Whether BAR will load this widget.
 *
 * `order = 0` is how BAR records a widget the player switched off. The value
 * arrives as a bigint because it is an i64 on the Rust side, so it is compared
 * against `0n` rather than `0`.
 */
export function isEnabled(state: WidgetState): boolean {
  return state.order !== 0n
}

/**
 * Whether the published files differ from what was installed.
 *
 * Compared by content hash rather than by a version string: a widget posted to
 * Discord has no version, and the ones that do have a version routinely forget
 * to bump it.
 */
export function isOutdated(widget: WidgetUsage): boolean {
  const entry = installedEntry(widget)
  if (!entry) return false
  const published = widget.install.files.map((file) => file.content_hash)
  if (published.length === 0) return false
  return (
    published.length !== entry.hashes.length ||
    published.some((hash, at) => hash !== entry.hashes[at])
  )
}

/**
 * The actions worth offering for one widget.
 *
 * Install and update are mutually exclusive; disable and enable depend on
 * BAR's config rather than on our ledger, because a widget can be switched off
 * whether or not modlobby is what put it there. Delete is offered only for
 * what modlobby installed: removing files it does not own is not its business.
 */
export function actionsFor(widget: WidgetUsage): Action[] {
  const entry = installedEntry(widget)
  const configured = configuredState(widget)
  const actions: Action[] = []
  if (!entry && widget.install.url) actions.push('install')
  if (entry && isOutdated(widget)) actions.push('update')
  if (configured && isEnabled(configured)) actions.push('disable')
  if (configured && !isEnabled(configured)) actions.push('enable')
  if (entry) actions.push('delete')
  return actions
}

/** Why a widget cannot be downloaded, when it cannot. */
export function unavailableBecause(install: Install): string | null {
  if (install.url && install.files.length > 0) return null
  if (install.reason) return install.reason
  return install.kind === 'none' ? 'no known source' : 'no download available'
}
