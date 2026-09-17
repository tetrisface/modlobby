import { createSignal } from 'solid-js'
import { api } from '../ipc/client'
import type { Fork } from '../ipc/bindings/Fork'
import type { Install } from '../ipc/bindings/Install'
import type { LocalWidget } from '../ipc/bindings/LocalWidget'
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
 * How keeping a widget is counted.
 *
 * `still`: the player's latest replay in the window had it enabled — switching
 * it off for one game and back on still counts, switching it off for good does
 * not. `once`: enabled in any replay. "Still using" is the default because it
 * is the one that notices a widget people tried and dropped.
 */
export type UsingMode = 'still' | 'once'

export const DEFAULT_USING_MODE: UsingMode = 'still'

/** What the column is called in each mode. */
export const USING_LABEL: Record<UsingMode, string> = {
  still: 'Still using',
  once: 'Used once',
}

/** Players counted as using it, in the chosen mode. */
export function usingCount(stats: WindowStats, mode: UsingMode): number {
  return mode === 'still' ? stats.players_still_using : stats.players_active
}

/** The share of players counted as using it, in the chosen mode. */
export function usingShare(stats: WindowStats, mode: UsingMode): number {
  return mode === 'still' ? stats.still_using : stats.retention
}

/** Players not counted as using it: the "Off" column, in the chosen mode. */
export function notUsing(stats: WindowStats, mode: UsingMode): number {
  return Math.max(0, stats.players - usingCount(stats, mode))
}

/**
 * Free-text search over what a reader would recognise a widget by.
 *
 * Name, author and description — not the key or the hash, which are ours
 * rather than theirs. Matching is on every word given, in any order and any
 * field, so "ping errrr" finds Errrrrrr's Ping Wheel. A fork's author counts
 * too, so searching for a forker finds the row their version sits under.
 */
export function matches(widget: WidgetUsage, query: string): boolean {
  const terms = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (terms.length === 0) return true
  const forkAuthors = forksOf(widget)
    .map((fork) => fork.author)
    .join(' ')
  const haystack =
    `${widget.name} ${widget.author} ${widget.description} ${forkAuthors}`.toLowerCase()
  return terms.every((term) => haystack.includes(term))
}

/**
 * Every version under a widget name, main first.
 *
 * A document from before forks existed has none listed; the row itself then
 * stands in as its only, main, version, so everything below works on either.
 */
export function forksOf(widget: WidgetUsage): [Fork, ...Fork[]] {
  const [first, ...rest] = widget.forks
  if (first) return [first, ...rest]
  return [
    {
      key: widget.main || widget.key,
      kind: 'lineage',
      main: true,
      id: widget.id,
      author: widget.author,
      description: widget.description,
      install: widget.install,
      image: widget.image,
      windows: widget.windows,
    },
  ]
}

/** The version the row speaks for. */
export function mainFork(widget: WidgetUsage): Fork {
  const forks = forksOf(widget)
  return forks.find((fork) => fork.main) ?? forks[0]
}

/** A fork's own numbers in one slice, falling back like a row's do. */
export function forkStatsFor(
  fork: Fork,
  audience: string,
  window: string,
): WindowStats | null {
  const inAudience = fork.windows[audience] ?? fork.windows[DEFAULT_AUDIENCE]
  if (!inAudience) return null
  if (inAudience[window]) return inAudience[window]
  for (const name of [...WINDOW_ORDER].reverse()) {
    if (inAudience[name]) return inAudience[name]
  }
  return null
}

/** What can be done about one widget: to the name, or to one version of it. */
export type Action = 'install' | 'update' | 'disable' | 'enable' | 'delete'

/** Modlobby's record of installing a version, by its lineage key. */
export function installedEntry(key: string) {
  return status()?.installed.find((entry) => entry.key === key) ?? null
}

/** What BAR's config says about a widget name, whoever installed it. */
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
export function isOutdated(fork: Fork): boolean {
  const entry = installedEntry(fork.key)
  if (!entry) return false
  const published = fork.install.files.map((file) => file.content_hash)
  if (published.length === 0) return false
  return (
    published.length !== entry.hashes.length ||
    published.some((hash, at) => hash !== entry.hashes[at])
  )
}

/**
 * What can be done to one version: install it, update it, remove it.
 *
 * Per version, because each is one publisher's files. Delete is offered only
 * for what modlobby installed: removing files it does not own is not its
 * business.
 */
export function forkActions(fork: Fork): Action[] {
  const entry = installedEntry(fork.key)
  const actions: Action[] = []
  if (!entry && fork.install.url) actions.push('install')
  if (entry && isOutdated(fork)) actions.push('update')
  if (entry) actions.push('delete')
  return actions
}

/**
 * Switching the widget on or off: once per name, never per version.
 *
 * BAR's config has one entry per `GetInfo().name`, so every file with that
 * name shares one switch. Offering it on each version would be several
 * buttons for one switch — which is exactly the confusion two "Disable"
 * buttons on "Dont Stand in Fire" caused.
 */
export function familyToggle(widget: WidgetUsage): Action | null {
  const configured = configuredState(widget)
  if (configured && isEnabled(configured)) return 'disable'
  // A widget BAR has never loaded has no config entry and starts switched off,
  // so something just installed has to be offered Enable without one.
  if (configured || isInstalled(widget)) return 'enable'
  return null
}

/**
 * The actions on a collapsed row: its main version's, and the name's switch.
 *
 * In reading order: get it, update it, switch it, remove it.
 */
export function actionsFor(widget: WidgetUsage): Action[] {
  const version = forkActions(mainFork(widget))
  const toggle = familyToggle(widget)
  const order: Action[] = ['install', 'update', 'disable', 'enable', 'delete']
  const all = toggle ? [...version, toggle] : version
  return order.filter((action) => all.includes(action))
}

/** Why a version cannot be downloaded, when it cannot. */
export function unavailableBecause(install: Install): string | null {
  if (install.url && install.files.length > 0) return null
  if (install.reason) return install.reason
  return install.kind === 'none' ? 'no known source' : 'no download available'
}

/**
 * A widget file on disk that answers to this row's name.
 *
 * BAR's config knows widgets by name alone, so a player's homebrewed "Dont
 * Stand in Fire" and the published one are the same entry to it. The files are
 * not: each is hashed the way the game hashes it, so a file either *is* one of
 * the published versions — and then says which — or is the player's own.
 */
export interface LocalMatch {
  file: LocalWidget
  /** The version this file is byte for byte, or null for the player's own. */
  fork: Fork | null
}

/** Every file on disk declaring this widget's name, matched ones first. */
export function localFor(widget: WidgetUsage): LocalMatch[] {
  const forks = forksOf(widget)
  return (status()?.local ?? [])
    .filter((file) => file.name === widget.name)
    .map((file) => ({
      file,
      fork:
        forks.find((fork) =>
          fork.install.files.some(
            (published) =>
              published.content_hash === file.hash ||
              published.content_hash === file.hash_text,
          ),
        ) ?? null,
    }))
    .sort((a, b) => Number(b.fork !== null) - Number(a.fork !== null))
}

/** Files on disk that are this version, byte for byte. */
export function localForFork(widget: WidgetUsage, fork: Fork): LocalWidget[] {
  return localFor(widget)
    .filter((match) => match.fork?.key === fork.key)
    .map((match) => match.file)
}

/**
 * Files on disk under this name that match no published version.
 *
 * A private homebrew, an edited copy, or a version nobody publishes: each is
 * shown as its own "Your version" line under the row, since that is where a
 * player looks for what they have.
 */
export function yourVersions(widget: WidgetUsage): LocalWidget[] {
  return localFor(widget)
    .filter((match) => match.fork === null)
    .map((match) => match.file)
}

/**
 * Whether any version of this widget is on this machine, by file or by
 * modlobby's own record.
 *
 * Not by BAR's config: it keeps entries for widgets long since deleted, so a
 * name there says the game once saw it, not that it is here now.
 */
export function isInstalled(widget: WidgetUsage): boolean {
  return (
    forksOf(widget).some((fork) => installedEntry(fork.key) !== null) ||
    localFor(widget).length > 0
  )
}

/** Where a file sits, in words a player recognises. */
export function locationOf(file: LocalWidget): string {
  return file.writable ? "modlobby's folder" : "BAR's folder"
}

/** What a header sorts by. */
export type SortKey =
  | 'rank'
  | 'name'
  | 'players'
  | 'using'
  | 'off'
  | 'replays'
  | 'sightings'
  | 'window'
  | 'source'
  | 'status'

/** Which way a header starts when first clicked: most first for numbers, A to Z for text. */
export const SORT_STARTS_DESCENDING: Record<SortKey, boolean> = {
  rank: false,
  name: false,
  players: true,
  using: true,
  off: true,
  replays: true,
  sightings: true,
  window: true,
  source: false,
  status: true,
}

/**
 * Where a widget stands on this machine, as a number to sort by.
 *
 * Installed and on, then installed and off, then installable, then the rest --
 * the order a player managing their widgets reads down.
 */
function standing(widget: WidgetUsage): number {
  const configured = configuredState(widget)
  if (isInstalled(widget)) {
    return configured && !isEnabled(configured) ? 2 : 3
  }
  return mainFork(widget).install.url ? 1 : 0
}

/**
 * Widgets in the order a header asks for, counted the way the page counts.
 *
 * Ties fall back to rank, so a column of equal values still reads as the
 * popularity list it came from rather than in document order.
 */
export function sortWidgets(
  widgets: WidgetUsage[],
  key: SortKey,
  descending: boolean,
  audience: string,
  window: string,
  mode: UsingMode = DEFAULT_USING_MODE,
): WidgetUsage[] {
  const stats = (widget: WidgetUsage) =>
    statsFor(widget, audience, window)?.stats
  const value = (widget: WidgetUsage): number | string => {
    const found = stats(widget)
    switch (key) {
      case 'rank':
        return found?.rank ?? Number.MAX_SAFE_INTEGER
      case 'name':
        return widget.name.toLowerCase()
      case 'players':
        return found?.players ?? -1
      case 'using':
        return found ? usingShare(found, mode) : -1
      case 'off':
        return found ? notUsing(found, mode) : -1
      case 'replays':
        return found?.replays ?? -1
      case 'sightings':
        return found?.sightings ?? -1
      case 'window':
        return found?.coverage ?? -1
      case 'source':
        return mainFork(widget).install.kind
      case 'status':
        return standing(widget)
    }
  }
  const rank = (widget: WidgetUsage) =>
    stats(widget)?.rank ?? Number.MAX_SAFE_INTEGER
  return [...widgets].sort((a, b) => {
    const left = value(a)
    const right = value(b)
    const order =
      typeof left === 'string' && typeof right === 'string'
        ? left.localeCompare(right)
        : (left as number) - (right as number)
    if (order !== 0) return descending ? -order : order
    return rank(a) - rank(b)
  })
}
