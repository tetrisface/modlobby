import { describe, expect, it } from 'vitest'
import { point, rect } from './geometry'
import { SEGMENTS, ring, tessellateRing } from './spline'

const diamond = [
  point(100, 20),
  point(180, 100),
  point(100, 180),
  point(20, 100),
]

describe('tessellating a ring', () => {
  it('gives back a plain polygon unchanged', () => {
    expect(tessellateRing(diamond)).toEqual(diamond)
  })

  it('samples every curved edge twelve times, anchors included', () => {
    const curved = diamond.map((p) => point(p.x, p.y, 1))
    const out = tessellateRing(curved)
    expect(out).toHaveLength(4 * SEGMENTS)
    // The anchors are on the curve, at the start of each run of samples.
    for (let i = 0; i < 4; i++) {
      expect(out[i * SEGMENTS]).toEqual(point(diamond[i]!.x, diamond[i]!.y))
    }
  })

  it('curves only the edges whose ends ask for it', () => {
    const one = [
      point(100, 20, 1),
      point(180, 100),
      point(100, 180),
      point(20, 100),
    ]
    // Edges 0 and 3 touch the tense anchor; edges 1 and 2 stay straight.
    expect(tessellateRing(one)).toHaveLength(2 * SEGMENTS + 2)
  })

  it('bows a curved edge outward on a convex ring', () => {
    const curved = diamond.map((p) => point(p.x, p.y, 1))
    const out = tessellateRing(curved)
    // The middle sample of the first edge (100,20)-(180,100) lies beyond the
    // straight chord: further from the centre (100,100) than the chord's midpoint.
    const mid = out[SEGMENTS / 2]!
    const chord = point(140, 60)
    const fromCentre = (p: { x: number; y: number }) =>
      Math.hypot(p.x - 100, p.y - 100)
    expect(fromCentre(mid)).toBeGreaterThan(fromCentre(chord))
  })

  it('blends toward the straight line for a partial strength', () => {
    const full = tessellateRing(diamond.map((p) => point(p.x, p.y, 1)))[
      SEGMENTS / 2
    ]!
    const half = tessellateRing(diamond.map((p) => point(p.x, p.y, 0.5)))[
      SEGMENTS / 2
    ]!
    const chord = point(140, 60)
    expect(half.x).toBeCloseTo((full.x + chord.x) / 2, 6)
    expect(half.y).toBeCloseTo((full.y + chord.y) / 2, 6)
  })

  it('treats a missing strength as a sharp corner and a two-point ring as a line', () => {
    expect(tessellateRing([point(0, 0, 1), point(50, 50, 1)])).toEqual([
      point(0, 0),
      point(50, 50),
    ])
  })
})

describe('the outline of a box', () => {
  it('expands a rectangle to its four corners', () => {
    expect(ring(rect(point(10, 10), point(30, 40)))).toEqual([
      [10, 10],
      [30, 10],
      [30, 40],
      [10, 40],
    ])
  })
})
