import { describe, expect, test } from 'vitest'
import { fitsBeside } from './Watchers'

/** A card is 268 px and the gap 28 px, as the stylesheet has them at 16 px. */
const CARD = 268
const GAP = 28

describe('fitsBeside', () => {
  test('a stack fits when the teams and one more card share the row', () => {
    // Two teams and the stack: 3 × 268 + 2 × 28 = 860.
    expect(fitsBeside(2, false, CARD, GAP, 860)).toBe(true)
    expect(fitsBeside(2, false, CARD, GAP, 859)).toBe(false)
    // Six teams need 7 × 268 + 6 × 28 = 2044.
    expect(fitsBeside(6, false, CARD, GAP, 2044)).toBe(true)
    expect(fitsBeside(6, false, CARD, GAP, 1140)).toBe(false)
  })

  test('nothing fits beside a team that spans the row', () => {
    expect(fitsBeside(2, true, CARD, GAP, 5000)).toBe(false)
  })

  test('a row with no width yet, or no card to measure, fits nothing', () => {
    expect(fitsBeside(2, false, CARD, GAP, 0)).toBe(false)
    expect(fitsBeside(2, false, 0, GAP, 860)).toBe(false)
  })
})
