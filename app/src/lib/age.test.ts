import { describe, expect, it } from 'vitest'

import { age, exactly } from './age'

const NOW = Date.parse('2026-09-19T12:00:00Z')
const ago = (days: number) => new Date(NOW - days * 86_400_000).toISOString()
const since = (ms: number) => new Date(NOW - ms).toISOString()

describe('age', () => {
	it('reads as the largest unit and the next', () => {
		expect(age(ago(400), NOW)).toBe('1y 1mo ago')
		expect(age(ago(102), NOW)).toBe('3mo 12d ago')
		expect(age(ago(5), NOW)).toBe('5d ago')
	})

	it('leaves out a next unit that is zero', () => {
		expect(age(ago(365), NOW)).toBe('1y ago')
		expect(age(ago(60), NOW)).toBe('2mo ago')
	})

	it('goes on down through hours, minutes and seconds, two units at a time', () => {
		expect(age(since(86_400_000 + 30 * 60_000), NOW)).toBe('1d ago')
		expect(age(since(2 * 3_600_000 + 5 * 60_000 + 9_000), NOW)).toBe(
			'2h 5m ago',
		)
		expect(age(since(5 * 60_000 + 9_000), NOW)).toBe('5m 9s ago')
		expect(age(since(40_000), NOW)).toBe('40s ago')
	})

	it('calls under a second just now, and a clock ahead of the data too', () => {
		expect(age(since(500), NOW)).toBe('just now')
		expect(age(ago(-1), NOW)).toBe('just now')
	})

	it('says nothing about a date nobody published', () => {
		expect(age('', NOW)).toBe('')
		expect(exactly('')).toBe('')
	})
})
