import { createSignal } from 'solid-js'
import { api } from '../ipc/client'
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
  // A game that ended takes its answer with it: the next one in that room is a
  // different game, and worth asking about again.
  for (const id of answered) if (!ids.has(id)) answered.delete(id)
  setRunning((held) => track(held, ids, settled, Date.now()))
}

/** What a host told us, which beats anything we watched. */
export function noteToldStart(id: number, secondsAgo: number): void {
  answered.add(id)
  setRunning((held) => told(held, id, secondsAgo, Date.now()))
}

/**
 * Asking a host how long its game has been going.
 *
 * This is a private message to a real account, so it is rationed the way
 * Chobby rations it (`gui_tooltip.lua:250-266`): at most one every 400ms, and
 * only for the room the pointer is on when that moment arrives, so running the
 * cursor down the list sends one message rather than thirty. Once a host has
 * answered we never ask again for that game — the clock runs on its own from
 * the start it gave us.
 */
const answered = new Set<number>()
let wanted: { id: number; founder: string } | null = null
let nextAsk = 0
let timer: ReturnType<typeof setTimeout> | undefined

export function askAboutGame(id: number, founder: string): void {
  if (answered.has(id)) return
  wanted = { id, founder }
  if (timer) return
  const wait = Math.max(0, nextAsk - Date.now())
  timer = setTimeout(() => {
    timer = undefined
    const ask = wanted
    wanted = null
    if (!ask || answered.has(ask.id)) return
    nextAsk = Date.now() + ASK_EVERY
    void api.requestGameStatus(ask.founder).catch(() => {
      // A host that will not answer is not worth a notice; the estimate stands.
    })
  }, wait)
}

const ASK_EVERY = 400
