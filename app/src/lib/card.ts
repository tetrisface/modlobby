/**
 * Where the lobby card sits while the window is over a game.
 *
 * The window covers the game's whole monitor and the card is drawn inside it,
 * so the card's edges are the ones a person reaches for. Kept as its distance
 * from each edge of the window rather than as a size, so a card shown on
 * another monitor keeps its margins.
 *
 * Pure, as `resize.ts` is: which way an edge goes for which pointer delta is
 * the kind of sign error a test catches and a quick try does not.
 */

import type { WidthStore } from './resize'

export type Insets = {
	top: number
	right: number
	bottom: number
	left: number
}
export type Size = { width: number; height: number }

/** The card as it is drawn until somebody moves an edge. */
export const INSETS: Insets = { top: 40, right: 40, bottom: 40, left: 40 }

/** The window's own minimum (`tauri.conf.json`), which the page is laid out for. */
export const SMALLEST: Size = { width: 900, height: 600 }

/** A side or a corner, by compass point: a corner moves both sides it joins. */
export type Grip = 'n' | 'ne' | 'e' | 'se' | 's' | 'sw' | 'w' | 'nw'
export const GRIPS: readonly Grip[] = [
	'n',
	'ne',
	'e',
	'se',
	's',
	'sw',
	'w',
	'nw',
]

/** `value` within `low` and `high`, or at `low` where there is no room for it. */
function within(value: number, low: number, high: number): number {
	return Math.max(low, Math.min(value, high))
}

/**
 * The insets after `grip` was dragged by `(dx, dy)` from `start`. An edge
 * stops at the window's edge, and where the card would get smaller than
 * `smallest`, rather than pushing the opposite edge along.
 */
export function dragInsets(
	start: Insets,
	grip: Grip,
	dx: number,
	dy: number,
	view: Size,
	smallest: Size = SMALLEST,
): Insets {
	// What the two margins on an axis may take between them.
	const spareX = view.width - smallest.width
	const spareY = view.height - smallest.height
	return {
		top: grip.includes('n')
			? within(Math.round(start.top + dy), 0, spareY - start.bottom)
			: start.top,
		bottom: grip.includes('s')
			? within(Math.round(start.bottom - dy), 0, spareY - start.top)
			: start.bottom,
		left: grip.includes('w')
			? within(Math.round(start.left + dx), 0, spareX - start.right)
			: start.left,
		right: grip.includes('e')
			? within(Math.round(start.right - dx), 0, spareX - start.left)
			: start.right,
	}
}

/**
 * Saved insets as they fit `view`: as saved where they leave the card its
 * smallest size, and otherwise shrunk in proportion on both sides, so a card
 * saved on a larger monitor keeps its place relative to the edges.
 */
export function fitInsets(
	saved: Insets,
	view: Size,
	smallest: Size = SMALLEST,
): Insets {
	const [left, right] = share(
		saved.left,
		saved.right,
		view.width - smallest.width,
	)
	const [top, bottom] = share(
		saved.top,
		saved.bottom,
		view.height - smallest.height,
	)
	return { top, right, bottom, left }
}

/** Two margins within `spare`, scaled down together where they take more. */
function share(a: number, b: number, spare: number): [number, number] {
	if (a + b <= spare) return [a, b]
	if (spare <= 0) return [0, 0]
	// Multiplied before dividing: a ratio first can land a pixel short.
	return [Math.floor((a * spare) / (a + b)), Math.floor((b * spare) / (a + b))]
}

/**
 * The insets saved last time, as `[top, right, bottom, left]`, or `null` when
 * there are none worth trusting.
 */
export function readInsets(
	storage: WidthStore | null,
	key: string,
): Insets | null {
	if (!storage) return null
	try {
		const raw: unknown = JSON.parse(storage.getItem(key) ?? 'null')
		if (!Array.isArray(raw) || raw.length !== 4) return null
		const margin = (n: unknown) =>
			typeof n === 'number' && Number.isFinite(n) && n >= 0
		if (!raw.every(margin)) return null
		const [top, right, bottom, left] = raw as [number, number, number, number]
		return { top, right, bottom, left }
	} catch {
		return null
	}
}

/** `null` forgets them, and the card is drawn as it was at first. */
export function writeInsets(
	storage: WidthStore | null,
	key: string,
	insets: Insets | null,
): void {
	if (!storage) return
	const value = insets && [insets.top, insets.right, insets.bottom, insets.left]
	try {
		storage.setItem(key, JSON.stringify(value))
	} catch {
		// Forgotten, not broken: the card is drawn as it was at first.
	}
}
