import { describe, expect, test } from 'vitest'
import type { Score } from '../ipc/bindings/Score'
import type { SizeScore } from '../ipc/bindings/SizeScore'
import { playerHandicap, reseated, uncertainSizes } from './teamSize'

function size(
	players: number,
	challenge: number,
	uncertain = false,
): SizeScore {
	return {
		players,
		challenge,
		winChance: 1 - challenge / 34,
		percentile: challenge * 2,
		uncertain,
	}
}

/** Asked with two seated and four seats, everyone unhandicapped. */
const asked = {
	challenge: 17,
	winChance: 0.5,
	percentile: 34,
	full: size(4, 12),
	sizes: [size(1, 21), size(2, 17), size(3, 14), size(4, 12)],
	seated: 2,
	handicap: 0,
} as Score

describe('playerHandicap', () => {
	test('averages and rounds the way the service and crates/pve do', () => {
		// The same cases `crates/pve` checks `player_handicap` against.
		expect(playerHandicap([])).toBeNull()
		expect(playerHandicap([0, 0])).toBe(0)
		expect(playerHandicap([0, 10, 0])).toBe(3.3)
		expect(playerHandicap([5, 0])).toBe(2.5)
		expect(playerHandicap([100, 0])).toBe(50)
	})
})

describe('reseated', () => {
	test('the room as asked is the answer itself', () => {
		expect(reseated(asked, { players: 2, handicap: 0 })).toBe(asked)
	})

	test('another head count reads that size off the table, the rest unchanged', () => {
		const three = reseated(asked, { players: 3, handicap: 0 })
		expect(three).toMatchObject({
			challenge: 14,
			winChance: 1 - 14 / 34,
			percentile: 28,
			seated: 3,
			full: asked.full,
		})
		expect(reseated(asked, { players: 4, handicap: 0 })?.challenge).toBe(12)
	})

	test('another handicap, or a size the table lacks, cannot be answered', () => {
		expect(reseated(asked, { players: 3, handicap: 5 })).toBeNull()
		expect(reseated(asked, { players: 2, handicap: 5 })).toBeNull()
		expect(reseated(asked, { players: 5, handicap: 0 })).toBeNull()
		// An answer from a service without the table has only itself.
		expect(
			reseated({ ...asked, sizes: [] }, { players: 3, handicap: 0 }),
		).toBeNull()
	})
})

describe('uncertainSizes', () => {
	test('names the seated and full sizes the model barely saw, once each', () => {
		expect(uncertainSizes(asked)).toEqual([])
		const big = {
			...asked,
			seated: 17,
			full: size(17, 15, true),
			sizes: [size(16, 7), size(17, 15, true)],
		} as Score
		expect(uncertainSizes(big)).toEqual([17])
		expect(
			uncertainSizes({ ...big, seated: 16, full: size(20, 15, true) }),
		).toEqual([20])
	})
})
