/**
 * Curved start boxes, drawn as the game draws them.
 *
 * A vertex may carry a `strength` in 0-1; an edge's tension is the mean of
 * its two ends', and a tense edge is sampled along a centripetal Catmull-Rom
 * curve through its neighbours. This is a line-for-line port of BAR's
 * `common/lib_spline.lua` (`SplineLib.TessellateRing`), which is what the game
 * runs on the same anchors before testing where a commander may stand. The
 * editor draws through here so what it shows is what the game will enforce.
 */

import type { Box } from '../../ipc/bindings/Box'
import type { Point } from '../../ipc/bindings/Point'
import type { Poly } from '../boxes'
import { corners, point } from './geometry'

/** Subdivisions per curved edge; the game's default. */
export const SEGMENTS = 12

function clamp01(v: number): number {
  return v < 0 ? 0 : v > 1 ? 1 : v
}

/** `|delta|^0.5`: alpha 0.5, the centripetal parameterisation. */
function knotDelta(a: Point, b: Point): number {
  const dx = b.x - a.x
  const dy = b.y - a.y
  return (dx * dx + dy * dy) ** 0.25
}

function lerp(
  tt: number,
  ax: number,
  ay: number,
  bx: number,
  by: number,
  ta: number,
  tb: number,
): [number, number] {
  const w = (tb - tt) / (tb - ta)
  return [w * ax + (1 - w) * bx, w * ay + (1 - w) * by]
}

/**
 * One sample between `p1` and `p2`, blended from the straight line toward the
 * Catmull-Rom curve by `tension`. Barry-Goldman's form, which keeps the
 * anchors on the curve and avoids the overshoot uniform Catmull-Rom gets at
 * sharp corners between unevenly spaced anchors.
 */
function sample(
  p0: Point,
  p1: Point,
  p2: Point,
  p3: Point,
  t: number,
  tension: number,
): [number, number] {
  const lx = p1.x + (p2.x - p1.x) * t
  const ly = p1.y + (p2.y - p1.y) * t
  if (tension <= 0) return [lx, ly]

  const t0 = 0
  const t1 = t0 + knotDelta(p0, p1)
  const t2 = t1 + knotDelta(p1, p2)
  const t3 = t2 + knotDelta(p2, p3)

  let cx: number
  let cy: number
  if (t2 - t1 <= 1e-9) {
    cx = p1.x
    cy = p1.y
  } else {
    const tt = t1 + (t2 - t1) * t
    let [a1x, a1y] = [p1.x, p1.y]
    if (t1 - t0 > 1e-9) [a1x, a1y] = lerp(tt, p0.x, p0.y, p1.x, p1.y, t0, t1)
    const [a2x, a2y] = lerp(tt, p1.x, p1.y, p2.x, p2.y, t1, t2)
    let [a3x, a3y] = [p2.x, p2.y]
    if (t3 - t2 > 1e-9) [a3x, a3y] = lerp(tt, p2.x, p2.y, p3.x, p3.y, t2, t3)
    const [b1x, b1y] = lerp(tt, a1x, a1y, a2x, a2y, t0, t2)
    const [b2x, b2y] = lerp(tt, a2x, a2y, a3x, a3y, t1, t3)
    ;[cx, cy] = lerp(tt, b1x, b1y, b2x, b2y, t1, t2)
  }

  if (tension >= 1) return [cx, cy]
  return [lx + (cx - lx) * tension, ly + (cy - ly) * tension]
}

/**
 * The closed ring through `anchors`, every curved edge sampled `segments`
 * times. Anchors with no strength give back exactly the input, so a plain
 * polygon costs nothing.
 */
export function tessellateRing(anchors: Point[], segments = SEGMENTS): Point[] {
  const n = anchors.length
  if (n < 2) return anchors.map((p) => point(p.x, p.y))
  const steps = Math.max(1, Math.floor(segments))

  const out: Point[] = []
  for (let i = 0; i < n; i++) {
    const p0 = anchors[(i + n - 1) % n]
    const p1 = anchors[i]
    const p2 = anchors[(i + 1) % n]
    const p3 = anchors[(i + 2) % n]
    if (!p0 || !p1 || !p2 || !p3) continue

    const tension = clamp01(
      (clamp01(p1.strength ?? 0) + clamp01(p2.strength ?? 0)) * 0.5,
    )
    out.push(point(p1.x, p1.y))
    if (tension > 0 && n >= 3) {
      for (let k = 1; k < steps; k++) {
        const [x, y] = sample(p0, p1, p2, p3, k / steps, tension)
        out.push(point(x, y))
      }
    }
  }
  return out
}

/** The box as the outline to draw: corners expanded, curves sampled. */
export function ring(box: Box): Poly {
  return tessellateRing(corners(box)).map((p) => [p.x, p.y] as const)
}
