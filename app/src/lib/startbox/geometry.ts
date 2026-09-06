/**
 * The arithmetic of editing start boxes, in the 0-200 space the wire uses.
 *
 * Everything here is pure and knows nothing of pointers or pixels, so the
 * three promises the editor makes can be tested without a window: a vertex
 * or an edge dragged off the map stops at the border, a whole box dragged
 * off the map slides along the border without changing shape, and a box is
 * never smaller than the game can place a commander in.
 *
 * A box is the wire shape (`Box` from the bindings): two points are the
 * opposite corners of a rectangle, three or more are a polygon in order.
 * Rectangles stay two points as long as they are only moved and resized, so
 * what goes back on the wire is as small as what came in.
 */

import type { Box } from '../../ipc/bindings/Box'
import type { Point } from '../../ipc/bindings/Point'

/** The map, on both axes. */
export const SIZE = 200

/** The least a rectangle's side may be: bar-lobby's floor of 0.05 of the map. */
export const MIN_SIDE = 5

export type Bounds = {
  left: number
  top: number
  right: number
  bottom: number
}

export function point(
  x: number,
  y: number,
  strength: number | null = null,
): Point {
  return { x, y, strength }
}

export function isRect(box: Box): boolean {
  return box.poly.length === 2
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high)
}

export function clampPoint(p: Point): Point {
  return point(clamp(p.x, 0, SIZE), clamp(p.y, 0, SIZE), p.strength)
}

/**
 * The corners to draw and to grab, in order. Mirrors the game's `expandPoly`:
 * a rectangle's two points become four, going round from the first.
 */
export function corners(box: Box): Point[] {
  const [a, b] = box.poly
  if (a === undefined || b === undefined || box.poly.length !== 2)
    return box.poly
  return [point(a.x, a.y), point(b.x, a.y), point(b.x, b.y), point(a.x, b.y)]
}

export function bounds(box: Box): Bounds {
  const ring = corners(box)
  const first = ring[0] ?? point(0, 0)
  const out = { left: first.x, top: first.y, right: first.x, bottom: first.y }
  for (const p of ring) {
    out.left = Math.min(out.left, p.x)
    out.top = Math.min(out.top, p.y)
    out.right = Math.max(out.right, p.x)
    out.bottom = Math.max(out.bottom, p.y)
  }
  return out
}

/** A rectangle from any two opposite corners, stored min corner first. */
export function rect(a: Point, b: Point): Box {
  return {
    poly: [
      point(Math.min(a.x, b.x), Math.min(a.y, b.y)),
      point(Math.max(a.x, b.x), Math.max(a.y, b.y)),
    ],
  }
}

/**
 * The rectangle a drag from `from` to `to` describes, or nothing when it is
 * too small to be a box anyone meant to draw.
 */
export function rectFrom(from: Point, to: Point): Box | null {
  const a = clampPoint(from)
  const b = clampPoint(to)
  if (Math.abs(a.x - b.x) < MIN_SIDE || Math.abs(a.y - b.y) < MIN_SIDE)
    return null
  return rect(a, b)
}

/** Chobby's "add a box in the centre": a third of the map, in the middle. */
export function centreBox(): Box {
  return rect(point(66, 66), point(133, 133))
}

/**
 * Moves a whole box by as much of `(dx, dy)` as keeps it on the map. The
 * shape never changes: a box pushed past the border rides along it.
 */
export function translateClamped(box: Box, dx: number, dy: number): Box {
  const b = bounds(box)
  const fx = clamp(dx, -b.left, SIZE - b.right)
  const fy = clamp(dy, -b.top, SIZE - b.bottom)
  return { poly: box.poly.map((p) => point(p.x + fx, p.y + fy, p.strength)) }
}

/**
 * `value` pushed to at least `min` away from `anchor`, on the side it was
 * already on -- or the other side when the map ends first.
 */
function awayFrom(anchor: number, value: number, min: number): number {
  if (Math.abs(value - anchor) >= min) return value
  const sign = value >= anchor ? 1 : -1
  const pushed = anchor + sign * min
  return pushed >= 0 && pushed <= SIZE ? pushed : anchor - sign * min
}

/**
 * Moves one vertex to `to`, clamped to the map. On a rectangle the vertex is
 * one of its four corners: the opposite corner stays put and the box keeps
 * at least `MIN_SIDE` a side.
 */
export function moveVertex(box: Box, index: number, to: Point): Box {
  const target = clampPoint(to)
  if (!isRect(box)) {
    return {
      poly: box.poly.map((p, i) =>
        i === index ? point(target.x, target.y, p.strength) : p,
      ),
    }
  }
  const ring = corners(box)
  const opposite = ring[(index + 2) % 4]
  if (opposite === undefined) return box
  return rect(
    opposite,
    point(
      awayFrom(opposite.x, target.x, MIN_SIDE),
      awayFrom(opposite.y, target.y, MIN_SIDE),
    ),
  )
}

/**
 * Moves edge `index` -- from corner `index` to the next -- by `(dx, dy)`.
 *
 * A rectangle's side moves only along its own normal, stopping at the map's
 * edge and `MIN_SIDE` short of the opposite side. A polygon's edge carries
 * both its endpoints, by as much of the delta as keeps both on the map.
 */
export function moveEdge(box: Box, index: number, dx: number, dy: number): Box {
  if (isRect(box)) {
    const b = bounds(box)
    switch (index % 4) {
      case 0:
        return rect(
          point(b.left, clamp(b.top + dy, 0, b.bottom - MIN_SIDE)),
          point(b.right, b.bottom),
        )
      case 1:
        return rect(
          point(b.left, b.top),
          point(clamp(b.right + dx, b.left + MIN_SIDE, SIZE), b.bottom),
        )
      case 2:
        return rect(
          point(b.left, b.top),
          point(b.right, clamp(b.bottom + dy, b.top + MIN_SIDE, SIZE)),
        )
      default:
        return rect(
          point(clamp(b.left + dx, 0, b.right - MIN_SIDE), b.top),
          point(b.right, b.bottom),
        )
    }
  }
  const n = box.poly.length
  const a = box.poly[index % n]
  const c = box.poly[(index + 1) % n]
  if (a === undefined || c === undefined) return box
  const fx = clamp(dx, -Math.min(a.x, c.x), SIZE - Math.max(a.x, c.x))
  const fy = clamp(dy, -Math.min(a.y, c.y), SIZE - Math.max(a.y, c.y))
  return {
    poly: box.poly.map((p, i) =>
      i === index % n || i === (index + 1) % n
        ? point(p.x + fx, p.y + fy, p.strength)
        : p,
    ),
  }
}

/** A rectangle as the four-cornered polygon it draws as; a polygon unchanged. */
export function expandRect(box: Box): Box {
  return isRect(box) ? { poly: corners(box) } : box
}

/** A new vertex at `at` on edge `index`. A rectangle becomes a polygon first. */
export function insertVertex(box: Box, index: number, at: Point): Box {
  const poly = [...expandRect(box).poly]
  const target = clampPoint(at)
  poly.splice((index % poly.length) + 1, 0, point(target.x, target.y))
  return { poly }
}

/**
 * The polygon without vertex `index`, or `null` when that would leave fewer
 * than three -- or when the box is a rectangle, whose corners are not
 * something you remove one of.
 */
export function removeVertex(box: Box, index: number): Box | null {
  if (isRect(box) || box.poly.length <= 3) return null
  return { poly: box.poly.filter((_, i) => i !== index) }
}

/**
 * Sets the curvature at one vertex, 0-1 as the game reads it. Zero is a sharp
 * corner and is stored as absence, which is what the map metadata does. A
 * rectangle has no curvature to set and becomes a polygon first.
 */
export function setStrength(box: Box, index: number, strength: number): Box {
  const held = clamp(strength, 0, 1)
  return {
    poly: expandRect(box).poly.map((p, i) =>
      i === index ? point(p.x, p.y, held > 0 ? held : null) : p,
    ),
  }
}

function distanceToSegment(p: Point, a: Point, b: Point): number {
  const vx = b.x - a.x
  const vy = b.y - a.y
  const length = vx * vx + vy * vy
  const t =
    length === 0
      ? 0
      : clamp(((p.x - a.x) * vx + (p.y - a.y) * vy) / length, 0, 1)
  const qx = a.x + t * vx
  const qy = a.y + t * vy
  return Math.hypot(p.x - qx, p.y - qy)
}

/** Ramer–Douglas–Peucker over an open run of points. */
function simplifyRun(points: Point[], epsilon: number): Point[] {
  const first = points[0]
  const last = points[points.length - 1]
  if (first === undefined || last === undefined || points.length < 3)
    return points
  let farthest = 0
  let at = 0
  for (let i = 1; i < points.length - 1; i++) {
    const p = points[i]
    if (p === undefined) continue
    const d = distanceToSegment(p, first, last)
    if (d > farthest) {
      farthest = d
      at = i
    }
  }
  if (farthest <= epsilon) return [first, last]
  const head = simplifyRun(points.slice(0, at + 1), epsilon)
  const tail = simplifyRun(points.slice(at), epsilon)
  return [...head.slice(0, -1), ...tail]
}

/**
 * A freehand ring with the points that add nothing removed: no two closer
 * than `epsilon` to the line their neighbours make. The ring is split at the
 * two points farthest apart and each half simplified as a run, so the closing
 * edge is treated like any other.
 */
export function simplify(points: Point[], epsilon: number): Point[] {
  if (points.length < 4) return points
  const first = points[0]
  if (first === undefined) return points
  let split = 1
  let farthest = -1
  for (let i = 1; i < points.length; i++) {
    const p = points[i]
    if (p === undefined) continue
    const d = Math.hypot(p.x - first.x, p.y - first.y)
    if (d > farthest) {
      farthest = d
      split = i
    }
  }
  const forth = simplifyRun(points.slice(0, split + 1), epsilon)
  const back = simplifyRun([...points.slice(split), first], epsilon)
  return [...forth, ...back.slice(1, -1)]
}

/** Epsilon for a lasso as drawn: under a unit and a half is hand tremor. */
export const LASSO_EPSILON = 1.5

/**
 * The polygon a lasso describes, or nothing when it does not enclose an area
 * worth the name.
 */
export function lassoBox(points: Point[], epsilon = LASSO_EPSILON): Box | null {
  const poly = simplify(points.map(clampPoint), epsilon)
  if (poly.length < 3) return null
  const b = bounds({ poly })
  if (b.right - b.left < MIN_SIDE || b.bottom - b.top < MIN_SIDE) return null
  return { poly }
}

/**
 * The box as it goes on the wire: whole units, since a finer position is
 * below what a map pixel can show, and curvature to a hundredth.
 */
export function round(box: Box): Box {
  return {
    poly: box.poly.map((p) =>
      point(
        Math.round(p.x),
        Math.round(p.y),
        p.strength === null || p.strength === undefined
          ? null
          : Math.round(p.strength * 100) / 100,
      ),
    ),
  }
}

export type Hit =
  | { kind: 'vertex'; box: number; index: number }
  | { kind: 'edge'; box: number; index: number }
  | { kind: 'inside'; box: number }
  | null

/** Even-odd inside test, the rule the game's own `PointInPolygon` uses. */
export function inside(p: Point, ring: Point[]): boolean {
  let hit = false
  const n = ring.length
  for (let i = 0, j = n - 1; i < n; j = i++) {
    const a = ring[i]
    const b = ring[j]
    if (a === undefined || b === undefined) continue
    if (
      a.y > p.y !== b.y > p.y &&
      p.x < ((b.x - a.x) * (p.y - a.y)) / (b.y - a.y) + a.x
    ) {
      hit = !hit
    }
  }
  return hit
}

/**
 * What is under the pointer, the way a hand expects it: a corner before the
 * edge it ends, an edge before the box it bounds, and the box drawn last
 * before the one under it. `tolerance` is in map units; the caller turns a
 * comfortable number of pixels into that for the scale the map is drawn at.
 */
export function hitTest(boxes: Box[], p: Point, tolerance: number): Hit {
  for (let b = boxes.length - 1; b >= 0; b--) {
    const box = boxes[b]
    if (box === undefined) continue
    const ring = corners(box)
    for (let i = 0; i < ring.length; i++) {
      const v = ring[i]
      if (v !== undefined && Math.hypot(v.x - p.x, v.y - p.y) <= tolerance) {
        return { kind: 'vertex', box: b, index: i }
      }
    }
  }
  for (let b = boxes.length - 1; b >= 0; b--) {
    const box = boxes[b]
    if (box === undefined) continue
    const ring = corners(box)
    for (let i = 0; i < ring.length; i++) {
      const a = ring[i]
      const c = ring[(i + 1) % ring.length]
      if (
        a !== undefined &&
        c !== undefined &&
        distanceToSegment(p, a, c) <= tolerance
      ) {
        return { kind: 'edge', box: b, index: i }
      }
    }
  }
  for (let b = boxes.length - 1; b >= 0; b--) {
    const box = boxes[b]
    if (box !== undefined && inside(p, corners(box)))
      return { kind: 'inside', box: b }
  }
  return null
}
