import { describe, expect, it } from 'vitest'
import { boxSignature, centre, isBoxKey, outline } from './boxes'

describe('recognising a start-box modoption', () => {
  it('takes both keys, bare or under their scripttag prefix', () => {
    expect(isBoxKey('mapmetadata_startbox_override')).toBe(true)
    expect(isBoxKey('game/modoptions/mapmetadata_startboxes_set')).toBe(true)
  })

  it('leaves other modoptions alone', () => {
    expect(isBoxKey('game/modoptions/tweakdefs1')).toBe(false)
    expect(isBoxKey('startbox')).toBe(false)
  })
})

describe('the signature that says the boxes moved', () => {
  it('changes when either key changes', () => {
    const before = boxSignature({
      'game/modoptions/mapmetadata_startbox_override': 'aaa',
    })
    const after = boxSignature({
      'game/modoptions/mapmetadata_startbox_override': 'bbb',
    })
    expect(after).not.toBe(before)
  })

  it('is the same for a room with nothing set and no room at all', () => {
    expect(boxSignature({})).toBe(boxSignature(undefined))
  })

  it('does not confuse one key being set with the other', () => {
    const override = boxSignature({
      'game/modoptions/mapmetadata_startbox_override': 'x',
    })
    const set = boxSignature({
      'game/modoptions/mapmetadata_startboxes_set': 'x',
    })
    expect(override).not.toBe(set)
  })
})

describe('drawing a box', () => {
  const square: [number, number][] = [
    [0, 0],
    [100, 0],
    [100, 100],
    [0, 100],
  ]

  it('moves to the first corner, lines to the rest, and closes', () => {
    expect(outline(square)).toBe('M0 0 L100 0 L100 100 L0 100 Z')
  })

  it('puts the label in the middle', () => {
    expect(centre(square)).toEqual({ x: 50, y: 50 })
  })

  it('has an answer for a polygon with no corners rather than NaN', () => {
    expect(centre([])).toEqual({ x: 100, y: 100 })
  })
})
