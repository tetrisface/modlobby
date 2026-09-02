/**
 * One question at a time, not too often, and not all at once.
 *
 * Written for the PvE score panel, whose service is a Lambda with a
 * concurrency of one and a twenty-second cold start: asking twice while an
 * answer is out gets two cold starts, every client in a room asking at the
 * same instant gets one answer and a row of refusals, and a lobby that asks
 * on every settings change goes through somebody else's budget for nothing.
 * Nothing in here knows what is being asked.
 */

export type Pace = {
  /** The least time between the starts of two asks. */
  floor: number
  /** How long to wait before an ask, read when the ask is scheduled. */
  stagger: () => number
}

export type Asker = {
  /** Asks, or arranges to: never more than one out, never sooner than the pace allows. */
  ask(): void
  /** Nothing further happens after this. */
  stop(): void
}

/**
 * `run` does the asking and owns its own failures; a rejection is swallowed
 * here so a timer never surfaces it.
 *
 * An ask that arrives while one is out is remembered, once: the follow-up
 * reads whatever is current when it goes, so one covers any number of
 * changes. An ask that arrives while one is scheduled is that one.
 */
export function asker(run: () => Promise<void>, pace: Pace): Asker {
  let inFlight = false
  let again = false
  let stopped = false
  let lastStart = Number.NEGATIVE_INFINITY
  let timer: ReturnType<typeof setTimeout> | undefined

  async function go() {
    timer = undefined
    inFlight = true
    lastStart = Date.now()
    try {
      await run()
    } catch {
      // run reports its own failures
    } finally {
      inFlight = false
    }
    if (stopped || !again) return
    again = false
    ask()
  }

  function ask() {
    if (stopped) return
    if (inFlight) {
      again = true
      return
    }
    if (timer !== undefined) return
    const wait = Math.max(pace.stagger(), lastStart + pace.floor - Date.now())
    timer = setTimeout(() => void go(), wait)
  }

  return {
    ask,
    stop() {
      stopped = true
      clearTimeout(timer)
      timer = undefined
    },
  }
}
