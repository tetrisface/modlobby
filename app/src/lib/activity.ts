/**
 * Telling the runtime that someone is here.
 *
 * The runtime drops a connection nobody has touched for a while, and it
 * counts from the last report it got. A report per keystroke would be
 * thousands of IPC calls for nothing, so reports are rate-limited: the first
 * goes out at once, and the rest wait until `every` milliseconds have passed
 * since the last one. The runtime only needs to know activity happened
 * within its limit, which is minutes, not that it happened this instant.
 */

/** How long a report covers before another is worth sending. */
export const REPORT_EVERY = 30_000

/** The window events that mean a person, not a message, is in the window. */
export const ACTIVITY_EVENTS: readonly (keyof WindowEventMap)[] = [
  'keydown',
  'pointerdown',
  'pointermove',
  'wheel',
  'focus',
]

/**
 * A handler that calls `report` for the first event and then at most once
 * per `every` milliseconds. `now` is a clock, replaceable for tests.
 */
export function activityReporter(
  report: () => void,
  every = REPORT_EVERY,
  now: () => number = Date.now,
): () => void {
  let last = Number.NEGATIVE_INFINITY
  return () => {
    const at = now()
    if (at - last < every) return
    last = at
    report()
  }
}
