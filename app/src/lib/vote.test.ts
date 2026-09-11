import { createRoot, createSignal, type Accessor } from 'solid-js'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { VoteView } from '../ipc/bindings/VoteView'
import { createCountdown, createSpan, share, tally, voteKey } from './vote'

const vote = (over: Partial<VoteView> = {}): VoteView => ({
  command: 'start',
  by: 'tetrisface',
  proposal: { type: 'other' },
  yes: 1,
  yesNeeded: 5,
  no: 0,
  noNeeded: 3,
  remainingSecs: 25,
  ...over,
})

describe('share', () => {
  test('is the fraction of what is needed, capped at whole', () => {
    expect(share(1, 4)).toBe(0.25)
    expect(share(6, 4)).toBe(1)
  })

  test('is nowhere when the host counts nothing', () => {
    expect(share(1, 0)).toBe(0)
  })
})

describe('tally', () => {
  test('reads have over needed, or just have', () => {
    expect(tally(1, 8)).toBe('1/8')
    expect(tally(1, 0)).toBe('1')
  })
})

describe('voteKey', () => {
  test('tells the same command called twice apart by who called it', () => {
    expect(voteKey(vote())).not.toBe(voteKey(vote({ by: 'someone' })))
    expect(voteKey(vote())).toBe(voteKey(vote({ yes: 3, remainingSecs: 4 })))
  })
})

describe('createCountdown', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  test('counts down by the second from what was said', () => {
    const [said, setSaid] = createSignal(3)
    let left!: Accessor<number>
    // The count is an effect, which a root runs once its body has returned.
    const dispose = createRoot((dispose) => {
      left = createCountdown(said)
      return dispose
    })
    expect(left()).toBe(3)
    vi.advanceTimersByTime(1000)
    expect(left()).toBe(2)
    vi.advanceTimersByTime(5000)
    expect(left()).toBe(0)

    // The host speaks again: the count starts over from its figure.
    setSaid(10)
    expect(left()).toBe(10)
    vi.advanceTimersByTime(1000)
    expect(left()).toBe(9)
    dispose()
  })
})

describe('createSpan', () => {
  test('remembers the longest a vote was given, until the vote changes', () => {
    createRoot((dispose) => {
      const [said, setSaid] = createSignal(25)
      const [key, setKey] = createSignal<string | null>('a')
      const span = createSpan(said, key)
      expect(span()).toBe(25)
      setSaid(17)
      expect(span()).toBe(25)

      setKey('b')
      setSaid(40)
      expect(span()).toBe(40)
      setSaid(30)
      expect(span()).toBe(40)
      dispose()
    })
  })
})
