import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router'
import { cleanup, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { createSignal } from 'solid-js'
import { afterEach, describe, expect, test, vi } from 'vitest'
import { fakeRoom, myBattle } from './room/fixture'
import { RoomProvider, type RoomModel } from './room/model'
import { Setup } from './Setup'

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
