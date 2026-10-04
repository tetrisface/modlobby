import { cleanup, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, expect, test, vi } from 'vitest'
import { AfterUpdate } from './AfterUpdate'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const asked = vi.mocked(invoke)

afterEach(() => {
	cleanup()
	asked.mockReset()
})

/** Lets the awaits between a mount and what it draws run. */
async function settle() {
	for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

const app = () => (
	<AfterUpdate>
		<main>the app</main>
	</AfterUpdate>
)

test('an ordinary start draws nothing, asks once, and then draws the app', async () => {
	asked.mockResolvedValue(null)
	const { container } = render(app)
	expect(container.textContent).toBe('')

	await settle()
	expect(container.textContent).toBe('the app')
	expect(asked.mock.calls.map(([command]) => command)).toEqual(['kept_update'])
})

test('a kept update holds the app back and names the version going in', async () => {
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

	// It did not install after all: the manifest had moved on.
	answer(null)
	await settle()
	expect(container.textContent).toBe('the app')
})

test('a start that cannot ask still opens the app', async () => {
	asked.mockRejectedValue({ code: 'internal', message: 'no such command' })
	const { container } = render(app)

	await settle()
	expect(container.textContent).toBe('the app')
})
