/**
 * How big a number or a corner handle is drawn on the map.
 *
 * The 0-200 start-box space is stretched over the picture, so anything with a
 * shape of its own -- a digit, a round handle -- is drawn through the editor's
 * `upright` anchor and sized here, in screen pixels, instead of in that space.
 *
 * Two things decide that size, and the order matters. The map view is the loud
 * one: the editor should read the same whether the drawing is 250 or 900 pixels
 * across, which is neither a fixed pixel size (lost on a large view, and the
 * complaint that started this) nor a fixed fraction of it (a zoom, illegible on
 * a small one). A fractional power is the weighting in between. The box is the
 * quiet one: a big box carries a slightly larger number than the sliver beside
 * it, at a third of the view's weight, so a room still reads as one type size.
 *
 * The box only ever nudges. Its term is unbounded below and has no knee, so a
 * box too small to hold its number overflows it rather than swallowing it --
 * a number that overflows can be found, one that vanishes cannot.
 */

import { SIZE, type Bounds } from './geometry'

/**
 * A size in CSS pixels.
 *
 * Not `drawn.ts`'s `Drawn`: that one is deliberately rounded up to whole 64px
 * steps for asking Rust for a picture, and a stepped size here would skew every
 * glyph it scales by as much as a tenth.
 */
export type Px = { width: number; height: number }

/** Pixels for the digit of a whole-map box on a `VIEW_REF` view: the one size set by eye. */
const LABEL_REF = 24
/** The view it was set on. The map column tops out near 876px, so most views interpolate down from here. */
const VIEW_REF = 800
/** The weight on the view: 1 would be a zoom, 0 the fixed pixels this replaces. */
const VIEW_POWER = 0.4
/** The weight on the box, a third of the view's: halving a box costs a tenth of its digit. */
const BOX_POWER = 0.15

/** Never smaller than the size that was there before, which was already called too small. */
export const LABEL_MIN = 13
/** Never large enough for a quarter of it to outgrow `GRAB_PIXELS`. */
export const LABEL_MAX = 28

/** How far from a corner or an edge a press still takes it, in screen pixels. */
export const GRAB_PIXELS = 8
/** A handle is a quarter of the digit beside it, so the two marks cannot drift apart. */
const HANDLE_RATIO = 0.25
/** Never smaller than the radius that was there before. */
const HANDLE_MIN = 4
/** The ring a handle is drawn with; half of it lies outside the radius. */
export const HANDLE_STROKE = 1.5

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high)
}

/**
 * The side of the square with the same area.
 *
 * One number for a two-dimensional size, blind to aspect -- these maps run from
 * 1:2 to 4:1 and the two pixel scales differ by as much -- and it fades on a
 * sliver where the shorter side alone would collapse. A side of nothing counts
 * as one, so a box mid-drag cannot raise zero to a power.
 */
function span(width: number, height: number): number {
  return Math.sqrt(Math.max(width, 1) * Math.max(height, 1))
}

/**
 * The size of a box's number, in screen pixels.
 *
 * `view` is the drawing in CSS pixels and `box` is in map units, which is all
 * the box term needs: the stretch that turns units into pixels cancels out of
 * the box's share of the view.
 */
export function labelSize(view: Px, box: Bounds): number {
  const seen = span(view.width, view.height) / VIEW_REF
  const share = span(box.right - box.left, box.bottom - box.top) / SIZE
  const size = LABEL_REF * seen ** VIEW_POWER * share ** BOX_POWER
  // A tenth of a pixel is under what a reader can see and over what a window
  // being dragged can churn through.
  return Math.round(clamp(size, LABEL_MIN, LABEL_MAX) * 10) / 10
}

/**
 * The radius of a corner handle, in screen pixels.
 *
 * Take `box` from the anchors rather than the drawn ring: bending an edge bows
 * the ring outward without moving a corner, and the beads have no business
 * growing when it does.
 *
 * It is never drawn past the reach that grabs it, the half of the ring outside
 * the radius counted, so the mark cannot promise a target `hitTest` will refuse.
 */
export function handleSize(view: Px, box: Bounds): number {
  const size = HANDLE_RATIO * labelSize(view, box)
  return (
    Math.round(clamp(size, HANDLE_MIN, GRAB_PIXELS - HANDLE_STROKE / 2) * 100) /
    100
  )
}
