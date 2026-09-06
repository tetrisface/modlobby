import { createSignal } from 'solid-js'
import type { NewsFeed } from '../ipc/bindings/NewsFeed'
import { api } from '../ipc/client'
import { devicePixels, thumbSrc } from '../lib/thumb'

/**
 * BAR's news, and how much of it nobody here has seen yet.
 *
 * The count belongs to the nav, which is drawn whether or not the News tab is
 * open, so the feed is asked for once when the shell mounts rather than by the
 * page. That costs at most one request per launch: Rust answers from its own
 * cache for the hour the feed asks to be trusted for, so most launches make
 * none. Nothing is scheduled and nothing polls.
 *
 * Failures are silent. Rust already answers with a stale feed when it cannot
 * reach the server and an empty one when it has never managed to, so there is
 * nothing here to report that the page does not already show.
 */
export const [news, setNews] = createSignal<NewsFeed | null>(null)

/** The banner box in `.news-card`, in CSS pixels. Mirrors the stylesheet. */
const BANNER = { width: 320, height: 180 }

let pending: Promise<NewsFeed> | null = null

/** One request, however many callers arrive together. */
export async function loadNews(): Promise<void> {
  try {
    pending ??= api.news()
    setNews(await pending)
  } catch {
    // Rust could not be reached at all; the next caller asks again.
    pending = null
  }
}

/** How many stories have turned up since the tab was last opened. */
export function unreadNews(): number {
  return news()?.unread ?? 0
}

/**
 * Remembers what the list showed, which is what clears the count.
 *
 * Waits for the feed first, so opening the tab before it has arrived does not
 * mark an empty list as read and leave the stories in it unread forever.
 */
export async function markNewsRead(): Promise<void> {
  await loadNews()
  if (unreadNews() === 0) return
  try {
    await api.markNewsRead()
    // The held promise still carries the count from before the mark, and
    // replaying it would put the badge back. Dropped, so the next caller asks
    // Rust — which now answers with the marks that were just written.
    pending = null
    setNews((feed) => (feed ? { ...feed, unread: 0 } : feed))
  } catch {
    // The mark is bookkeeping; failing to write it costs one badge.
  }
}

/**
 * The banner a story published, at the size the card draws it. Rust fetches
 * and resizes it; a story with no picture answers 404, which reaches the
 * `<img>` as an `error` event.
 */
export function newsBanner(id: string): string | null {
  if (!id) return null
  const tile = devicePixels(BANNER)
  return thumbSrc(`news/${tile.width}x${tile.height}/${id}`)
}
