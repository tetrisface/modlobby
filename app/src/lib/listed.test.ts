import { describe, expect, test } from 'vitest'
import type { ListedScore } from '../ipc/bindings/ListedScore'
import {
	challengeBand,
	challengeLabel,
	challengeParts,
	challengeTip,
} from './listed'

const SCORED_AT = '2026-11-07T18:00:00Z'
const filling: ListedScore = {
	challenge: 12.4,
	fullChallenge: 15.1,
	players: 3,
	sizes: {},
	scoredAt: SCORED_AT,
}
const older: ListedScore = {
	challenge: 16.4,
	fullChallenge: null,
	players: null,
	sizes: {},
	scoredAt: SCORED_AT,
}
/** Scored with one player seated, its table filled in to four seats. */
const tabled: ListedScore = {
	challenge: 21.5,
	fullChallenge: 12.0,
	players: 1,
	sizes: { 1: 21.5, 2: 17.0, 3: 14.2, 4: 12.0 },
	scoredAt: SCORED_AT,
}
const twelveMinutesOn = Date.parse(SCORED_AT) + 12 * 60_000

describe('a listed challenge', () => {
	test('reads as now–full, whole numbers and an en dash', () => {
		expect(challengeLabel(filling, 3)).toBe('12–15')
		expect(challengeParts(filling, 3)).toEqual({ now: '12', main: '15' })
	})

	test('the full-team figure leads even when it is the easier one', () => {
		const easing = { ...filling, challenge: 20.2, fullChallenge: 13.6 }
		expect(challengeLabel(easing, 3)).toBe('20–14')
		expect(challengeParts(easing, 3).main).toBe('14')
	})

	test('one figure when the two read the same or there is only one', () => {
		expect(challengeLabel({ ...filling, fullChallenge: 12.3 }, 3)).toBe('12')
		expect(challengeParts({ ...filling, fullChallenge: 12.3 }, 3)).toEqual({
			now: null,
			main: '12',
		})
		expect(challengeLabel(older, 3)).toBe('16')
		expect(challengeLabel({ ...older, challenge: 34 }, 3)).toBe('34')
		expect(challengeLabel({ ...older, challenge: 0.04 }, 3)).toBe('0')
	})

	test('colours by the bold figure as it reads on screen', () => {
		const at = (fullChallenge: number) =>
			challengeBand({ ...filling, fullChallenge }, 3)
		expect(at(24.4)).toBeNull()
		expect(at(24.5)).toBe('hard')
		expect(at(29.4)).toBe('hard')
		expect(at(29.5)).toBe('extreme')
		// A hard seat count on the way to an easy full room stays plain.
		expect(
			challengeBand({ ...filling, challenge: 31, fullChallenge: 22 }, 3),
		).toBeNull()
		expect(challengeBand({ ...older, challenge: 26 }, 3)).toBe('hard')
	})

	test('explains its scale, who each figure is for and how old they are', () => {
		expect(challengeTip(filling, 3, twelveMinutesOn).split('\n')).toEqual([
			'Challenge out of 34, where 17 is an even game; yellow from 25, red from 30.',
			'With the 3 players seated when it was scored: 12.4',
			'With every seat taken: 15.1',
			'Last scored 12m ago by someone in this room; a change since then shows here only if it moved the map.',
		])
		expect(
			challengeTip({ ...filling, players: 1 }, 3, twelveMinutesOn),
		).toContain('With the 1 player seated')
	})

	test('an older row says only what it knows', () => {
		const tip = challengeTip({ ...older, scoredAt: 'whenever' }, 3, Date.now())
		expect(tip).toBe(
			'Challenge out of 34, where 17 is an even game; yellow from 25, red from 30.\nAs it was scored: 16.4',
		)
	})
})

describe('a listed challenge with its table', () => {
	test('follows the seats taken since the room was asked about', () => {
		expect(challengeLabel(tabled, 1)).toBe('22–12')
		expect(challengeLabel(tabled, 3)).toBe('14–12')
		// Every seat taken: the two figures are one.
		expect(challengeLabel(tabled, 4)).toBe('12')
	})

	test('colours by the table entry where there is no full-team figure', () => {
		const solo = { ...tabled, fullChallenge: null }
		expect(challengeBand(solo, 1)).toBeNull()
		expect(challengeBand({ ...solo, sizes: { 1: 30.2 } }, 1)).toBe('extreme')
	})

	test('says the figure is for the room as seated now', () => {
		expect(challengeTip(tabled, 2, twelveMinutesOn).split('\n')).toEqual([
			'Challenge out of 34, where 17 is an even game; yellow from 25, red from 30.',
			'With the 2 players seated now: 17.0',
			'With every seat taken: 12.0',
			'Last scored 12m ago by someone in this room; seats taken since are counted, other changes show here only if they moved the map.',
		])
	})

	test('an empty room, or a head count past the table, shows the full team alone', () => {
		for (const players of [0, 5]) {
			expect(challengeParts(tabled, players)).toEqual({ now: null, main: '12' })
			expect(challengeTip(tabled, players, twelveMinutesOn)).not.toContain(
				'seated',
			)
		}
	})
})
