import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { ListedScore } from '../ipc/bindings/ListedScore'
import type { Settings } from '../ipc/bindings/Settings'
import { POLL_EVERY, listed, watchListedScores } from './listed'
import { setSettingsSignal } from './settings'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const asked = vi.mocked(invoke)

const KEY = 'server4.beyondallreason.info/5'
const SCORE: ListedScore = { challenge: 16.4, scoredAt: '2026-11-07T18:00:00Z' }

/** Lets an awaited answer reach the store. */
async function settle() {
	for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

let visibility: DocumentVisibilityState = 'visible'

function show(state: DocumentVisibilityState) {
	visibility = state
	document.dispatchEvent(new Event('visibilitychange'))
}

function pveStats(on: boolean) {
	setSettingsSignal({ play: { pveStats: on } } as unknown as Settings)
}

describe('what rooms last scored', () => {
	let stops: Array<() => void> = []

	function watch() {
		const stop = watchListedScores()
		stops.push(stop)
		return stop
	}

	beforeEach(() => {
		vi.useFakeTimers()
		visibility = 'visible'
		Object.defineProperty(document, 'visibilityState', {
			configurable: true,
			get: () => visibility,
		})
		asked.mockReset()
		asked.mockResolvedValue({ [KEY]: SCORE })
		pveStats(true)
	})

	afterEach(async () => {
		for (const stop of stops) stop()
		stops = []
		// The store outlives a test; an empty answer with the setting off
		// leaves it empty for the next.
		pveStats(false)
		watchListedScores()()
		await settle()
		setSettingsSignal(null)
		vi.useRealTimers()
	})

	test('is asked on mount and then once a poll', async () => {
		watch()
		await settle()
		expect(asked).toHaveBeenCalledWith('pve_listed_scores')
		expect(listed()[KEY]).toEqual(SCORE)

		vi.advanceTimersByTime(POLL_EVERY)
		await settle()
		expect(asked).toHaveBeenCalledTimes(2)
	})

	test('is never asked with the setting off, and shows nothing', async () => {
		watch()
		await settle()
		expect(listed()[KEY]).toEqual(SCORE)

		pveStats(false)
		vi.advanceTimersByTime(POLL_EVERY)
		await settle()
		expect(asked).toHaveBeenCalledTimes(1)
		expect(listed()).toEqual({})
	})

	test('a hidden window is not asked for; coming back asks at once', async () => {
		visibility = 'hidden'
		watch()
		vi.advanceTimersByTime(POLL_EVERY)
		await settle()
		expect(asked).not.toHaveBeenCalled()

		show('visible')
		await settle()
		expect(asked).toHaveBeenCalledTimes(1)
		expect(listed()[KEY]).toEqual(SCORE)
	})

	test('a failed ask keeps the numbers already shown', async () => {
		watch()
		await settle()
		asked.mockRejectedValue({ code: 'pve', message: 'pve stats answered 503' })
		vi.advanceTimersByTime(POLL_EVERY)
		await settle()
		expect(asked).toHaveBeenCalledTimes(2)
		expect(listed()[KEY]).toEqual(SCORE)
	})

	test('two pages showing the list share one poll, which stops with the last', async () => {
		const first = watch()
		const second = watch()
		await settle()
		expect(asked).toHaveBeenCalledTimes(1)

		first()
		vi.advanceTimersByTime(POLL_EVERY)
		await settle()
		expect(asked).toHaveBeenCalledTimes(2)

		second()
		stops = []
		vi.advanceTimersByTime(POLL_EVERY)
		show('visible')
		await settle()
		expect(asked).toHaveBeenCalledTimes(2)
	})
})
