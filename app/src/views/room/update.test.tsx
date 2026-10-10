import { MemoryRouter, Route } from '@solidjs/router'
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { Room } from '../Room'
import { type Calls, battle, fakeRoom, myBattle, recordingIo } from './fixture'
import { RoomProvider } from './model'

vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(),
	convertFileSrc: (path: string, scheme: string) => `${scheme}://${path}`,
}))

const NEWER = 'Beyond All Reason test-31452-0a1b2c3'

beforeEach(() => {
	vi.mocked(invoke).mockImplementation(async (command: string) => {
		switch (command) {
			case 'newer_game':
				return NEWER
			case 'game_modoptions':
			case 'game_ais':
			case 'game_unit_names':
				return []
			case 'engine_def_tags':
				return { weapon: [] }
			default:
				return null
		}
	})
})

afterEach(() => {
	cleanup()
	vi.clearAllMocks()
})

test('on a mods host, the update says it rehosts the lobby, then asks for it', async () => {
	const calls: Calls = []
	const room = fakeRoom({
		battle: () => battle({ gameName: 'Beyond All Reason test-31368-8379d65' }),
		// A spectator: whether it applies, votes or refuses is the host's call.
		my: () => myBattle({ scriptTags: { 'game/mutatorhost': '1' } }),
		io: recordingIo(calls),
	})
	const { container } = render(() => (
		<MemoryRouter
			root={(props) => (
				<RoomProvider value={room}>{props.children}</RoomProvider>
			)}
		>
			<Route path='/' component={Room} />
		</MemoryRouter>
	))
	const update = await waitFor(() => {
		const found = container.querySelector<HTMLButtonElement>(
			'[aria-label^="Update the game"]',
		)
		if (!found) throw new Error('no update offered')
		return found
	})
	expect(update.title).toContain('rehosts the lobby')

	const said = () => calls.filter(([name]) => name === 'sayBattle')
	fireEvent.click(update)
	expect(update.textContent).toBe('Rehost the lobby?')
	expect(said()).toEqual([])

	fireEvent.click(update)
	await waitFor(() =>
		expect(said()).toEqual([['sayBattle', ['!gameVersion byar:test']]]),
	)
})
