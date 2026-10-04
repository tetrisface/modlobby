import { describe, expect, test } from 'vitest'
import { standsAbreast } from './Watchers'

describe('standsAbreast', () => {
	test('two cards stand abreast when the teams leave room for both and, stacked, they would outgrow them', () => {
		expect(standsAbreast(2, 400, 300)).toBe(true)
		expect(standsAbreast(1, 400, 300)).toBe(false)
	})

	test('a stack no taller than the tallest team stays one over the other', () => {
		expect(standsAbreast(3, 300, 300)).toBe(false)
	})

	test('nothing stands abreast where the teams leave no room', () => {
		expect(standsAbreast(0, 400, 300)).toBe(false)
		expect(standsAbreast(-1, 400, 300)).toBe(false)
	})
})
