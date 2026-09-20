import { describe, expect, test } from 'vitest'
import { hasEveryWord } from './search'

describe('hasEveryWord', () => {
	test('an empty or blank query matches anything', () => {
		expect(hasEveryWord('Download what a room needs', '')).toBe(true)
		expect(hasEveryWord('', '   ')).toBe(true)
	})

	test('every word must appear, in any order and case', () => {
		const text = 'Download what a room needs automatically'
		expect(hasEveryWord(text, 'ROOM download')).toBe(true)
		expect(hasEveryWord(text, 'room upload')).toBe(false)
	})

	test('a word may sit inside a longer one', () => {
		expect(hasEveryWord('a metered connection', 'meter')).toBe(true)
	})
})
