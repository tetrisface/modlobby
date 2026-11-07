import { describe, expect, test } from 'vitest'
import { challengeLabel, challengeTip } from './listed'

const SCORED_AT = '2026-11-07T18:00:00Z'
const score = { challenge: 16.4, scoredAt: SCORED_AT }

describe('a listed challenge', () => {
	test("reads as the panel's figure, without its scale", () => {
		expect(challengeLabel(score)).toBe('16.4')
		expect(challengeLabel({ ...score, challenge: 34 })).toBe('34.0')
		expect(challengeLabel({ ...score, challenge: 0.04 })).toBe('0.0')
	})

	test('explains its scale and how long ago it was scored', () => {
		const twelveMinutesOn = Date.parse(SCORED_AT) + 12 * 60_000
		const tip = challengeTip(score, twelveMinutesOn)
		expect(tip).toContain('Challenge 16.4 of 34, where 17 is an even game.')
		expect(tip).toContain('Last scored 12m ago by someone in this room')
		expect(tip).toContain('only if it moved the map')
	})

	test('a moment it cannot read leaves the age out, not the scale', () => {
		const tip = challengeTip({ ...score, scoredAt: 'whenever' }, Date.now())
		expect(tip).toBe('Challenge 16.4 of 34, where 17 is an even game.')
	})
})
