import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import {
	afterEach,
	beforeAll,
	beforeEach,
	describe,
	expect,
	test,
	vi,
} from 'vitest'
import type { Settings } from '../ipc/bindings/Settings'
import { newServer } from '../lib/servers'
import { applySettings } from '../store/settings'
import { blankSettings, SettingsView } from './Settings'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const asked = vi.mocked(invoke)

const BAR = 'server4.beyondallreason.info'

function loaded(): Settings {
	return {
		...blankSettings(),
		servers: [{ ...newServer(BAR), name: 'BAR', username: 'me' }],
	}
}

function open() {
	const history = createMemoryHistory()
	history.set({ value: '/?section=servers' })
	return render(() => (
		<MemoryRouter history={history}>
			<Route path='/' component={SettingsView} />
		</MemoryRouter>
	))
}

const cards = (root: HTMLElement) => [...root.querySelectorAll('.server-card')]

function button(root: Element, text: string): HTMLButtonElement {
	const found = [...root.querySelectorAll('button')].find(
		(candidate) => candidate.textContent?.trim() === text,
	)
	if (!found) throw new Error(`no ${text} button`)
	return found
}

function typeHost(root: HTMLElement, host: string) {
	const field = root.querySelector<HTMLInputElement>('.server-add input')
	if (!field) throw new Error('no host field')
	fireEvent.input(field, { target: { value: host } })
}

beforeAll(() => {
	Element.prototype.scrollIntoView = () => {}
})

beforeEach(() => {
	asked.mockReset()
	asked.mockImplementation(async (command: string) => {
		if (command === 'login_wait') return 0
		if (command === 'has_password') return false
		return null
	})
	applySettings(loaded())
})

afterEach(() => cleanup())

describe('the servers section', () => {
	test('lists each server with where its session stands', () => {
		const { container } = open()
		expect(cards(container)).toHaveLength(1)
		expect(container.querySelector('.server-name')).toHaveProperty(
			'value',
			'BAR',
		)
		expect(cards(container)[0]?.textContent).toContain('not connected')
		expect(cards(container)[0]?.textContent).toContain('account me')
	})

	test('adds a server by its host, and only once', () => {
		const { container } = open()
		typeHost(container, 'server.example.com')
		fireEvent.click(button(container, 'Add'))
		expect(cards(container)).toHaveLength(2)
		expect(cards(container)[1]?.textContent).toContain('server.example.com')

		typeHost(container, 'server.example.com')
		expect(button(container, 'Add').disabled).toBe(true)
		expect(container.querySelector('.server-add + .error')?.textContent).toBe(
			'that server is already listed',
		)
	})

	test('a server just added is saved before its login form opens', async () => {
		asked.mockImplementation(async (command: string, args?: unknown) => {
			if (command === 'update_settings')
				return structuredClone((args as { settings: Settings }).settings)
			if (command === 'login_wait') return 0
			if (command === 'has_password') return false
			return null
		})
		const { container } = open()
		typeHost(container, 'rapid.example')
		fireEvent.click(button(container, 'Add'))
		fireEvent.click(button(cards(container)[1]!, 'Log in'))
		await vi.waitFor(() =>
			expect(document.querySelector('.login-sheet')).not.toBeNull(),
		)
		const saved = asked.mock.calls.find(
			([command]) => command === 'update_settings',
		)
		expect(
			(saved?.[1] as { settings: Settings }).settings.servers.map(
				(entry) => entry.host,
			),
		).toEqual([BAR, 'rapid.example'])
	})

	test('a save leaves the card being typed in where it is', async () => {
		asked.mockImplementation(async (command: string, args?: unknown) => {
			if (command === 'update_settings')
				return structuredClone((args as { settings: Settings }).settings)
			return null
		})
		const { container } = open()
		const name = container.querySelector<HTMLInputElement>('.server-name')!
		fireEvent.input(name, { target: { value: 'BAR main' } })
		await vi.waitFor(
			() =>
				expect(asked).toHaveBeenCalledWith(
					'update_settings',
					expect.anything(),
				),
			{ timeout: 2000 },
		)
		await new Promise((settled) => setTimeout(settled, 50))
		expect(name.isConnected).toBe(true)
	})

	test('ports that are not ports say so and are not saved', () => {
		const { container } = open()
		const card = cards(container)[0]!
		const ports = card.querySelector<HTMLInputElement>(
			'.server-connection input',
		)!
		fireEvent.input(ports, { target: { value: '8200, tls' } })
		expect(card.querySelector('.server-connection .error')).not.toBeNull()
		fireEvent.input(ports, { target: { value: '8200' } })
		expect(card.querySelector('.server-connection .error')).toBeNull()
	})

	test('removing a server forgets what was kept for it', async () => {
		const { container } = open()
		const card = cards(container)[0]!
		fireEvent.click(button(card, 'Remove'))
		fireEvent.click(button(card, 'Really remove?'))
		await vi.waitFor(() => expect(cards(container)).toHaveLength(0))
		expect(asked).toHaveBeenCalledWith('clear_password', {
			server: BAR,
			username: 'me',
		})
		expect(asked).toHaveBeenCalledWith('forget_way', { host: BAR })
	})

	test('a server added is looked at for a rapid server of its own', async () => {
		asked.mockImplementation(async (command: string, args?: unknown) => {
			if (command === 'check_rapid') {
				const { url } = args as { url: string }
				if (url === 'https://mods.example/repos.gz') return { own: 2, bars: 2 }
				throw { code: 'input', message: `${url} is not a rapid index` }
			}
			return null
		})
		const { container } = open()
		const rapidOf = (card: Element) =>
			[
				...card.querySelectorAll<HTMLInputElement>('.server-connection input'),
			].at(-1)!

		typeHost(container, 'mods.example')
		fireEvent.click(button(container, 'Add'))
		await vi.waitFor(() =>
			expect(rapidOf(cards(container)[1]!).value).toBe(
				'https://mods.example/repos.gz',
			),
		)

		// A guess that is wrong leaves the server on BAR's games.
		typeHost(container, 'stock.example')
		fireEvent.click(button(container, 'Add'))
		await vi.waitFor(() =>
			expect(asked).toHaveBeenCalledWith('check_rapid', {
				url: 'https://stock.example/repos.gz',
			}),
		)
		expect(rapidOf(cards(container)[2]!).value).toBe('')
		expect(cards(container)[2]!.textContent).toContain("looked for in BAR's")
	})

	test('a rapid address that is refused says why, and that nothing comes from it', async () => {
		asked.mockImplementation(async (command: string) => {
			if (command === 'check_rapid')
				throw {
					code: 'input',
					message: 'http://mods.example/repos.gz is not served over https',
				}
			return null
		})
		const { container } = open()
		const card = cards(container)[0]!
		const field = [
			...card.querySelectorAll<HTMLInputElement>('.server-connection input'),
		].at(-1)!
		fireEvent.input(field, {
			target: { value: 'http://mods.example/repos.gz' },
		})
		await vi.waitFor(
			() =>
				expect(
					card.querySelector('.server-connection .error')?.textContent,
				).toBe(
					'http://mods.example/repos.gz is not served over https. Nothing is fetched from it.',
				),
			{ timeout: 3000 },
		)
	})

	test('removing takes a second click', () => {
		const { container } = open()
		const card = cards(container)[0]!
		fireEvent.click(button(card, 'Remove'))
		expect(cards(container)).toHaveLength(1)
		fireEvent.click(button(card, 'Really remove?'))
		return vi.waitFor(() => expect(cards(container)).toHaveLength(0))
	})

	test("reset password opens the server's own page", async () => {
		const { container } = open()
		fireEvent.click(button(cards(container)[0]!, 'Reset password'))
		await vi.waitFor(() =>
			expect(asked).toHaveBeenCalledWith('open_url', {
				url: `https://${BAR}/forgot_password`,
			}),
		)
	})

	test('log in opens the form for that server over the page', async () => {
		const { container } = open()
		fireEvent.click(button(cards(container)[0]!, 'Log in'))
		const sheet = await vi.waitFor(() => {
			const found = document.querySelector('.login-sheet')
			expect(found).not.toBeNull()
			return found
		})
		expect(sheet?.textContent).toContain('Forgot the password?')
		await vi.waitFor(() =>
			expect(asked).toHaveBeenCalledWith('login_wait', { server: BAR }),
		)
		fireEvent.click(button(sheet!, 'Close'))
		expect(document.querySelector('.login-sheet')).toBeNull()
	})
})
