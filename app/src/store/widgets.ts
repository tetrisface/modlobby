import { createSignal } from 'solid-js'
import { api } from '../ipc/client'
import type { Usage } from '../ipc/bindings/Usage'
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

export { usage, loaded }

export async function loadWidgetUsage(): Promise<void> {
  if (loaded()) return
  setUsage(await api.widgetUsage())
  setLoaded(true)
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

/** The windows the document actually carries. */
export function windows(): string[] {
  return usage()?.windows ?? []
}

/**
 * Widgets ranked for one window, most used first.
 *
 * A widget missing from a window is absent, not zero: the k-anonymity floor is
 * applied inside each window, so a widget with six players this year and two
 * this week is withheld from the week rather than shown with a two.
 */
export function ranked(window: string): WidgetUsage[] {
  const document = usage()
  if (!document) return []
  const withRank = document.widgets.flatMap((widget) => {
    const stats = widget.windows[window]
    return stats ? [{ widget, rank: stats.rank }] : []
  })
  return withRank.sort((a, b) => a.rank - b.rank).map((entry) => entry.widget)
}

/** The stats to show for a widget, falling back when the window withheld it. */
export function statsFor(
  widget: WidgetUsage,
  window: string,
): { window: string; stats: WindowStats } | null {
  const wanted = widget.windows[window]
  if (wanted) return { window, stats: wanted }
  for (const name of [...WINDOW_ORDER].reverse()) {
    const stats = widget.windows[name]
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
