import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Settings } from '../ipc/bindings/Settings'
import { newServer } from '../lib/servers'

const { login, loginWait, hasPassword } = vi.hoisted(() => ({
	login: vi.fn(),
	loginWait: vi.fn(),
	hasPassword: vi.fn(),
}))
vi.mock('../ipc/client', () => ({
	api: {
		login: (...args: unknown[]) => login(...args),
		loginWait: (...args: unknown[]) => loginWait(...args),
		hasPassword: (...args: unknown[]) => hasPassword(...args),
	},
	describeError: (error: unknown) => String(error),
}))
// What a login writes back is applied; nothing here reads it again.
vi.mock('./settings', () => ({ applySettings: vi.fn() }))

/** The once-per-run flag is module state; each test wants a run of its own. */
async function fresh() {
	vi.resetModules()
	return import('./session')
}

/** Lets an awaited answer reach the code under test. */
async function settle() {
	for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

const BAR = 'server4.beyondallreason.info'
const RAPID = 'server.example.com'

/** Settings with the given flags and servers; the rest is never read here. */
function remembered(
	over: Partial<Settings['account']> = {},
	servers = [
		{ ...newServer(BAR), username: 'me' },
		{ ...newServer(RAPID), username: 'other' },
	],
): Settings {
	return {
		account: { rememberPassword: true, autoLogin: true, ...over },
		servers,
	} as unknown as Settings
}

beforeEach(() => {
	login.mockReset()
	login.mockResolvedValue({})
	loginWait.mockReset()
	loginWait.mockResolvedValue(0)
	hasPassword.mockReset()
	hasPassword.mockResolvedValue(true)
})

afterEach(() => vi.useRealTimers())

describe('logging back in on startup', () => {
	test('logs in once per run, not once per mount', async () => {
		const session = await fresh()
		const one = remembered({}, [{ ...newServer(BAR), username: 'me' }])

		await session.autoLogin(one)
		await session.autoLogin(one)

		expect(login).toHaveBeenCalledTimes(1)
		// No password given: the keyring holds it and Rust is what reads it.
		expect(login).toHaveBeenCalledWith(BAR, 'me', null, true, true)
	})

	test('does nothing without a remembered password or an account', async () => {
		const session = await fresh()
		await session.autoLogin(remembered({ rememberPassword: false }))
		expect(login).not.toHaveBeenCalled()

		const again = await fresh()
		await again.autoLogin(remembered({ autoLogin: false }))
		expect(login).not.toHaveBeenCalled()

		const nobody = await fresh()
		await nobody.autoLogin(
			remembered({}, [{ ...newServer(BAR), username: '   ' }]),
		)
		expect(login).not.toHaveBeenCalled()
	})

	test('goes to every server whose password is kept', async () => {
		const session = await fresh()

		await session.autoLogin(remembered())

		expect(login).toHaveBeenCalledWith(BAR, 'me', null, true, true)
		expect(login).toHaveBeenCalledWith(RAPID, 'other', null, true, true)
	})

	test('skips a server whose password is not kept', async () => {
		hasPassword.mockImplementation(async (server: string) => server === RAPID)
		const session = await fresh()

		await session.autoLogin(remembered())

		expect(login).toHaveBeenCalledTimes(1)
		expect(login).toHaveBeenCalledWith(RAPID, 'other', null, true, true)
	})

	test('waits out the throttle instead of failing the login', async () => {
		vi.useFakeTimers()
		loginWait.mockResolvedValue(20)
		const session = await fresh()

		const done = session.autoLogin(
			remembered({}, [{ ...newServer(BAR), username: 'me' }]),
		)
		await settle()
		expect(session.loginHold()).not.toBeNull()
		await vi.advanceTimersByTimeAsync(20_000)
		expect(login).not.toHaveBeenCalled()

		// A second past the server's own count, so the allowance has lapsed.
		await vi.advanceTimersByTimeAsync(1_000)
		await done

		expect(login).toHaveBeenCalledTimes(1)
		expect(loginWait).toHaveBeenCalledWith(BAR)
		expect(session.loginHold()).toBeNull()
	})

	test('a refused login is said once and not tried again', async () => {
		login.mockRejectedValue({ code: 'refused', message: 'no' })
		const session = await fresh()

		const one = remembered({}, [{ ...newServer(BAR), username: 'me' }])
		await session.autoLogin(one)
		await session.autoLogin(one)

		expect(login).toHaveBeenCalledTimes(1)
	})
})
