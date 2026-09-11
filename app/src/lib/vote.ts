import {
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  type Accessor,
} from 'solid-js'
import type { VoteView } from '../ipc/bindings/VoteView'

/**
 * What the vote bar works out for itself between the host's statements.
 *
 * SPADS says where a vote stands when it is called, on every vote cast and on
 * its reminders, and nothing in between. What is here fills those silences,
 * and is kept apart from the bar so it can be tested against a clock.
 */

/** Tells one vote from the next: the same command called again is a new vote. */
export function voteKey(vote: VoteView): string {
  return `${vote.by ?? ''}\n${vote.command}`
}

/**
 * How far a side is towards what it needs, 0 to 1. A side the host is not
 * counting -- a needed count of nought -- is nowhere.
 */
export function share(have: number, needed: number): number {
  if (needed <= 0) return 0
  return Math.min(1, have / needed)
}

/** `1/8`, or just `1` where the host names no target. */
export function tally(have: number, needed: number): string {
  return needed > 0 ? `${have}/${needed}` : `${have}`
}

/**
 * Seconds left, counted down from the last figure the host gave.
 *
 * Each statement restarts the count from what it said. A clock that only
 * moved when the host spoke would stand still for most of the vote.
 */
export function createCountdown(said: Accessor<number>): Accessor<number> {
  const [left, setLeft] = createSignal(0)
  createEffect(() => {
    const from = said()
    setLeft(from)
    if (from <= 0) return
    const timer = setInterval(
      () => setLeft((seconds) => Math.max(0, seconds - 1)),
      1000,
    )
    onCleanup(() => clearInterval(timer))
  })
  return left
}

/**
 * The longest the host has said this vote had, which is how long it was
 * given: the first statement comes with the vote, before any of it has run.
 * Forgotten with the vote, so the next one is measured against its own time.
 */
export function createSpan(
  said: Accessor<number>,
  key: Accessor<string | null>,
): Accessor<number> {
  let of: string | null = null
  return createMemo<number>((longest) => {
    const now = said()
    const which = key()
    const span = which === of ? Math.max(longest, now) : now
    of = which
    return span
  }, 0)
}
