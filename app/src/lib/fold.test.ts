import { describe, expect, test } from 'vitest'
import { fitCount } from './fold'

const GAP = 10
const MENU = 30

describe('fitCount', () => {
  test('a row with room for everything folds nothing', () => {
    // 300 of items and two gaps: 320.
    expect(fitCount([100, 100, 100], 320, GAP, MENU)).toBe(3)
  })

  test('folds from the end, and the menu button takes its own room', () => {
    // Two items, the button and two gaps: 250 exactly.
    expect(fitCount([100, 100, 100], 250, GAP, MENU)).toBe(2)
    expect(fitCount([100, 100, 100], 249, GAP, MENU)).toBe(1)
  })

  test('hiding a short item can cost more than it frees', () => {
    // Both fit in 130 without a button; take five away and neither the pair
    // nor the first alone with the button (140) fits.
    expect(fitCount([100, 20], 130, GAP, MENU)).toBe(2)
    expect(fitCount([100, 20], 125, GAP, MENU)).toBe(0)
  })

  test('a row too narrow even for the button keeps nothing', () => {
    expect(fitCount([100, 100], 20, GAP, MENU)).toBe(0)
  })

  test('nothing to place fits anywhere', () => {
    expect(fitCount([], 0, GAP, MENU)).toBe(0)
  })

  test('an item of unknown width costs nothing, so it is kept and measured', () => {
    expect(fitCount([0, 0, 0], 20, GAP, MENU)).toBe(3)
  })
})
