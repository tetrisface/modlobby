import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Settings } from '../ipc/bindings/Settings'
import { newServer } from '../lib/servers'

const { login, loginWait, hasPassword, autoLogin, loginHolds, applySettings } =
	vi.hoisted(() => ({
		login: vi.fn(),
		loginWait: vi.fn(),
		hasPassword: vi.fn(),
		autoLogin: vi.fn(),
		loginHolds: vi.fn(),
		applySettings: vi.fn(),
	}))
vi.mock('../ipc/client', () => ({
	api: {
		login: (...args: unknown[]) => login(...args),
		loginWait: (...args: unknown[]) => loginWait(...args),
		hasPassword: (...args: unknown[]) => hasPassword(...args),
		autoLogin: () => autoLogin(),
		loginHolds: () => loginHolds(),
	},
	describeError: (error: unknown) => String(error),
}))
// What a login writes back is applied; nothing here reads it again. The
// lobby store, imported for which servers are up, reads `settings` lazily.
vi.mock('./settings', () => ({
	applySettings: (...args: unknown[]) => applySettings(...args),
	serverLabel: (id: string) => id,
	settings: () => null,
}))

/** Module state is per run; each test wants a run of its own. */
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
	autoLogin.mockReset()
	autoLogin.mockResolvedValue({ settings: {}, failures: [] })
	loginHolds.mockReset()
	loginHolds.mockResolvedValue({})
	applySettings.mockReset()
})

afterEach(() => vi.useRealTimers())

describe("the start's logins, which Rust makes", () => {
	test('are waited for, never made from here', async () => {
		const left = { account: { autoLogin: true } }
		autoLogin.mockResolvedValue({ settings: left, failures: [] })
		const session = await fresh()

		await session.autoLogin()

		expect(login).not.toHaveBeenCalled()
		// A login writes the account back; this is how the page learns of it.
		expect(applySettings).toHaveBeenCalledWith(left)
	})

	test('say what went wrong with any of them', async () => {
		autoLogin.mockResolvedValue({
			settings: {},
			failures: [{ server: BAR, message: 'wrong password' }],
		})
		const session = await fresh()
		const { chat } = await import('./chat')

		await session.autoLogin()

		expect(chat.notices.at(-1)).toMatchObject({
			level: 'warning',
			text: `could not log in to ${BAR}: wrong password`,
		})
	})

	test('are counted down to while one is held back', async () => {
		const until = Date.now() + 20_000
		loginHolds.mockResolvedValue({ [BAR]: until })
		let over = (_went: unknown) => {}
		autoLogin.mockReturnValue(new Promise((resolve) => (over = resolve)))
		const session = await fresh()

		const done = session.autoLogin()
		await settle()
		expect(session.loginHold()).toBe(until)

		over({ settings: {}, failures: [] })
		await done
		expect(session.loginHold()).toBeNull()
	})
})

describe('logging in without being asked, from the battle list', () => {
	test('goes to every server whose password is kept', async () => {
		const session = await fresh()

		expect(await session.loginUnattended(remembered())).toBe(true)

		// No password given: the keyring holds it and Rust is what reads it.
		expect(login).toHaveBeenCalledWith(BAR, 'me', null, true, true)
		expect(login).toHaveBeenCalledWith(RAPID, 'other', null, true, true)
	})

	test('does nothing without a remembered password or an account', async () => {
		const session = await fresh()

		await session.loginUnattended(remembered({ rememberPassword: false }))
		await session.loginUnattended(remembered({ autoLogin: false }))
		await session.loginUnattended(
			remembered({}, [{ ...newServer(BAR), username: '   ' }]),
		)

		expect(login).not.toHaveBeenCalled()
	})

	test("follows a server's own answer over the account's", async () => {
		const session = await fresh()
		await session.loginUnattended(
			remembered({ autoLogin: false }, [
				{ ...newServer(BAR), username: 'me', autoLogin: true },
				{ ...newServer(RAPID), username: 'other' },
			]),
		)
		expect(login).toHaveBeenCalledTimes(1)
		// Sent back off: this server's answer is not the account's to change.
		expect(login).toHaveBeenCalledWith(BAR, 'me', null, true, false)

		await session.loginUnattended(
			remembered({}, [{ ...newServer(BAR), username: 'me', autoLogin: false }]),
		)
		expect(login).toHaveBeenCalledTimes(1)
	})

	test('skips a server whose password is not kept', async () => {
		hasPassword.mockImplementation(async (server: string) => server === RAPID)
		const session = await fresh()

		await session.loginUnattended(remembered())

		expect(login).toHaveBeenCalledTimes(1)
		expect(login).toHaveBeenCalledWith(RAPID, 'other', null, true, true)
	})

	test('waits out the throttle instead of failing the login', async () => {
		vi.useFakeTimers()
		loginWait.mockResolvedValue(20)
		const session = await fresh()

		const done = session.loginUnattended(
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

	test('says at once when nowhere can go in without asking', async () => {
		hasPassword.mockResolvedValue(false)
		const session = await fresh()

		expect(await session.loginUnattended(remembered())).toBe(false)
		expect(login).not.toHaveBeenCalled()

		hasPassword.mockImplementation(async (server: string) => server === BAR)
		expect(await session.loginUnattended(remembered())).toBe(true)
		expect(login).toHaveBeenCalledTimes(1)
		expect(login).toHaveBeenCalledWith(BAR, 'me', null, true, true)
	})
})
