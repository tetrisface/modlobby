import { describe, expect, it } from 'vitest'
import {
  MIN_SIDE,
  SIZE,
  bounds,
  centreBox,
  corners,
  expandRect,
  hitTest,
  insertVertex,
  lassoBox,
  moveEdge,
  moveVertex,
  point,
  rect,
  rectFrom,
  removeVertex,
  round,
  setStrength,
  simplify,
  translateClamped,
} from './geometry'

const square = rect(point(20, 20), point(60, 60))
const triangle = {
  poly: [point(10, 10), point(50, 10), point(30, 40)],
}

describe('a rectangle', () => {
  it('is stored min corner first whichever way it was drawn', () => {
    expect(rect(point(60, 60), point(20, 20))).toEqual(square)
  })

  it('draws as four corners going round from the first, as the game expands it', () => {
    expect(corners(square)).toEqual([
      point(20, 20),
      point(60, 20),
      point(60, 60),
      point(20, 60),
    ])
  })

  it('too small to draw is not drawn', () => {
    expect(rectFrom(point(10, 10), point(13, 40))).toBeNull()
    expect(rectFrom(point(10, 10), point(40, 13))).toBeNull()
    expect(rectFrom(point(10, 10), point(40, 40))).toEqual(
      rect(point(10, 10), point(40, 40)),
    )
  })

  it('drawn off the map is cut at the border', () => {
    expect(rectFrom(point(-30, 150), point(50, 260))).toEqual(
      rect(point(0, 150), point(50, 200)),
    )
  })

  it('in the centre is what Chobby adds', () => {
    expect(bounds(centreBox())).toEqual({
      left: 66,
      top: 66,
      right: 133,
      bottom: 133,
    })
  })
})

describe('dragging a whole box', () => {
  it('moves it', () => {
    expect(bounds(translateClamped(square, 10, -5))).toEqual({
      left: 30,
      top: 15,
      right: 70,
      bottom: 55,
    })
  })

  it('stops at the border and keeps its shape', () => {
    const pushed = translateClamped(square, 500, -500)
    expect(bounds(pushed)).toEqual({
      left: 160,
      top: 0,
      right: 200,
      bottom: 40,
    })
  })

  it('slides along a border it is already against', () => {
    const against = translateClamped(square, 500, 0)
    const slid = translateClamped(against, 500, 30)
    expect(bounds(slid)).toEqual({ left: 160, top: 50, right: 200, bottom: 90 })
  })

  it('keeps a polygon rigid, curvature included', () => {
    const curved = { poly: [point(0, 0, 0.5), point(40, 0), point(20, 30)] }
    const moved = translateClamped(curved, -100, 10)
    expect(moved.poly).toEqual([
      point(0, 10, 0.5),
      point(40, 10),
      point(20, 40),
    ])
  })
})

describe('dragging a vertex', () => {
  it('of a polygon goes where the pointer is, but not off the map', () => {
    const moved = moveVertex(triangle, 2, point(30, 999))
    expect(moved.poly[2]).toEqual(point(30, SIZE))
    expect(moved.poly.slice(0, 2)).toEqual(triangle.poly.slice(0, 2))
  })

  it('keeps its curvature', () => {
    const curved = { poly: [point(10, 10, 0.7), point(50, 10), point(30, 40)] }
    expect(moveVertex(curved, 0, point(5, 5)).poly[0]).toEqual(point(5, 5, 0.7))
  })

  it('of a rectangle resizes it around the opposite corner', () => {
    // Corner 2 is (60,60); its opposite is (20,20).
    expect(moveVertex(square, 2, point(100, 90))).toEqual(
      rect(point(20, 20), point(100, 90)),
    )
    // Corner 0 is (20,20); its opposite is (60,60).
    expect(moveVertex(square, 0, point(0, 0))).toEqual(
      rect(point(0, 0), point(60, 60)),
    )
  })

  it('cannot collapse a rectangle', () => {
    const squeezed = moveVertex(square, 2, point(21, 21))
    expect(bounds(squeezed)).toEqual({
      left: 20,
      top: 20,
      right: 20 + MIN_SIDE,
      bottom: 20 + MIN_SIDE,
    })
  })

  it('pushes a corner the other way when the map ends first', () => {
    const nearEdge = rect(point(150, 150), point(198, 198))
    // Corner 0 dragged onto its opposite (198,198): pushing it on past would
    // leave the map, so it lands the minimum side on the near side instead.
    const squeezed = moveVertex(nearEdge, 0, point(198, 198))
    expect(bounds(squeezed)).toEqual({
      left: 193,
      top: 193,
      right: 198,
      bottom: 198,
    })
  })
})

describe('dragging an edge', () => {
  it('of a rectangle moves that side along its normal only', () => {
    // Edge 1 is the right side.
    expect(moveEdge(square, 1, 25, 999)).toEqual(
      rect(point(20, 20), point(85, 60)),
    )
    // Edge 0 is the top.
    expect(moveEdge(square, 0, 999, -10)).toEqual(
      rect(point(20, 10), point(60, 60)),
    )
  })

  it('of a rectangle stops at the map and short of the other side', () => {
    expect(moveEdge(square, 3, -500, 0)).toEqual(
      rect(point(0, 20), point(60, 60)),
    )
    expect(moveEdge(square, 3, 500, 0)).toEqual(
      rect(point(60 - MIN_SIDE, 20), point(60, 60)),
    )
    expect(moveEdge(square, 2, 0, 500)).toEqual(
      rect(point(20, 20), point(60, 200)),
    )
  })

  it('of a polygon carries both ends, no further than the map allows', () => {
    // Edge 0 runs (10,10)-(50,10); pushing it up 30 is stopped after 10.
    const moved = moveEdge(triangle, 0, 5, -30)
    expect(moved.poly).toEqual([point(15, 0), point(55, 0), point(30, 40)])
  })
})

describe('reshaping a polygon', () => {
  it('a rectangle becomes four corners before it grows a vertex', () => {
    expect(expandRect(square).poly).toHaveLength(4)
    const grown = insertVertex(square, 1, point(70, 40))
    expect(grown.poly).toEqual([
      point(20, 20),
      point(60, 20),
      point(70, 40),
      point(60, 60),
      point(20, 60),
    ])
  })

  it('a polygon keeps at least three vertices', () => {
    expect(removeVertex(triangle, 0)).toBeNull()
    const quad = insertVertex(triangle, 0, point(30, 5))
    expect(removeVertex(quad, 1)).toEqual(triangle)
  })

  it('a rectangle has no corner to remove', () => {
    expect(removeVertex(square, 0)).toBeNull()
  })

  it('curvature is set on a vertex, zero meaning none, and a rectangle expands first', () => {
    expect(setStrength(triangle, 1, 0.6).poly[1]).toEqual(point(50, 10, 0.6))
    expect(setStrength(triangle, 1, 1.7).poly[1]).toEqual(point(50, 10, 1))
    expect(setStrength(setStrength(triangle, 1, 0.6), 1, 0).poly[1]).toEqual(
      point(50, 10),
    )
    expect(setStrength(square, 0, 0.5).poly).toHaveLength(4)
  })
})

describe('simplifying a lasso', () => {
  it('drops points that sit on the line between their neighbours', () => {
    const noisy = [
      point(0, 0),
      point(25, 0.4),
      point(50, 0),
      point(50, 25),
      point(50, 50),
      point(0, 50),
      point(0, 25),
    ]
    expect(simplify(noisy, 1.5)).toEqual([
      point(0, 0),
      point(50, 0),
      point(50, 50),
      point(0, 50),
    ])
  })

  it('keeps a corner that matters', () => {
    const bent = [
      point(0, 0),
      point(50, 0),
      point(50, 50),
      point(25, 40),
      point(0, 50),
    ]
    expect(simplify(bent, 1.5)).toEqual(bent)
  })

  it('a lasso that encloses nothing is not a box', () => {
    expect(lassoBox([point(10, 10), point(12, 10), point(11, 12)])).toBeNull()
    expect(lassoBox([point(10, 10), point(90, 10), point(90, 90)])).toEqual({
      poly: [point(10, 10), point(90, 10), point(90, 90)],
    })
  })
})

describe('what goes on the wire', () => {
  it('is whole units and curvature to a hundredth', () => {
    expect(
      round({ poly: [point(10.4, 19.6, 0.333), point(50.5, 60.49)] }),
    ).toEqual({
      poly: [point(10, 20, 0.33), point(51, 60)],
    })
  })
})

describe('what is under the pointer', () => {
  const boxes = [square, triangle]

  it('a corner before the edge it ends', () => {
    expect(hitTest(boxes, point(61, 21), 3)).toEqual({
      kind: 'vertex',
      box: 0,
      index: 1,
    })
  })

  it('an edge before the inside', () => {
    expect(hitTest(boxes, point(40, 19), 3)).toEqual({
      kind: 'edge',
      box: 0,
      index: 0,
    })
  })

  it('the inside, the box drawn last first', () => {
    // (30,25) is inside both the square and the triangle.
    expect(hitTest(boxes, point(30, 25), 1)).toEqual({ kind: 'inside', box: 1 })
    expect(hitTest(boxes, point(40, 50), 1)).toEqual({ kind: 'inside', box: 0 })
  })

  it('nothing, off every box', () => {
    expect(hitTest(boxes, point(150, 150), 3)).toBeNull()
  })
})
