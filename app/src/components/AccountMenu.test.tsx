import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { newServer } from '../lib/servers'
import { emptyServer } from '../store/lobby'
import { applySettings } from '../store/settings'
import { onlySession, seedSession } from '../store/testing'
import { blankSettings } from '../views/Settings'
import { AccountMenu } from './AccountMenu'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const asked = vi.mocked(invoke)

const BAR = 'server4.beyondallreason.info'
const RAPID = 'rapid.example'

function button(root: ParentNode, text: string): HTMLButtonElement {
	const found = [...root.querySelectorAll('button')].find(
		(candidate) => candidate.textContent?.trim() === text,
	)
	if (!found) throw new Error(`no ${text} button`)
	return found
}

function open() {
	const rendered = render(() => <AccountMenu name='me' />)
	fireEvent.click(button(rendered.container, 'me'))
	return rendered
}

const rows = (root: HTMLElement) =>
	[...root.querySelectorAll('.account-server')].map((row) =>
		[...row.children].map((cell) => cell.textContent?.trim()),
	)

beforeEach(() => {
	asked.mockReset()
	asked.mockImplementation(async (command: string) => {
		if (command === 'login_wait') return 0
		if (command === 'has_password') return false
		return null
	})
	applySettings({
		...blankSettings(),
		servers: [
			{ ...newServer(BAR), name: 'BAR', username: 'me' },
			{ ...newServer(RAPID), username: 'me' },
			newServer('fresh.example'),
		],
	})
	onlySession({ ...emptyServer(), phase: 'ready', me: 'me' }, BAR)
})

afterEach(() => cleanup())

describe('the account menu', () => {
	test('lists every server, how it stands, and a way back into each that is down', () => {
		const { container } = open()
		expect(rows(container)).toEqual([
			['BAR', 'logged in as me', 'Log out'],
			[RAPID, 'not connected', 'Reconnect'],
			['fresh.example', 'not connected', 'Log in'],
		])
		// Down because it was never logged in to, or was logged out of: as asked.
		expect(container.querySelector('.account-name.partial')).toBeNull()
	})

	test('marks the name only for a server that dropped and is being retried', () => {
		seedSession({ ...emptyServer(), retryAt: Date.now() + 30_000 }, RAPID)
		const { container } = render(() => <AccountMenu name='me' />)
		expect(container.querySelector('.account-name.partial')).not.toBeNull()
	})

	test('logs out of just the server asked for', () => {
		const { container } = open()
		fireEvent.click(button(container, 'Log out'))
		expect(asked).toHaveBeenCalledWith('logout', { server: BAR })
	})

	test('reconnects just the server asked for', () => {
		const { container } = open()
		fireEvent.click(button(container, 'Reconnect'))
		expect(asked).toHaveBeenCalledWith('reconnect', { server: RAPID })
	})

	test('asks for a login where there is nothing to reconnect with', async () => {
		asked.mockImplementation(async (command: string) => {
			if (command === 'reconnect')
				throw { code: 'noCredentials', message: 'not logged in' }
			if (command === 'login_wait') return 0
			return false
		})
		const { container } = open()
		fireEvent.click(button(container, 'Reconnect'))
		await vi.waitFor(() =>
			expect(document.querySelector('.login-sheet')).not.toBeNull(),
		)
		expect(container.querySelector('.account-menu')).toBeNull()
	})

	test('closes on Escape', () => {
		const { container } = open()
		fireEvent.keyDown(document, { key: 'Escape' })
		expect(container.querySelector('.account-menu')).toBeNull()
	})
})
