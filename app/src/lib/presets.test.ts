import { describe, expect, test } from 'vitest'
import type { Preset } from '../ipc/bindings/Preset'
import { DEFAULT_SORT, search, sort, tweakCount, when } from './presets'

const preset = (over: Partial<Preset> = {}): Preset => ({
  name: 'a preset',
  map: 'Supreme Isthmus v2.1',
  modoptions: {},
  battle: {},
  startBoxes: {},
  bots: {},
  created: 1000,
  updated: 1000,
  lastUsed: null,
  ...over,
})

const names = (presets: Preset[]) => presets.map((p) => p.name)

describe('ordering saved setups', () => {
  test('last used is the order they open in', () => {
    expect(DEFAULT_SORT).toBe('used')
  })

  test('a preset never used sorts last, whichever way the column points', () => {
    const all = [
      preset({ name: 'never', lastUsed: null }),
      preset({ name: 'old', lastUsed: 100 }),
      preset({ name: 'recent', lastUsed: 900 }),
    ]
    // Descending: most recent first, and "never" is not the most recent.
    expect(names(sort(all, 'used', true))).toEqual(['recent', 'old', 'never'])
    // Ascending: "never" is not the earliest date either — it is no date.
    expect(names(sort(all, 'used', false))).toEqual(['never', 'old', 'recent'])
  })

  test('equal values keep a stable order rather than shuffling', () => {
    const all = [
      preset({ name: 'b', updated: 5 }),
      preset({ name: 'a', updated: 5 }),
      preset({ name: 'c', updated: 5 }),
    ]
    expect(names(sort(all, 'updated', true))).toEqual(['a', 'b', 'c'])
    expect(names(sort(all, 'updated', false))).toEqual(['a', 'b', 'c'])
  })

  test('names read as text, not as numbers', () => {
    const all = [preset({ name: 'Zulu' }), preset({ name: 'alpha' })]
    expect(names(sort(all, 'name', false))).toEqual(['alpha', 'Zulu'])
  })

  test('sorting does not disturb the list it was given', () => {
    const all = [preset({ name: 'b' }), preset({ name: 'a' })]
    sort(all, 'name', false)
    expect(names(all)).toEqual(['b', 'a'])
  })

  test('size is how many settings a preset carries', () => {
    const all = [
      preset({ name: 'small', modoptions: { a: '1' } }),
      preset({ name: 'big', modoptions: { a: '1', b: '2', c: '3' } }),
    ]
    expect(names(sort(all, 'options', true))).toEqual(['big', 'small'])
  })
})

describe('finding one', () => {
  const all = [
    preset({ name: 'raptor hell', map: 'Comet Catcher Remake 1.8' }),
    preset({ name: 'scav party', map: 'Supreme Isthmus v2.1' }),
  ]

  test('the name and the map are both searched', () => {
    expect(names(search(all, 'raptor'))).toEqual(['raptor hell'])
    expect(names(search(all, 'isthmus'))).toEqual(['scav party'])
  })

  test('every word has to match, in any order', () => {
    expect(names(search(all, 'comet raptor'))).toEqual(['raptor hell'])
    expect(search(all, 'raptor isthmus')).toEqual([])
  })

  test('nothing typed is everything', () => {
    expect(search(all, '   ')).toHaveLength(2)
  })
})

describe('describing one', () => {
  test('only a filled tweak slot counts as one', () => {
    const held = preset({
      modoptions: {
        tweakdefs1: 'LS1OdXR0eUI',
        tweakunits3: 'LS1TcGhlcmU',
        // Cleared slots are written as a single character, not removed.
        tweakdefs2: '0',
        raptor_endless: '1',
      },
    })
    expect(tweakCount(held)).toBe(2)
  })

  test('dates read as ages until they are old enough to be dates', () => {
    const now = 10_000_000_000
    const secondsNow = Math.floor(now / 1000)
    expect(when(null, now)).toBe('never')
    expect(when(secondsNow - 10, now)).toBe('just now')
    expect(when(secondsNow - 60 * 30, now)).toBe('30m ago')
    expect(when(secondsNow - 3600 * 5, now)).toBe('5h ago')
    expect(when(secondsNow - 86400 * 3, now)).toBe('3d ago')
    // Beyond a fortnight the age stops meaning anything; the date does.
    expect(when(secondsNow - 86400 * 400, now)).toMatch(/^\d{4}-\d{2}-\d{2}$/)
  })

  test('a clock that disagrees does not produce a negative age', () => {
    const now = 1_000_000
    expect(when(Math.floor(now / 1000) + 500, now)).toBe('just now')
  })
})
