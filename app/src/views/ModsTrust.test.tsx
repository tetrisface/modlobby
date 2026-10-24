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
import { applySettings } from '../store/settings'
import { blankSettings, SettingsView } from './Settings'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const asked = vi.mocked(invoke)

const SHA = '9108a17078f79d09925edc305ec83bc06c3a7cb3'
const SINCE = '2026-10-22T10:00:00Z'

function open() {
	const history = createMemoryHistory()
	history.set({ value: '/?section=mods' })
	return render(() => (
		<MemoryRouter history={history}>
			<Route path='/' component={SettingsView} />
		</MemoryRouter>
	))
}

const section = (root: HTMLElement) =>
	root.querySelector<HTMLElement>('#settings-mods')!

const rows = (root: HTMLElement) => [
	...section(root).querySelectorAll<HTMLElement>('.mod-trust'),
]

function button(root: Element, text: string): HTMLButtonElement {
	const found = [...root.querySelectorAll('button')].find(
		(candidate) => candidate.textContent?.trim() === text,
	)
	if (!found) throw new Error(`no ${text} button`)
	return found
}

/** The mods settings as the last save sent them. */
function saved(): Settings['mods'] {
	const calls = asked.mock.calls.filter(
		([command]) => command === 'update_settings',
	)
	const last = calls.at(-1)?.[1] as { settings: Settings } | undefined
	return last?.settings.mods ?? { trusted: [], perCommit: false }
}

function withTrust() {
	applySettings({
		...blankSettings(),
		mods: {
			trusted: [
				{ repo: 'dev/sphere', commit: null, sandboxed: true, since: SINCE },
				{ repo: 'dev/tiny', commit: SHA, sandboxed: false, since: SINCE },
			],
			perCommit: false,
		},
	})
}

beforeAll(() => {
	Element.prototype.scrollIntoView = () => {}
})

beforeEach(() => {
	asked.mockReset()
	asked.mockImplementation(async (command: string, args?: unknown) => {
		if (command === 'update_settings')
			return structuredClone((args as { settings: Settings }).settings)
		return null
	})
	applySettings(blankSettings())
})

afterEach(() => cleanup())

describe('the mods section', () => {
	test('lists each trusted repository with its commit or default-branch wording', () => {
		withTrust()
		const { container } = open()
		const [sphere, tiny] = rows(container)
		expect(sphere?.textContent).toContain('dev/sphere')
		expect(sphere?.textContent).toContain('any commit on its default branch')
		expect(sphere?.textContent).not.toContain('outside the sandbox')
		expect(tiny?.textContent).toContain('dev/tiny')
		expect(tiny?.textContent).toContain('9108a17')
		// Already off: the warning stands under it.
		expect(tiny?.textContent).toContain(
			'Mod code from this repository runs outside the sandbox, as you do. Only for mods you have read.',
		)
	})

	test('revoke removes the row from the draft', async () => {
		withTrust()
		const { container } = open()
		fireEvent.click(button(rows(container)[0]!, 'Revoke'))
		expect(rows(container)).toHaveLength(1)
		await vi.waitFor(
			() =>
				expect(saved().trusted.map((entry) => entry.repo)).toEqual([
					'dev/tiny',
				]),
			{ timeout: 3000 },
		)
	})

	test('turning the sandbox off shows the warning under the row', async () => {
		withTrust()
		const { container } = open()
		const sphere = () => rows(container)[0]!
		fireEvent.change(sphere().querySelector('select')!, {
			target: { value: 'off' },
		})
		expect(sphere().textContent).toContain('runs outside the sandbox')
		await vi.waitFor(() => expect(saved().trusted[0]?.sandboxed).toBe(false), {
			timeout: 3000,
		})
	})

	test('adding a repository appends a sandboxed default-branch entry', async () => {
		withTrust()
		const { container } = open()
		const field = section(container).querySelector<HTMLInputElement>(
			'.mods-trust-add input',
		)!
		fireEvent.input(field, { target: { value: 'dev/new' } })
		fireEvent.click(button(section(container), 'Allow a repository'))
		expect(rows(container)).toHaveLength(3)
		expect(rows(container)[2]?.textContent).toContain('dev/new')
		expect(rows(container)[2]?.textContent).toContain(
			'any commit on its default branch',
		)
		await vi.waitFor(
			() =>
				expect(saved().trusted[2]).toEqual({
					repo: 'dev/new',
					commit: null,
					sandboxed: true,
					since: expect.any(String),
				}),
			{ timeout: 3000 },
		)
	})

	test('the per-commit switch sets mods.perCommit', async () => {
		const { container } = open()
		const box = [
			...section(container).querySelectorAll<HTMLInputElement>(
				'input[type="checkbox"]',
			),
		].find((input) =>
			input.parentElement?.textContent?.includes('Trust per commit'),
		)!
		fireEvent.click(box)
		await vi.waitFor(() => expect(saved().perCommit).toBe(true), {
			timeout: 3000,
		})
	})
})
