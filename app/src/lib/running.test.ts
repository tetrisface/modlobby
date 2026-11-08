import { describe, expect, test } from 'vitest'
import { elapsed, stopwatch, told, track, type Running } from './running'

const at = (minute: number) => minute * 60_000
const always = () => true
const never = () => false

describe('timing a game nobody told us the start of', () => {
	test('the first look can only give a floor', () => {
		// Logging in: these were already running, and how long for is unknowable.
		const seen = track({}, new Set(['1', '2']), never, at(0))
		expect(seen[1]).toEqual({ since: at(0), exact: false })
		expect(seen[2]?.exact).toBe(false)
	})

	test('a game that starts under us is timed exactly', () => {
		const first = track({}, new Set(['1']), never, at(0))
		const later = track(first, new Set(['1', '2']), always, at(5))
		expect(later[1]).toEqual({ since: at(0), exact: false })
		expect(later[2]).toEqual({ since: at(5), exact: true })
	})

	test('a start already known is never restarted', () => {
		let held = track({}, new Set(['1']), always, at(0))
		for (const minute of [1, 2, 3])
			held = track(held, new Set(['1']), always, at(minute))
		expect(held[1]?.since).toBe(at(0))
	})

	test('a game that ends is forgotten, and a new one is timed afresh', () => {
		const first = track({}, new Set(['1']), always, at(0))
		const ended = track(first, new Set(), always, at(9))
		expect(ended[1]).toBeUndefined()
		const again = track(ended, new Set(['1']), always, at(20))
		expect(again[1]).toEqual({ since: at(20), exact: true })
	})

	test('each server has its own first look', () => {
		// One server came back after a dropped session: its games were going
		// all along, while the other server's new game started under us.
		const back = (key: string) => !key.startsWith('dropped/')
		const seen = track({}, new Set(['dropped/1', 'steady/2']), back, at(4))
		expect(seen['dropped/1']).toEqual({ since: at(4), exact: false })
		expect(seen['steady/2']).toEqual({ since: at(4), exact: true })
	})

	test('a room that closes while running takes its timing with it', () => {
		const first = track({}, new Set(['1', '2']), always, at(0))
		expect(track(first, new Set(['2']), always, at(3))).toEqual({
			2: { since: at(0), exact: true },
		})
	})
})

describe('a start we were told', () => {
	test('replaces a floor with the real thing', () => {
		const guessed = track({}, new Set(['7']), never, at(30))
		expect(guessed[7]).toEqual({ since: at(30), exact: false })
		const known = told(guessed, '7', 28 * 60 + 4, at(30))
		expect(known[7]).toEqual({
			since: at(30) - (28 * 60 + 4) * 1000,
			exact: true,
		})
	})

	test('replaces an exact we only thought we had', () => {
		// A room can appear with its game already going, and then the moment we
		// first looked is not the start at all.
		const assumed = track({}, new Set(['7']), always, at(30))
		expect(assumed[7]?.exact).toBe(true)
		expect(told(assumed, '7', 600, at(30))[7]?.since).toBe(at(20))
	})

	test('a later tick keeps what we were told', () => {
		const known = told({}, '7', 600, at(30))
		const ticked = track(known, new Set(['7']), always, at(31))
		expect(ticked[7]).toEqual(known[7])
	})

	test('it says nothing about anyone else', () => {
		const both = track({}, new Set(['7', '8']), always, at(30))
		const known = told(both, '7', 600, at(30))
		expect(known[8]).toEqual(both[8])
	})
})

describe('saying how long', () => {
	const exact = (since: number): Running => ({ since, exact: true })
	const floor = (since: number): Running => ({ since, exact: false })

	test('minutes, then hours and minutes', () => {
		expect(elapsed(exact(at(0)), at(7))).toBe('7m')
		expect(elapsed(exact(at(0)), at(59))).toBe('59m')
		expect(elapsed(exact(at(0)), at(64))).toBe('1h04')
		expect(elapsed(exact(at(0)), at(130))).toBe('2h10')
	})

	test('a floor is marked, so nobody reads it as the real thing', () => {
		expect(elapsed(floor(at(0)), at(7))).toBe('7m+')
		expect(elapsed(floor(at(0)), at(64))).toBe('1h04+')
	})

	test('a game that has only just started says so', () => {
		expect(elapsed(exact(at(0)), at(0.9))).toBe('<1m')
		expect(elapsed(exact(at(0)), at(1))).toBe('1m')
		// A floor under a minute is no figure at all, until it is one.
		expect(elapsed(floor(at(0)), at(0.5))).toBe('')
		expect(elapsed(floor(at(0)), at(1))).toBe('1m+')
	})

	test('a clock that went backwards says nothing silly', () => {
		expect(elapsed(exact(at(10)), at(0))).toBe('<1m')
	})

	test('a stopwatch counts minutes past the hour rather than rolling over', () => {
		expect(stopwatch(0, 425_000)).toBe('07:05')
		expect(stopwatch(0, at(75) + 12_000)).toBe('75:12')
		expect(stopwatch(at(10), at(0))).toBe('00:00')
	})
})
