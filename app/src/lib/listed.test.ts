import { describe, expect, test } from 'vitest'
import type { ListedScore } from '../ipc/bindings/ListedScore'
import { challengeLabel, challengeParts, challengeTip } from './listed'

const SCORED_AT = '2026-11-07T18:00:00Z'
const filling: ListedScore = {
	challenge: 12.4,
	fullChallenge: 15.1,
	players: 3,
	scoredAt: SCORED_AT,
}
const older: ListedScore = {
	challenge: 16.4,
	fullChallenge: null,
	players: null,
	scoredAt: SCORED_AT,
}

describe('a listed challenge', () => {
	test('reads as now–full, whole numbers and an en dash', () => {
		expect(challengeLabel(filling)).toBe('12–15')
		expect(challengeParts(filling)).toEqual({ now: '12', main: '15' })
	})

	test('the full-team figure leads even when it is the easier one', () => {
		const easing = { ...filling, challenge: 20.2, fullChallenge: 13.6 }
		expect(challengeLabel(easing)).toBe('20–14')
		expect(challengeParts(easing).main).toBe('14')
	})

	test('one figure when the two read the same or there is only one', () => {
		expect(challengeLabel({ ...filling, fullChallenge: 12.3 })).toBe('12')
		expect(challengeParts({ ...filling, fullChallenge: 12.3 })).toEqual({
			now: null,
			main: '12',
		})
		expect(challengeLabel(older)).toBe('16')
		expect(challengeLabel({ ...older, challenge: 34 })).toBe('34')
		expect(challengeLabel({ ...older, challenge: 0.04 })).toBe('0')
	})

	test('explains its scale, who each figure is for and how old they are', () => {
		const twelveMinutesOn = Date.parse(SCORED_AT) + 12 * 60_000
		expect(challengeTip(filling, twelveMinutesOn).split('\n')).toEqual([
			'Challenge out of 34, where 17 is an even game.',
			'With the 3 players seated when it was scored: 12.4',
			'With every seat taken: 15.1',
			'Last scored 12m ago by someone in this room; a change since then shows here only if it moved the map.',
		])
		expect(challengeTip({ ...filling, players: 1 }, twelveMinutesOn)).toContain(
			'With the 1 player seated',
		)
	})

	test('an older row says only what it knows', () => {
		const tip = challengeTip({ ...older, scoredAt: 'whenever' }, Date.now())
		expect(tip).toBe(
			'Challenge out of 34, where 17 is an even game.\nAs it was scored: 16.4',
		)
	})
})
