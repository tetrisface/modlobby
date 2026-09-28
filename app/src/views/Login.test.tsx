import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { Settings } from '../ipc/bindings/Settings'
import { newServer } from '../lib/servers'
import { setSettingsSignal } from '../store/settings'
import { Login } from './Login'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const address = vi.hoisted(() => ({ params: {} as Record<string, string> }))
vi.mock('@solidjs/router', () => ({
	useNavigate: () => vi.fn(),
	useSearchParams: () => [address.params],
}))
const asked = vi.mocked(invoke)

const BAR = 'server4.beyondallreason.info'

function withServers(...hosts: string[]): Settings {
	return {
		// The first is BAR's own, as every install's list starts.
		servers: hosts.map((host, at) => ({
			...newServer(host),
			builtin: at === 0 ? 'bar' : null,
		})),
		account: { rememberPassword: false, autoLogin: false },
		chat: {},
	} as unknown as Settings
}

const serverLine = (root: HTMLElement) =>
	[...root.querySelectorAll('p.muted')].find((p) =>
		p.textContent?.startsWith('Server:'),
	)?.textContent

const chips = (root: HTMLElement) => [
	...root.querySelectorAll<HTMLButtonElement>('.login-servers .chip-choice'),
]

const chip = (root: HTMLElement, name: string) =>
	chips(root).find((held) => held.textContent === name)!

beforeEach(() => {
	asked.mockReset()
	asked.mockImplementation(async (command: string) =>
		command === 'login_wait' ? 0 : null,
	)
})

afterEach(() => {
	cleanup()
	setSettingsSignal(null)
	address.params = {}
})

describe('the login page', () => {
	test('with one server there is nothing to choose', () => {
		setSettingsSignal(withServers(BAR))
		const { container } = render(() => <Login />)
		expect(chips(container)).toHaveLength(0)
		// BAR is the one everybody means, so the form does not name it.
		expect(serverLine(container)).toBeUndefined()
	})

	test('with more, each is a chip, and the form is for the one pressed', () => {
		setSettingsSignal(withServers(BAR, 'server.example.com'))
		const { container } = render(() => <Login />)
		expect(chips(container).map((held) => held.textContent)).toEqual([
			BAR,
			'server.example.com',
		])
		expect(chip(container, BAR).getAttribute('aria-pressed')).toBe('true')
		expect(serverLine(container)).toBeUndefined()

		fireEvent.click(chip(container, 'server.example.com'))
		expect(serverLine(container)).toContain('server.example.com')
		expect(chip(container, BAR).getAttribute('aria-pressed')).toBe('false')
	})

	test('past ten, the rest fold into a dropdown', () => {
		const more = Array.from({ length: 10 }, (_, at) => `s${at}.test`)
		setSettingsSignal(withServers(BAR, ...more))
		const { container } = render(() => <Login />)
		expect(chips(container)).toHaveLength(10)
		const rest = container.querySelector('select')!
		expect([...rest.options].map((option) => option.value)).toEqual([
			'',
			's9.test',
		])

		fireEvent.change(rest, { target: { value: 's9.test' } })
		expect(serverLine(container)).toContain('s9.test')
		expect(rest.value).toBe('s9.test')
	})

	test('a link that names a server gets that one, until one is picked', () => {
		address.params = { server: 'server.example.com' }
		setSettingsSignal(withServers(BAR, 'server.example.com'))
		const { container } = render(() => <Login />)
		expect(serverLine(container)).toContain('server.example.com')

		fireEvent.click(chip(container, BAR))
		expect(serverLine(container)).toBeUndefined()
	})

	test('servers set to log in at startup are ticked, and go in with the login', async () => {
		const kept = withServers(BAR, 'server.example.com', 'quiet.test')
		kept.account = { rememberPassword: true, autoLogin: true }
		kept.servers[1]!.username = 'other'
		// Set to log in at startup, but with no account to do it as.
		kept.servers[2]!.autoLogin = true
		setSettingsSignal(kept)
		asked.mockImplementation(async (command: string) =>
			command === 'login_wait' ? 0 : command === 'login' ? kept : null,
		)
		const { container } = render(() => <Login />)
		expect(
			chips(container)
				.filter((held) => held.classList.contains('also'))
				.map((held) => held.textContent),
		).toEqual(['server.example.com'])

		fireEvent.input(
			container.querySelector('input[autocomplete="username"]')!,
			{
				target: { value: 'me' },
			},
		)
		fireEvent.input(
			container.querySelector('input[autocomplete="current-password"]')!,
			{ target: { value: 'pw' } },
		)
		fireEvent.submit(container.querySelector('form')!)
		await vi.waitFor(() => {
			expect(asked).toHaveBeenCalledWith('login', {
				server: BAR,
				username: 'me',
				password: 'pw',
				remember: true,
				autoLogin: true,
			})
			expect(asked).toHaveBeenCalledWith('login', {
				server: 'server.example.com',
				username: 'other',
				password: null,
				remember: true,
				autoLogin: true,
			})
		})
		expect(
			asked.mock.calls.filter(([command]) => command === 'login'),
		).toHaveLength(2)
	})

	test('the picked server never goes in twice', () => {
		const kept = withServers(BAR, 'server.example.com')
		kept.account = { rememberPassword: true, autoLogin: true }
		kept.servers[0]!.username = 'me'
		kept.servers[1]!.username = 'other'
		setSettingsSignal(kept)
		const { container } = render(() => <Login />)
		expect(chip(container, BAR).classList.contains('also')).toBe(false)
		expect(
			chip(container, 'server.example.com').classList.contains('also'),
		).toBe(true)

		fireEvent.click(chip(container, 'server.example.com'))
		expect(chip(container, BAR).classList.contains('also')).toBe(true)
		expect(
			chip(container, 'server.example.com').classList.contains('also'),
		).toBe(false)
	})

	test('with none, says where one is added', () => {
		setSettingsSignal(withServers())
		const { container } = render(() => <Login />)
		expect(container.textContent).toContain('Add one in Settings')
	})
})
