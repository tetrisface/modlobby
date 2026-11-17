import { describe, expect, test } from 'vitest'
import { watchStep } from './watch'

const idle = { state: 'idle' } as const

describe('watchStep', () => {
	test('fills with the engine download', () => {
		expect(
			watchStep('105', { phase: 'downloading', got: 21e6, total: 48e6 }, idle),
		).toMatchObject({ label: 'Engine 44%', fraction: 21 / 48 })
	})

	test('unpacking has no size to fill to', () => {
		expect(watchStep('105', { phase: 'extracting' }, idle)).toMatchObject({
			label: 'Unpacking',
			fraction: null,
		})
	})

	// The engine's `done` stays the latest word through pr-downloader's run.
	test('the game and map follow the engine', () => {
		const running = {
			state: 'running',
			what: 'Beyond All Reason test-26869-30e6ef8',
			current: 3,
			total: 4,
		} as const
		expect(
			watchStep('105', { phase: 'done', version: '105' }, running),
		).toMatchObject({ label: 'Content 75%', fraction: 0.75 })
	})

	test('with nothing left to fetch it is starting', () => {
		expect(watchStep('105', null, idle).label).toBe('Starting')
	})
})
