import { describe, expect, test } from 'vitest'
import { elapsed, track, type Running } from './running'

const at = (minute: number) => minute * 60_000

describe('timing a game nobody told us the start of', () => {
  test('the first look can only give a floor', () => {
    // Logging in: these were already running, and how long for is unknowable.
    const seen = track({}, new Set([1, 2]), false, at(0))
    expect(seen[1]).toEqual({ since: at(0), exact: false })
    expect(seen[2]?.exact).toBe(false)
  })

  test('a game that starts under us is timed exactly', () => {
    const first = track({}, new Set([1]), false, at(0))
    const later = track(first, new Set([1, 2]), true, at(5))
    expect(later[1]).toEqual({ since: at(0), exact: false })
    expect(later[2]).toEqual({ since: at(5), exact: true })
  })

  test('a start already known is never restarted', () => {
    let held = track({}, new Set([1]), true, at(0))
    for (const minute of [1, 2, 3])
      held = track(held, new Set([1]), true, at(minute))
    expect(held[1]?.since).toBe(at(0))
  })

  test('a game that ends is forgotten, and a new one is timed afresh', () => {
    const first = track({}, new Set([1]), true, at(0))
    const ended = track(first, new Set(), true, at(9))
    expect(ended[1]).toBeUndefined()
    const again = track(ended, new Set([1]), true, at(20))
    expect(again[1]).toEqual({ since: at(20), exact: true })
  })

  test('a room that closes while running takes its timing with it', () => {
    const first = track({}, new Set([1, 2]), true, at(0))
    expect(track(first, new Set([2]), true, at(3))).toEqual({
      2: { since: at(0), exact: true },
    })
  })
})

describe('saying how long', () => {
  const exact = (since: number): Running => ({ since, exact: true })
  const floor = (since: number): Running => ({ since, exact: false })

  test('minutes, then hours and minutes', () => {
    expect(elapsed(exact(at(0)), at(7))).toBe('7m')
    expect(elapsed(exact(at(0)), at(59))).toBe('59m')
    expect(elapsed(exact(at(0)), at(64))).toBe('1h04')
    expect(elapsed(exact(at(0)), at(130))).toBe('2h10')
  })

  test('a floor is marked, so nobody reads it as the real thing', () => {
    expect(elapsed(floor(at(0)), at(7))).toBe('7m+')
    expect(elapsed(floor(at(0)), at(64))).toBe('1h04+')
  })

  test('a clock that went backwards says nothing silly', () => {
    expect(elapsed(exact(at(10)), at(0))).toBe('0m')
  })
})
