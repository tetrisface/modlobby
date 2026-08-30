/**
 * How long a room's game has been going.
 *
 * Nothing on the wire says when a game started. `BATTLEOPENED` carries no time
 * (`spring_out.ex:229`), no other message adds one, and there is nothing to ask
 * for — so this is not a fetch we are missing, it is a fact the protocol does
 * not have. The one signal there is: the host's `CLIENTSTATUS` in-game bit,
 * which we can watch.
 *
 * That makes two kinds of answer. A game that starts while we are connected is
 * timed exactly. A game already running when we logged in has no knowable
 * start, only a floor — we have seen it running since we arrived — and saying
 * "18m" for a game that began an hour ago would be a lie the reader has no way
 * to catch. Those are marked, and the room bar renders the mark as a `+`.
 */

export type Running = {
  /** Unix milliseconds we first saw this game running. */
  since: number
  /** Whether that is the real start, rather than the moment we first looked. */
  exact: boolean
}

/**
 * The new record of what is running, given what was running before.
 *
 * Pure, so the awkward cases can be tested: a game seen for the first time at
 * login, a game that starts under us, one that ends, and a room that closes
 * while its game is running.
 */
export function track(
  previous: Readonly<Record<number, Running>>,
  running: ReadonlySet<number>,
  /** Whether anything has been seen at all yet — false on the first look. */
  settled: boolean,
  now: number,
): Record<number, Running> {
  const next: Record<number, Running> = {}
  for (const id of running) {
    const held = previous[id]
    // Already timed, and still the same game: keep the start we have.
    if (held) {
      next[id] = held
      continue
    }
    next[id] = { since: now, exact: settled }
  }
  return next
}

/** `7m`, `1h04`, and `+` when the start is only a floor. */
export function elapsed(running: Running, now: number): string {
  const minutes = Math.max(0, Math.floor((now - running.since) / 60_000))
  const mark = running.exact ? '' : '+'
  if (minutes < 60) return `${minutes}m${mark}`
  const hours = Math.floor(minutes / 60)
  return `${hours}h${String(minutes % 60).padStart(2, '0')}${mark}`
}
