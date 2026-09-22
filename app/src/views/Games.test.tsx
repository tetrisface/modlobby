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

function open() {
	const history = createMemoryHistory()
	history.set({ value: '/?section=games' })
	return render(() => (
		<MemoryRouter history={history}>
			<Route path='/' component={SettingsView} />
		</MemoryRouter>
	))
}

const section = (root: HTMLElement) =>
	root.querySelector<HTMLElement>('#settings-games')!

function button(root: Element, text: string): HTMLButtonElement {
	const found = [...root.querySelectorAll('button')].find(
		(candidate) => candidate.textContent?.trim() === text,
	)
	if (!found) throw new Error(`no ${text} button`)
	return found
}

/** The overrides as the last save sent them. */
function saved(): Settings['games']['overrides'] {
	const calls = asked.mock.calls.filter(
		([command]) => command === 'update_settings',
	)
	const last = calls.at(-1)?.[1] as { settings: Settings } | undefined
	return last?.settings.games.overrides ?? []
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

describe('the games section', () => {
	test('an override is added, filled in and saved in the settings', async () => {
		const { container } = open()
		fireEvent.click(button(section(container), 'Add an override'))
		const row = section(container).querySelector('.game-override')!
		const [name, repo, asset] = [...row.querySelectorAll('input')]
		fireEvent.input(name!, { target: { value: 'SplinterFaction 0.1.86' } })
		fireEvent.input(repo!, { target: { value: 'fork/SplinterFaction' } })
		fireEvent.input(asset!, { target: { value: 'lite' } })

		await vi.waitFor(
			() =>
				expect(saved()).toEqual([
					{
						name: 'SplinterFaction 0.1.86',
						source: {
							kind: 'github',
							value: 'fork/SplinterFaction',
							asset: 'lite',
						},
					},
				]),
			{ timeout: 3000 },
		)
	})

	test('switching to an address drops the repository fields, and Remove removes', async () => {
		applySettings({
			...blankSettings(),
			games: {
				overrides: [
					{
						name: 'X 1',
						source: { kind: 'github', value: 'a/b', asset: null },
					},
				],
			},
		})
		const { container } = open()
		const row = () => section(container).querySelector('.game-override')
		fireEvent.change(row()!.querySelector('select')!, {
			target: { value: 'url' },
		})
		expect(row()!.querySelectorAll('input')).toHaveLength(2)
		expect(row()!.textContent).toContain('Address')

		// A commit, built: the repository and, optionally, the placeholder.
		fireEvent.change(row()!.querySelector('select')!, {
			target: { value: 'git' },
		})
		expect(row()!.textContent).toContain('Repository')
		expect(row()!.textContent).toContain('Version placeholder')
		fireEvent.input(row()!.querySelectorAll('input')[1]!, {
			target: { value: 'dev/Game' },
		})
		await vi.waitFor(
			() =>
				expect(saved()).toEqual([
					{ name: 'X 1', source: { kind: 'git', value: 'dev/Game' } },
				]),
			{ timeout: 3000 },
		)

		fireEvent.click(button(row()!, 'Remove'))
		expect(row()).toBeNull()
		await vi.waitFor(() => expect(saved()).toEqual([]), { timeout: 3000 })
	})
})
