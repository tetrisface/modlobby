import { createRoot } from 'solid-js'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { REST_MS, createInspecting } from './inspecting'

beforeEach(() => {
	vi.useFakeTimers()
})

afterEach(() => {
	vi.useRealTimers()
})

function inspecting() {
	return createRoot((dispose) => ({ ...createInspecting(), dispose }))
}

describe('createInspecting', () => {
	test('a row is read once the pointer has rested on it, not as it crosses', () => {
		const it = inspecting()
		it.point('a')
		expect(it.key()).toBeNull()
		vi.advanceTimersByTime(REST_MS)
		expect(it.key()).toBe('a')
		it.dispose()
	})

	test('rows crossed on the way somewhere are never read', () => {
		const it = inspecting()
		it.point('a')
		vi.advanceTimersByTime(REST_MS - 1)
		it.point('b')
		vi.advanceTimersByTime(REST_MS - 1)
		it.point('c')
		vi.advanceTimersByTime(REST_MS)
		expect(it.key()).toBe('c')
		it.dispose()
	})

	test('between rows nothing new is picked and the row being read stays', () => {
		const it = inspecting()
		it.point('a')
		vi.advanceTimersByTime(REST_MS)
		it.point('b')
		it.point(null)
		vi.advanceTimersByTime(REST_MS)
		expect(it.key()).toBe('a')
		it.dispose()
	})

	test('leaving the list for the strip under it keeps the row, so its choices can be reached', () => {
		const it = inspecting()
		it.point('a')
		vi.advanceTimersByTime(REST_MS)
		it.point('b')
		it.stop()
		vi.advanceTimersByTime(REST_MS)
		expect(it.key()).toBe('a')
		it.dispose()
	})

	test('without a pin the last row read stays after the pointer has gone', () => {
		const it = inspecting()
		it.point('a')
		vi.advanceTimersByTime(REST_MS)
		it.leave()
		expect(it.key()).toBe('a')
		it.dispose()
	})

	test('a pin is read at once, and returned to when the pointer goes', () => {
		const it = inspecting()
		it.toggle('a')
		expect(it.key()).toBe('a')
		expect(it.pinned()).toBe('a')

		it.point('b')
		vi.advanceTimersByTime(REST_MS)
		expect(it.key()).toBe('b')
		expect(it.pinned()).toBe('a')

		it.leave()
		expect(it.key()).toBe('a')
		it.dispose()
	})

	test('pressing the pinned row again lets go of it', () => {
		const it = inspecting()
		it.toggle('a')
		it.toggle('a')
		expect(it.pinned()).toBeNull()
		it.dispose()
	})

	test('a row taking the focus is pinned and stays pinned however often', () => {
		const it = inspecting()
		it.pin('a')
		it.pin('a')
		expect(it.pinned()).toBe('a')
		expect(it.key()).toBe('a')
		it.dispose()
	})

	test('a pin takes over from a row still waiting for its rest', () => {
		const it = inspecting()
		it.point('b')
		it.pin('a')
		vi.advanceTimersByTime(REST_MS)
		expect(it.key()).toBe('a')
		it.dispose()
	})

	test('nothing is left waiting once it is disposed', () => {
		const it = inspecting()
		it.point('a')
		it.dispose()
		expect(vi.getTimerCount()).toBe(0)
	})
})
