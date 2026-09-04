/**
 * How big a box is drawn, for asking Rust for a picture of that size.
 *
 * The map squares are as wide as their column, so their size is measured, not
 * assumed. It is reported in steps: a window being dragged wider must not ask
 * for a new picture every frame, and the webview shrinking a picture by less
 * than a step is nowhere near the ratio that aliases.
 */

import { createEffect, createSignal, onCleanup, type Accessor } from 'solid-js'

export type Drawn = { width: number; height: number }

/** CSS pixels per step. */
export const STEP = 64

/** `side` rounded up to a whole number of steps. Zero stays zero. */
export function step(side: number): number {
  return Math.ceil(side / STEP) * STEP
}

/**
 * The box `el` is drawn in, in stepped CSS pixels, followed as it is laid out
 * and resized. `null` until it has both a width and a height, so nothing is
 * asked for a box that is not on screen yet.
 */
export function createDrawnSize(
  el: Accessor<HTMLElement | undefined>,
): Accessor<Drawn | null> {
  const [drawn, setDrawn] = createSignal<Drawn | null>(null, {
    equals: (a, b) => a?.width === b?.width && a?.height === b?.height,
  })
  createEffect(() => {
    const element = el()
    if (!element) return
    const measure = () => {
      const width = step(element.clientWidth)
      const height = step(element.clientHeight)
      setDrawn(width > 0 && height > 0 ? { width, height } : null)
    }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    onCleanup(() => observer.disconnect())
  })
  return drawn
}
