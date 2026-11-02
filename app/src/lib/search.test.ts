import { describe, expect, test } from 'vitest'
import { hasEveryWord, pieces, wordsOf } from './search'

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

	test('words split once match the same as the query', () => {
		const words = wordsOf('  ROOM download ')
		expect(words).toEqual(['room', 'download'])
		expect(hasEveryWord('Download what a room needs', words)).toBe(true)
		expect(hasEveryWord('Upload what a room needs', words)).toBe(false)
	})
})

describe('pieces', () => {
	test('an empty query leaves the text whole', () => {
		expect(pieces('Starting Metal', '')).toEqual([
			{ text: 'Starting Metal', hit: false },
		])
	})

	test('marks each word where it is, keeping the text as written', () => {
		expect(pieces('Starting Metal', 'metal start')).toEqual([
			{ text: 'Start', hit: true },
			{ text: 'ing ', hit: false },
			{ text: 'Metal', hit: true },
		])
	})

	test('marks every occurrence', () => {
		expect(pieces('ab ab', 'ab')).toEqual([
			{ text: 'ab', hit: true },
			{ text: ' ', hit: false },
			{ text: 'ab', hit: true },
		])
	})

	test('words that overlap or touch read as one mark', () => {
		expect(pieces('metalworks', 'metal alwo work')).toEqual([
			{ text: 'metalwork', hit: true },
			{ text: 's', hit: false },
		])
	})

	test('a text that changes length when folded is left whole', () => {
		expect(pieces('İstanbul', 'stan')).toEqual([
			{ text: 'İstanbul', hit: false },
		])
	})
})
