import { describe, expect, test, vi } from 'vitest'

// `keepTrying` lives beside the alerts, which reach for the notification
// plugin at import time; nothing here calls it, but it has to resolve.
vi.mock('@tauri-apps/plugin-notification', () => ({
	isPermissionGranted: vi.fn(async () => false),
	requestPermission: vi.fn(async () => 'denied'),
	sendNotification: vi.fn(),
}))

vi.mock('@tauri-apps/api/window', () => ({
	UserAttentionType: { Critical: 1, Informational: 2 },
	getCurrentWindow: () => ({ requestUserAttention: vi.fn(async () => {}) }),
}))

const { launching, watchLaunch } = await import('./launch')
const { setLobby } = await import('./lobby')

describe('waiting for the engine window', () => {
	test('running without a window is launching; seen once, it is not', async () => {
		const answers = [false, false, true]
		const hasWindow = vi.fn(async () => answers.shift() ?? true)
		const stop = watchLaunch(hasWindow, 1000, 0, 0)
		try {
			expect(launching()).toBe(false)
			setLobby('engine', { state: 'running', pid: 7 })
			expect(launching()).toBe(true)
			await vi.waitFor(() => expect(launching()).toBe(false))
			expect(hasWindow).toHaveBeenCalledTimes(3)

			// Another game is another wait.
			setLobby('engine', { state: 'exited', code: 0 })
			expect(launching()).toBe(false)
			setLobby('engine', { state: 'running', pid: 8 })
			expect(launching()).toBe(true)
			await vi.waitFor(() => expect(launching()).toBe(false))
		} finally {
			stop()
			setLobby('engine', { state: 'idle' })
		}
	})

	test('the wait ends when the window never comes', async () => {
		// No settle on this path: the card has waited the whole deadline out.
		const stop = watchLaunch(async () => false, 0, 0, 10_000)
		try {
			setLobby('engine', { state: 'running', pid: 9 })
			await vi.waitFor(() => expect(launching()).toBe(false))
		} finally {
			stop()
			setLobby('engine', { state: 'idle' })
		}
	})

	/**
	 * The window handle beats the first frame on screen, so a found window
	 * holds the card a moment longer rather than ending it on the spot.
	 */
	test('a found window still settles before the card stops waiting', async () => {
		const stop = watchLaunch(async () => true, 1000, 0, 50)
		try {
			setLobby('engine', { state: 'running', pid: 11 })
			expect(launching()).toBe(true)
			// Long enough for the poll to answer, short of the settle.
			await new Promise((resolve) => setTimeout(resolve, 20))
			expect(launching()).toBe(true)
			await vi.waitFor(() => expect(launching()).toBe(false))
		} finally {
			stop()
			setLobby('engine', { state: 'idle' })
		}
	})

	test('a reaped child has no window to wait for', () => {
		setLobby('engine', { state: 'running', pid: null })
		expect(launching()).toBe(false)
		setLobby('engine', { state: 'idle' })
	})
})
