import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { reconcile } from 'solid-js/store'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Settings } from '../ipc/bindings/Settings'
import { newServer } from '../lib/servers'
import { emptyLobby, setLobby } from '../store/lobby'
import { setSettingsSignal } from '../store/settings'
import { LoginForm } from './LoginForm'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const SERVER = 'server4.beyondallreason.info'
const Login = () => <LoginForm server={SERVER} />

const asked = vi.mocked(invoke)

/** What teiserver sends, trailing empty line and all. */
const AGREEMENT = [
	'A verification code has been sent to your email address. Please read our terms of service at https://www.beyondallreason.info/privacy',
	'',
]

/** Answers every command the form reaches for; overridable per test. */
function serve(answers: Record<string, unknown> = {}) {
	asked.mockImplementation(async (command: string) => {
		if (command in answers) {
			const answer = answers[command]
			if (answer instanceof Error) throw answer
			return answer
		}
		switch (command) {
			case 'login_wait':
				return 0
			case 'has_password':
				return false
			case 'name_problem':
				return null
			case 'register':
				return AGREEMENT
			case 'login':
			case 'confirm_agreement':
				return settingsWith()
			default:
				throw new Error(`unexpected ${command}`)
		}
	})
}

function fill(container: HTMLElement, selector: string, value: string) {
	const field = container.querySelector<HTMLInputElement>(selector)
	if (!field) throw new Error(`no ${selector}`)
	fireEvent.input(field, { target: { value } })
}

function send(container: HTMLElement) {
	fireEvent.submit(container.querySelector('form')!)
}

/** Switches the form between logging in and creating an account. */
function flip(container: HTMLElement) {
	fireEvent.click(container.querySelector('.login-switch button')!)
}

/** Only what the form reads: the server it is for, and the two flags. */
function settingsWith(): Settings {
	return {
		servers: [newServer(SERVER)],
		account: { rememberPassword: false, autoLogin: false },
		chat: { muted: [] },
	} as unknown as Settings
}

beforeEach(() => {
	asked.mockReset()
	serve()
	setSettingsSignal(settingsWith())
})

afterEach(() => {
	cleanup()
	setLobby(reconcile(emptyLobby()))
	setSettingsSignal(null)
})

describe('the login form', () => {
	test("names the server only when it is not BAR's", () => {
		const { container } = render(() => <Login />)
		expect(container.textContent).not.toContain('Server:')
		expect(container.textContent).toContain('Forgot the password?')
		cleanup()

		setSettingsSignal({
			...settingsWith(),
			servers: [{ ...newServer('rapid.example'), name: 'Rapid' }],
		} as Settings)
		const other = render(() => <LoginForm server='rapid.example' />)
		expect(other.container.textContent).toContain('Server: Rapid')
	})

	test('on the login page, asks whether passwords are kept, starting from the one answer', () => {
		const kept = settingsWith()
		kept.account = { rememberPassword: true, autoLogin: false }
		setSettingsSignal(kept)
		const { container } = render(() => <LoginForm server={SERVER} asksFlags />)
		const boxes = [
			...container.querySelectorAll<HTMLInputElement>('input[type=checkbox]'),
		]
		expect(boxes.map((box) => box.checked)).toEqual([true, false])
	})

	test('over Settings, follows the one answer without asking', async () => {
		const kept = settingsWith()
		kept.account = { rememberPassword: true, autoLogin: true }
		setSettingsSignal(kept)
		const { container } = render(() => <Login />)
		expect(container.querySelector('input[type=checkbox]')).toBeNull()

		fill(container, 'input[autocomplete="username"]', 'me')
		fill(container, 'input[autocomplete="current-password"]', 'pw')
		send(container)
		await vi.waitFor(() =>
			expect(asked).toHaveBeenCalledWith('login', {
				server: SERVER,
				username: 'me',
				password: 'pw',
				remember: true,
				autoLogin: true,
			}),
		)
	})

	test('the way to register sits under the button, and the error above it', () => {
		const { container } = render(() => <Login />)
		const children = [...container.querySelector('form')!.children]
		const button = children.findIndex((el) => el.matches('button[type=submit]'))
		const switcher = children.findIndex((el) => el.matches('.login-switch'))

		expect(switcher).toBeGreaterThan(button)
		expect(container.querySelector('.login-switch')?.textContent).toContain(
			"Don't have an account?",
		)
		expect(container.querySelector('.login-switch button')?.textContent).toBe(
			'Register',
		)
	})

	test('the register form offers new-password, the login form current-password', () => {
		const { container } = render(() => <Login />)
		expect(
			container.querySelector('input[autocomplete="current-password"]'),
		).not.toBeNull()

		flip(container)

		expect(
			container.querySelector('input[autocomplete="new-password"]'),
		).not.toBeNull()
		expect(container.querySelector('input[type="email"]')).not.toBeNull()
	})

	test('the password can be revealed instead of typed a second time', () => {
		const { container } = render(() => <Login />)
		const field = () =>
			container.querySelector<HTMLInputElement>(
				'input[autocomplete$="-password"]',
			)
		const toggle = () =>
			container.querySelector('.password-field .password-reveal')!
		expect(field()?.type).toBe('password')
		expect(toggle().getAttribute('aria-label')).toBe('Show password')

		fireEvent.click(toggle())
		expect(field()?.type).toBe('text')
		expect(toggle().getAttribute('aria-label')).toBe('Hide password')

		fireEvent.click(toggle())
		expect(field()?.type).toBe('password')
	})

	test('a username the rules refuse is said without asking the server', async () => {
		serve({ name_problem: 'only a-z, A-Z, 0-9, [, ] and _ are allowed' })
		const { container } = render(() => <Login />)
		flip(container)
		fill(container, 'input[autocomplete="username"]', 'no spaces')
		fireEvent.blur(container.querySelector('input[autocomplete="username"]')!)

		await waitFor(() =>
			expect(container.querySelector('.error')?.textContent).toContain(
				'only a-z',
			),
		)
		// And the button will not spend a login on a name that cannot work.
		expect(
			container.querySelector<HTMLButtonElement>('button[type=submit]')
				?.disabled,
		).toBe(true)
		expect(asked).not.toHaveBeenCalledWith('register', expect.anything())
	})

	test("registering shows the server's own agreement text with its links", async () => {
		const { container } = render(() => <Login />)
		flip(container)
		fill(container, 'input[autocomplete="username"]', 'me')
		fill(container, 'input[autocomplete="new-password"]', 'pw')
		fill(container, 'input[type="email"]', 'a@b.c')
		send(container)

		await waitFor(() =>
			expect(
				container.querySelector('input[autocomplete="one-time-code"]'),
			).not.toBeNull(),
		)
		expect(asked).toHaveBeenCalledWith('register', {
			server: SERVER,
			username: 'me',
			password: 'pw',
			email: 'a@b.c',
		})
		expect(container.textContent).toContain('A verification code has been sent')
		// The address in that text is a link, not something to retype.
		expect(container.querySelector('a.chat-link')?.getAttribute('href')).toBe(
			'https://www.beyondallreason.info/privacy',
		)
		// The empty line teiserver always sends is not a paragraph.
		expect(container.querySelectorAll('p.muted a.chat-link')).toHaveLength(1)
	})

	test("a wrong code leaves the form on the code field with the server's reason", async () => {
		const { container } = render(() => <Login />)
		flip(container)
		fill(container, 'input[autocomplete="username"]', 'me')
		fill(container, 'input[autocomplete="new-password"]', 'pw')
		fill(container, 'input[type="email"]', 'a@b.c')
		send(container)
		await waitFor(() =>
			expect(
				container.querySelector('input[autocomplete="one-time-code"]'),
			).not.toBeNull(),
		)

		serve({
			confirm_agreement: Object.assign(new Error('Incorrect code'), {
				code: 'refused',
				message: 'Incorrect code',
			}),
		})
		fill(container, 'input[autocomplete="one-time-code"]', 'nope')
		send(container)

		await waitFor(() =>
			expect(container.querySelector('.error')?.textContent).toBe(
				'Incorrect code',
			),
		)
		// Still on the code, so the next attempt is a retype and not a re-register.
		expect(
			container.querySelector('input[autocomplete="one-time-code"]'),
		).not.toBeNull()
	})
})
