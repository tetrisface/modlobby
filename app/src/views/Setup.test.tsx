import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { createSignal } from 'solid-js'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { ModOption } from '../ipc/bindings/ModOption'
import type { Row } from '../lib/setup'
import { fakeRoom, myBattle } from './room/fixture'
import { RoomProvider, type RoomModel } from './room/model'
import { Rows, Setup } from './Setup'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

afterEach(() => {
	cleanup()
	vi.mocked(invoke).mockReset()
})

/** The pane in `room`, on a route since its Mods face navigates. */
function pane(room: RoomModel) {
	// The tables the pane and its tweak editor read at mount, all empty.
	vi.mocked(invoke).mockImplementation(async (command: string) =>
		command === 'engine_def_tags' ? { weapon: [] } : [],
	)
	const { container } = render(() => (
		<MemoryRouter history={createMemoryHistory()}>
			<Route
				path='/*'
				component={() => (
					<RoomProvider value={room}>
						<Setup />
					</RoomProvider>
				)}
			/>
		</MemoryRouter>
	))
	return () => container.querySelector('.pane-tab.on')?.textContent
}

describe('which face the pane opens on', () => {
	test('the options, in a room whose host runs no mods', () => {
		expect(pane(fakeRoom())()).toBe('Options')
	})

	test('the mods, once the host says it runs them', () => {
		const [tags, setTags] = createSignal<Record<string, string>>({})
		const on = pane(fakeRoom({ my: () => myBattle({ scriptTags: tags() }) }))
		expect(on()).toBe('Options')

		// The tags land after the room does.
		setTags({ 'game/mutatorhost': '1' })
		expect(on()).toBe('Mods')
	})
})

describe('Rows', () => {
	const row = (key: string, name: string, changed = false): Row => ({
		option: { key, name, type: 'number', def: 1 } as ModOption,
		current: null,
		changed,
	})

	test('a row that moves takes its input with it, so a commit lands on its own setting', () => {
		const metal = row('startmetal', 'Starting Metal')
		const energy = row('startenergy', 'Starting Energy')
		const [rows, setRows] = createSignal([metal, energy])
		const set = vi.fn(async () => {})
		const { getByText } = render(() => (
			<RoomProvider value={fakeRoom()}>
				<Rows rows={rows()} editable set={set} />
			</RoomProvider>
		))
		const inputOf = (name: string) =>
			getByText(name).closest('.opt')!.querySelector('input')!
		const input = inputOf('Starting Energy')

		// Somebody else's change sorts energy to the top while it is being typed in.
		setRows([{ ...energy, changed: true }, metal])

		expect(inputOf('Starting Energy')).toBe(input)
		fireEvent.change(input, { target: { value: '5000' } })
		expect(set).toHaveBeenCalledWith('startenergy', '5000')
	})
})
