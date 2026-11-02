import { createRoot, createSignal } from 'solid-js'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { OptionChangeView } from '../ipc/bindings/OptionChangeView'
import { RECENT_SECONDS, createRecent } from './recent'

const START = 1_800_000_000

const change = (key: string, at: number, seq = at): OptionChangeView => ({
	seq,
	key,
	from: '',
	to: '1',
	by: null,
	at,
})

beforeEach(() => {
	vi.useFakeTimers()
	vi.setSystemTime(START * 1000)
})

afterEach(() => {
	vi.useRealTimers()
})

describe('createRecent', () => {
	function keep(history: () => OptionChangeView[]) {
		return createRoot((dispose) => {
			const recent = createRecent(history)
			return { recent, dispose }
		})
	}

	test('a change just seen is recent, and one from long ago is not', () => {
		const { recent, dispose } = keep(() => [
			change('old', START - RECENT_SECONDS - 1),
			change('new', START - 1),
		])
		expect([...recent()]).toEqual(['new'])
		dispose()
	})

	test('goes out by itself once its time is up', () => {
		const { recent, dispose } = keep(() => [change('x', START)])
		expect(recent().has('x')).toBe(true)
		vi.advanceTimersByTime(RECENT_SECONDS * 1000 - 1000)
		expect(recent().has('x')).toBe(true)
		vi.advanceTimersByTime(2000)
		expect(recent().has('x')).toBe(false)
		dispose()
	})

	test('a change arriving later starts its own time', () => {
		const [history, setHistory] = createSignal<OptionChangeView[]>([])
		const { recent, dispose } = keep(history)
		expect(recent().size).toBe(0)

		vi.advanceTimersByTime(60_000)
		setHistory([change('x', START + 60)])
		expect(recent().has('x')).toBe(true)

		vi.advanceTimersByTime(RECENT_SECONDS * 1000 + 1000)
		expect(recent().has('x')).toBe(false)
		dispose()
	})

	test('a key changed again stays recent from its newest change', () => {
		const [history, setHistory] = createSignal([change('x', START)])
		const { recent, dispose } = keep(history)

		vi.advanceTimersByTime(RECENT_SECONDS * 1000 - 5000)
		setHistory([
			change('x', START),
			change('x', START + RECENT_SECONDS - 5, START + 1),
		])
		vi.advanceTimersByTime(10_000)
		expect(recent().has('x')).toBe(true)
		dispose()
	})

	test('nothing is left waiting once it is disposed', () => {
		const { dispose } = keep(() => [change('x', START)])
		dispose()
		expect(vi.getTimerCount()).toBe(0)
	})
})
