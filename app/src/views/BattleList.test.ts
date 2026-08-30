import { describe, expect, test } from 'vitest'
import { cardLeft } from './BattleList'

/** What `.occupants` is set to in the stylesheet. */
const WIDTH = 300
const GAP = 20

describe('where the occupants card opens', () => {
  test('to the right of the pointer, with Chobby\u2019s offset', () => {
    expect(cardLeft(100, 1920)).toBe(120)
  })

  test('to the left once opening right would run off the window', () => {
    // 1600 + 20 + 300 = 1920, which is past the window less its margin.
    expect(cardLeft(1600, 1920)).toBe(1600 - GAP - WIDTH)
  })

  test('the flip happens before the card is clipped, not after', () => {
    const viewport = 1920
    const last = viewport - GAP - WIDTH - GAP
    expect(cardLeft(last, viewport)).toBe(last + GAP)
    expect(cardLeft(last + 1, viewport)).toBeLessThan(last)
  })

  test('a window too narrow for either side still puts it on screen', () => {
    // Nothing fits; the card must not end up at a negative offset, off the
    // left edge, where it would be unreadable rather than merely awkward.
    expect(cardLeft(10, 320)).toBeGreaterThanOrEqual(0)
    expect(cardLeft(300, 320)).toBeGreaterThanOrEqual(0)
  })
})
