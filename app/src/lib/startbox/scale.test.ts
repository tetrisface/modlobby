import { describe, expect, test } from 'vitest'
import { SIZE } from './geometry'
import {
  GRAB_PIXELS,
  HANDLE_STROKE,
  LABEL_MAX,
  LABEL_MIN,
  handleSize,
  labelSize,
} from './scale'

/** A box in map units, given as the rectangle it fits in. */
const box = (width: number, height: number) => ({
  left: 0,
  top: 0,
  right: width,
  bottom: height,
})

const whole = box(SIZE, SIZE)
/** The narrowest box the editor will make, `MIN_SIDE` on a side. */
const sliver = box(5, 5)

/** The owner's screenshot: the sheet at its widest, on a squarish map. */
const shot = { width: 875, height: 660 }
const strip = box(40, SIZE)
const half = box(91, 151)

describe('label size', () => {
  test('a bigger map view carries a bigger number', () => {
    const sizes = [
      { width: 250, height: 250 },
      shot,
      { width: 1200, height: 1200 },
    ].map((view) => labelSize(view, half))
    expect(sizes[0]).toBeLessThan(sizes[1]!)
    expect(sizes[1]).toBeLessThan(sizes[2]!)
  })

  test('the number grows far slower than the view it sits in', () => {
    const small = labelSize({ width: 180, height: 180 }, half)
    const large = labelSize({ width: 1260, height: 1260 }, half)
    // Seven times the view for barely twice the type: the point of the
    // exponent, and loose bounds so the curve can be retuned without this
    // test being rewritten.
    expect(large / small).toBeGreaterThan(1.5)
    expect(large / small).toBeLessThan(2.5)
  })

  test('a bigger box carries a bigger number than the one beside it', () => {
    expect(labelSize(shot, half)).toBeGreaterThan(labelSize(shot, strip))
  })

  test('the box only nudges the number, it never sets it', () => {
    const sizes = [box(5, SIZE), strip, half, whole].map((shape) =>
      labelSize(shot, shape),
    )
    // Every ordinary box in one type size: a whole-map box is a third larger
    // than a five-unit strip, where the range of views is twice that.
    expect(Math.max(...sizes) / Math.min(...sizes)).toBeLessThan(1.4)
    // Monotone, so a box never shrinks its own number by growing.
    expect([...sizes].sort((a, b) => a - b)).toEqual(sizes)
  })

  test('the smallest box the editor can make is still not on the floor', () => {
    // Five units square, and the box term is what shrinks it -- not a clamp.
    expect(labelSize(shot, sliver)).toBeGreaterThan(LABEL_MIN)
    expect(labelSize(shot, sliver)).toBeLessThan(labelSize(shot, strip))
  })

  test('a sliver still gets a readable number', () => {
    const size = labelSize(shot, box(0.5, 0.5))
    expect(size).toBe(LABEL_MIN)
    expect(Number.isFinite(size)).toBe(true)
  })

  test('the number is never smaller than the fixed one it replaces', () => {
    for (const view of [
      { width: 250, height: 250 },
      { width: 875, height: 44 },
      shot,
      { width: 1260, height: 1260 },
    ])
      for (const shape of [sliver, strip, half, whole]) {
        expect(labelSize(view, shape)).toBeGreaterThanOrEqual(LABEL_MIN)
        expect(labelSize(view, shape)).toBeLessThanOrEqual(LABEL_MAX)
      }
  })

  test('a map that is not square gets the same number either way round', () => {
    expect(labelSize({ width: 400, height: 200 }, box(100, 50))).toBe(
      labelSize({ width: 200, height: 400 }, box(50, 100)),
    )
  })

  test('a view nobody has measured yet still gets a size', () => {
    expect(labelSize({ width: 0, height: 0 }, whole)).toBe(LABEL_MIN)
  })
})

describe('handle size', () => {
  test('a handle is never drawn bigger than what grabs it', () => {
    for (const view of [
      { width: 200, height: 200 },
      shot,
      { width: 1800, height: 1800 },
    ])
      for (const shape of [sliver, strip, half, whole])
        expect(handleSize(view, shape) + HANDLE_STROKE / 2).toBeLessThanOrEqual(
          GRAB_PIXELS,
        )
  })

  test('a handle keeps its proportion to the number beside it', () => {
    expect(handleSize(shot, half)).toBe(
      Math.round(labelSize(shot, half) * 25) / 100,
    )
  })

  test('a handle is never smaller than the one it replaces', () => {
    for (const view of [{ width: 200, height: 200 }, shot])
      for (const shape of [sliver, half, whole])
        expect(handleSize(view, shape)).toBeGreaterThanOrEqual(4)
  })
})
