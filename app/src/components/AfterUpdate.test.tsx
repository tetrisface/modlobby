import { cleanup, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { Boot } from '../ipc/bindings/Boot'
import { AfterUpdate } from './AfterUpdate'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const asked = vi.mocked(invoke)
const navigation = vi.spyOn(performance, 'getEntriesByType')

afterEach(() => {
	cleanup()
	asked.mockReset()
	navigation.mockReset()
	delete window.__MODLOBBY_BOOT__
})

/** Lets the awaits between a mount and what it draws run. */
async function settle() {
	for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

/** What Rust hands over with the page, on the load it was made for. */
function handedOver(keptUpdate: string | null) {
	window.__MODLOBBY_BOOT__ = { keptUpdate } as Boot
	navigation.mockReturnValue([
		{ type: 'navigate' } as unknown as PerformanceEntry,
	])
}

const app = () => (
	<AfterUpdate>
		<main>the app</main>
	</AfterUpdate>
)

describe('an ordinary start, told with the page what was kept', () => {
	test('draws the app at once and asks nothing', () => {
		handedOver(null)
		const { container } = render(app)

		expect(container.textContent).toBe('the app')
		expect(asked).not.toHaveBeenCalled()
	})

	test('holds the app back from the first draw while a kept update goes in', async () => {
		handedOver('0.1.34')
		let answer = (_outcome: null) => {}
		asked.mockReturnValue(new Promise((resolve) => (answer = resolve)))
		const { container } = render(app)

		expect(container.textContent).toBe('Installing version 0.1.34…')
		await settle()
		expect(asked.mock.calls.map(([command]) => command)).toEqual([
			'resume_update',
		])

		// It did not install after all: the manifest had moved on.
		answer(null)
		await settle()
		expect(container.textContent).toBe('the app')
	})
})

describe('a reloaded page, which has to ask', () => {
	test('draws nothing, asks once, and then draws the app', async () => {
		asked.mockResolvedValue(null)
		const { container } = render(app)
		expect(container.textContent).toBe('')

		await settle()
		expect(container.textContent).toBe('the app')
		expect(asked.mock.calls.map(([command]) => command)).toEqual([
			'kept_update',
		])
	})

	test('holds the app back and names the version going in', async () => {
		let answer = (_outcome: null) => {}
		asked.mockImplementation(async (command: string) => {
			if (command === 'kept_update') return '0.1.34'
			return new Promise((resolve) => (answer = resolve))
		})
		const { container } = render(app)

		await settle()
		expect(container.textContent).toBe('Installing version 0.1.34…')
		expect(asked.mock.calls.map(([command]) => command)).toEqual([
			'kept_update',
			'resume_update',
		])

		answer(null)
		await settle()
		expect(container.textContent).toBe('the app')
	})

	test('still opens the app when it cannot ask', async () => {
		asked.mockRejectedValue({ code: 'internal', message: 'no such command' })
		const { container } = render(app)

		await settle()
		expect(container.textContent).toBe('the app')
	})
})
