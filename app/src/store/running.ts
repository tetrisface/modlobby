import { createSignal } from 'solid-js'
import { type Running, told, track } from '../lib/running'

/**
 * How long each running game has been going.
 *
 * A store rather than component state because it has two sources that meet
 * here: what this window watched happen, and the one thing the server ever
 * says about it — SPADS's welcome message, which arrives as a delta from a
 * room you walked into. See `lib/running` for why those are not the same kind
 * of answer.
 */
const [running, setRunning] = createSignal<Record<number, Running>>({})

export { running }

/** What the battle list can see: whichever games are going right now. */
export function noteRunning(ids: ReadonlySet<number>, settled: boolean): void {
  setRunning((held) => track(held, ids, settled, Date.now()))
}

/** What a host told us on the way in, which beats anything we watched. */
export function noteToldStart(id: number, secondsAgo: number): void {
  setRunning((held) => told(held, id, secondsAgo, Date.now()))
}
