/**
 * The arithmetic behind a pane you drag wider or narrower.
 *
 * Kept apart from the pointer events so the clamping and the persistence can
 * be tested without a window: the handle sits on the pane's *left* edge, so
 * moving the pointer left makes the pane wider, which is the kind of sign
 * error a test catches and a quick try in the app does not.
 */

export type Bounds = { min: number; max: number }

/** The subset of `Storage` used here, so a test can pass a plain object. */
export type WidthStore = Pick<Storage, 'getItem' | 'setItem'>

export function clamp(width: number, bounds: Bounds): number {
  return Math.min(Math.max(width, bounds.min), bounds.max)
}

/** The width after the pointer moved from `startX` to `x`, kept in bounds. */
export function dragWidth(
  startWidth: number,
  startX: number,
  x: number,
  bounds: Bounds,
): number {
  return clamp(Math.round(startWidth - (x - startX)), bounds)
}

/**
 * The width saved last time, or `null` when there is none worth trusting.
 *
 * `null` rather than a default because the caller may have something better
 * than a constant to fall back on -- a measurement of what would fit.
 */
export function readWidth(
  storage: WidthStore | null,
  key: string,
): number | null {
  if (!storage) return null
  let raw: string | null
  try {
    raw = storage.getItem(key)
  } catch {
    return null
  }
  if (raw === null) return null
  const width = Number(raw)
  return Number.isFinite(width) && width > 0 ? width : null
}

export function writeWidth(
  storage: WidthStore | null,
  key: string,
  width: number,
): void {
  if (!storage) return
  try {
    storage.setItem(key, String(Math.round(width)))
  } catch {
    // Storage that refuses a write (private mode, a full quota) costs the
    // reader nothing but remembering the width next time.
  }
}
