/**
 * A length in `rem`, in the CSS pixels a virtualiser needs.
 *
 * Ctrl+wheel sizes the interface by writing `--ui-scale`, which the root font
 * size is a `calc` of — so every `rem` in the stylesheet follows the gesture,
 * but a row height written as a number in a `.tsx` file does not. A virtual
 * list has to hand its height over as a number, and this is where that number
 * comes from: measured off the root, re-measured whenever the size changes.
 */

import { createMemo } from 'solid-js'
import { uiScale } from '../store/settings'

/** What the root is drawn at when it cannot be measured, as in a test runner. */
const FALLBACK = 16

export function remPx(count: number): () => number {
	return createMemo(() => {
		// The dependency: the root font size is the whole of what scaling changes.
		uiScale()
		const root = parseFloat(getComputedStyle(document.documentElement).fontSize)
		return count * (root > 0 ? root : FALLBACK)
	})
}
