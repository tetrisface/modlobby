import { describe, expect, test } from 'vitest'
import { move, ordered } from './reorder'

describe('dragging a tab to a new place', () => {
  const tabs = ['a', 'b', 'c', 'd']

  test('leftwards lands where it was dropped', () => {
    expect(move(tabs, 2, 0)).toEqual(['c', 'a', 'b', 'd'])
  })

  test('rightwards lands where it was dropped', () => {
    // The one that is easy to get wrong: removing 'a' shifts everything after
    // it down, so a naive implementation puts it one place short of the drop.
    expect(move(tabs, 0, 2)).toEqual(['b', 'c', 'a', 'd'])
    expect(move(tabs, 0, 3)).toEqual(['b', 'c', 'd', 'a'])
  })

  test('a drop where it already was changes nothing', () => {
    expect(move(tabs, 1, 1)).toEqual(tabs)
  })

  test('a drop that makes no sense is a no-op, not a crash', () => {
    expect(move(tabs, -1, 2)).toEqual(tabs)
    expect(move(tabs, 1, 9)).toEqual(tabs)
    expect(move([], 0, 0)).toEqual([])
  })

  test('the original is never modified', () => {
    const before = [...tabs]
    move(tabs, 0, 3)
    expect(tabs).toEqual(before)
  })
})

describe('applying a saved order to what is actually open', () => {
  test('what is open follows the saved order', () => {
    expect(ordered(['a', 'b', 'c'], ['c', 'a', 'b'])).toEqual(['c', 'a', 'b'])
  })

  test('a saved name that is no longer open is dropped', () => {
    // A channel left last week must not reappear as an empty tab.
    expect(ordered(['a', 'b'], ['gone', 'b', 'a'])).toEqual(['b', 'a'])
  })

  test('something newly open goes to the end', () => {
    // A first message from someone arrives with no saved position; appending
    // keeps every existing tab where the reader last put it.
    expect(ordered(['a', 'b', 'new'], ['b', 'a'])).toEqual(['b', 'a', 'new'])
  })

  test('no saved order at all keeps what is open as it came', () => {
    expect(ordered(['a', 'b'], [])).toEqual(['a', 'b'])
  })
})
