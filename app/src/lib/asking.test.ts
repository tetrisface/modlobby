import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { asker } from './asking'

function deferred() {
  let resolve!: () => void
  const promise = new Promise<void>((yes) => {
    resolve = yes
  })
  return { promise, resolve }
}

async function settle() {
  for (let turn = 0; turn < 6; turn++) await Promise.resolve()
}

describe('asker', () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] })
  })
  afterEach(() => {
    vi.useRealTimers()
  })

  test('the first ask waits its stagger, then goes once', () => {
    const run = vi.fn(async () => {})
    const asks = asker(run, { floor: 2000, stagger: () => 400 })
    asks.ask()
    asks.ask()
    vi.advanceTimersByTime(399)
    expect(run).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(run).toHaveBeenCalledTimes(1)
  })

  test('asks while one is out become one follow-up, after it', async () => {
    const slow = deferred()
    const run = vi
      .fn()
      .mockReturnValueOnce(slow.promise)
      .mockResolvedValue(undefined)
    const asks = asker(run, { floor: 0, stagger: () => 0 })
    asks.ask()
    vi.advanceTimersByTime(0)
    expect(run).toHaveBeenCalledTimes(1)

    asks.ask()
    asks.ask()
    asks.ask()
    vi.advanceTimersByTime(10_000)
    expect(run).toHaveBeenCalledTimes(1)

    slow.resolve()
    await settle()
    vi.advanceTimersByTime(0)
    expect(run).toHaveBeenCalledTimes(2)
  })

  test('two asks start no closer than the floor', async () => {
    const run = vi.fn(async () => {})
    const asks = asker(run, { floor: 2000, stagger: () => 0 })
    asks.ask()
    vi.advanceTimersByTime(0)
    await settle()
    expect(run).toHaveBeenCalledTimes(1)

    vi.advanceTimersByTime(500)
    asks.ask()
    vi.advanceTimersByTime(1499)
    expect(run).toHaveBeenCalledTimes(1)
    vi.advanceTimersByTime(1)
    expect(run).toHaveBeenCalledTimes(2)
  })

  test('the stagger is read when the ask is scheduled, not when made', () => {
    let delay = 100
    const run = vi.fn(async () => {})
    const asks = asker(run, { floor: 0, stagger: () => delay })
    delay = 700
    asks.ask()
    vi.advanceTimersByTime(699)
    expect(run).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1)
    expect(run).toHaveBeenCalledTimes(1)
  })

  test('nothing goes after stop, not even a scheduled or remembered ask', async () => {
    const slow = deferred()
    const run = vi
      .fn()
      .mockReturnValueOnce(slow.promise)
      .mockResolvedValue(undefined)
    const asks = asker(run, { floor: 0, stagger: () => 0 })
    asks.ask()
    vi.advanceTimersByTime(0)
    asks.ask()
    asks.stop()
    slow.resolve()
    await settle()
    vi.advanceTimersByTime(10_000)
    expect(run).toHaveBeenCalledTimes(1)

    asks.ask()
    vi.advanceTimersByTime(10_000)
    expect(run).toHaveBeenCalledTimes(1)
  })

  test('a run that rejects does not stop the next ask', async () => {
    const run = vi
      .fn()
      .mockRejectedValueOnce(new Error('no'))
      .mockResolvedValue(undefined)
    const asks = asker(run, { floor: 0, stagger: () => 0 })
    asks.ask()
    vi.advanceTimersByTime(0)
    await settle()
    asks.ask()
    vi.advanceTimersByTime(0)
    expect(run).toHaveBeenCalledTimes(2)
  })
})
