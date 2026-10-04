import { describe, expect, test } from 'vitest'
import { UNMEASURED, cardsAcross, teamColumns } from './Columns'

/** A card is 268 px and the gap 28 px, as the stylesheet has them at 16 px. */
const CARD = 268
const GAP = 28

describe('cardsAcross', () => {
	test('as many cards as the row holds with a gap between each', () => {
		// Three cards: 3 × 268 + 2 × 28 = 860.
		expect(cardsAcross(CARD, GAP, 860)).toBe(3)
		expect(cardsAcross(CARD, GAP, 859)).toBe(2)
		// Seven: 7 × 268 + 6 × 28 = 2044.
		expect(cardsAcross(CARD, GAP, 2044)).toBe(7)
		expect(cardsAcross(CARD, GAP, 1140)).toBe(3)
	})

	test('a row with no width yet, or no card to measure, holds none', () => {
		expect(cardsAcross(CARD, GAP, 0)).toBe(0)
		expect(cardsAcross(0, GAP, 860)).toBe(0)
	})
})

describe('teamColumns', () => {
	const space = (across: number, deep: number) => ({ across, deep })

	test('a column each while the deepest team fits', () => {
		expect(teamColumns([8, 8], space(5, 19))).toEqual([1, 1])
		expect(teamColumns([19, 3], space(5, 19))).toEqual([1, 1])
	})

	test('a side too deep takes the fewest columns that fit, and the other side flows with it', () => {
		// The room that asked for this: 38 v 29, where the old per-team
		// threshold spread one side across the width and left the other a
		// single column of 29.
		expect(teamColumns([38, 29], space(5, 33))).toEqual([2, 2])
		expect(teamColumns([38, 29], space(5, 19))).toEqual([2, 2])
		expect(teamColumns([30, 30], space(5, 19))).toEqual([2, 2])
	})

	test('a side that is shallow anyway keeps its one column', () => {
		expect(teamColumns([38, 3], space(5, 19))).toEqual([2, 1])
		expect(teamColumns([38, 0], space(5, 19))).toEqual([2, 1])
	})

	test('a row too narrow for that depth takes the shallowest it holds', () => {
		// Thirteen deep would be 3 + 3 columns; five across holds 3 + 2.
		expect(teamColumns([38, 29], space(5, 13))).toEqual([3, 2])
		expect(teamColumns([38, 29], space(3, 19))).toEqual([2, 1])
		expect(teamColumns([80, 80], space(5, 19))).toEqual([2, 2])
		expect(teamColumns([80, 80], space(6, 19))).toEqual([3, 3])
	})

	test('teams that share the row at nothing shallower keep a column each', () => {
		expect(teamColumns([25, 25, 25, 25], space(5, 19))).toEqual([1, 1, 1, 1])
		expect(teamColumns([30, 30], space(2, 19))).toEqual([1, 1])
		expect(teamColumns(Array(64).fill(40), space(5, 19))).toEqual(
			Array(64).fill(1),
		)
	})

	test('a cap too short for one row still counts as one', () => {
		expect(teamColumns([8, 8], space(4, 0))).toEqual([2, 2])
	})

	test('a row nobody has measured flows nothing', () => {
		expect(teamColumns([80, 80], UNMEASURED)).toEqual([1, 1])
	})
})
