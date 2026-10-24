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
import type { ModSetView } from '../ipc/bindings/ModSetView'
import { applySettings } from '../store/settings'
import { blankSettings, SettingsView } from './Settings'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const asked = vi.mocked(invoke)

const AT = new Date(Date.now() - 2 * 3_600_000).toISOString()

function sphereAndTiny(): ModSetView {
	return {
		id: '0123456789abcdef',
		repos: ['dev/sphere', 'dev/tiny'],
		order: ['dev/tiny', 'dev/sphere'],
		at: AT,
		engine: [
			{ key: 'Fullscreen', before: '1', after: '0', plain: true },
			{ key: 'LogFile', before: null, after: 'x', plain: false },
		],
		widgets: [{ widget: 'Chat', on: true }],
		keybinds: {
			added: ['"active": "b"'],
			removed: ['"active": "a"'],
			more: 0,
		},
		hotkeys: null,
		blueprints: null,
	}
}

function open() {
	const history = createMemoryHistory()
	history.set({ value: '/?section=modded' })
	return render(() => (
		<MemoryRouter history={history}>
			<Route path='/' component={SettingsView} />
		</MemoryRouter>
	))
}

const section = (root: HTMLElement) =>
	root.querySelector<HTMLElement>('#settings-modded')!

function button(root: Element, text: string): HTMLButtonElement {
	const found = [...root.querySelectorAll('button')].find(
		(candidate) => candidate.textContent?.trim() === text,
	)
	if (!found) throw new Error(`no ${text} button`)
	return found
}

function checkbox(root: Element, label: string): HTMLInputElement {
	const found = [...root.querySelectorAll('label')].find((candidate) =>
		candidate.textContent?.includes(label),
	)
	const box = found?.querySelector<HTMLInputElement>('input[type=checkbox]')
	if (!box) throw new Error(`no ${label} checkbox`)
	return box
}

/** What the backend was asked, by command, with the arguments. */
const calls = (command: string) =>
	asked.mock.calls.filter(([name]) => name === command).map(([, args]) => args)

let sets: ModSetView[] = []

beforeAll(() => {
	Element.prototype.scrollIntoView = () => {}
})

beforeEach(() => {
	sets = []
	asked.mockReset()
	asked.mockImplementation(async (command: string) => {
		if (command === 'mod_settings') return sets
		if (command === 'fold_in_mod_settings') return []
		if (command === 'discard_mod_settings') return []
		return null
	})
	applySettings(blankSettings())
})

afterEach(() => cleanup())

describe('the modded games section', () => {
	test('nothing to show says so', async () => {
		const { container } = open()
		await vi.waitFor(() =>
			expect(section(container).textContent).toContain(
				'No modded game has left settings behind yet. Settings changed inside a modded game are kept with that set of mods and listed here.',
			),
		)
	})

	test('a set lists its changes, greys a text key, and folds the chosen kinds in', async () => {
		sets = [sphereAndTiny()]
		const { container } = open()
		await vi.waitFor(() =>
			expect(section(container).textContent).toContain('dev/sphere'),
		)
		const text = section(container).textContent ?? ''
		expect(text).toContain('loaded as dev/tiny, dev/sphere')
		expect(text).toContain('played 2h')
		expect(text).toContain('Fullscreen: 1 → 0')
		expect(text).toContain('Chat: off → on')
		expect(text).not.toContain('Chat.size')
		expect(text).toContain('+ "active": "b"')
		expect(text).toContain('− "active": "a"')
		expect(text).not.toContain('Hotkeys')
		const logFile = [...section(container).querySelectorAll('li')].find((li) =>
			li.textContent?.trim().startsWith('LogFile'),
		)
		expect(logFile?.classList.contains('by-hand')).toBe(true)
		expect(logFile?.textContent).toContain('set by hand')
		// Keybinds are not wanted this time; the rest fold in.
		fireEvent.click(checkbox(section(container), 'Keybinds'))
		fireEvent.click(button(section(container), 'Fold in'))
		await vi.waitFor(() =>
			expect(calls('fold_in_mod_settings')).toEqual([
				{
					id: '0123456789abcdef',
					kinds: {
						engine: true,
						widgets: true,
						keybinds: false,
						hotkeys: false,
						blueprints: false,
					},
				},
			]),
		)
		// Folded in, the list is what the backend says it is now: nothing.
		await vi.waitFor(() =>
			expect(section(container).textContent).toContain(
				'No modded game has left settings behind yet.',
			),
		)
	})

	test('discard asks for the set to be forgotten', async () => {
		sets = [sphereAndTiny()]
		const { container } = open()
		await vi.waitFor(() =>
			expect(section(container).textContent).toContain('dev/sphere'),
		)
		fireEvent.click(button(section(container), 'Discard'))
		await vi.waitFor(() =>
			expect(calls('discard_mod_settings')).toEqual([
				{ id: '0123456789abcdef' },
			]),
		)
	})
})
