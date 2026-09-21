import { fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { describe, expect, test, vi } from 'vitest'
import type { TweakView } from '../../ipc/bindings/TweakView'
import type { Row } from '../../lib/setup'
import { slotId } from '../../lib/tweakspace'
import { chat } from '../../store/chat'
import { tweakspaceFor } from '../../store/tweakspaceInstance'
import {
	fakeRoom,
	myBattle,
	recordingIo,
	status,
	user,
	type Calls,
} from '../room/fixture'
import { RoomProvider, type RoomModel } from '../room/model'
import { TweakRow } from './TweakRow'

/** Decodes every blob but `garbage`; the name is the blob, as a header. */
vi.mock('@tauri-apps/api/core', () => ({
	invoke: vi.fn(async (command: string, args: { blob?: string }) => {
		if (command !== 'tweak_decode') return []
		if (args.blob === 'garbage') throw new Error('not base64url')
		return {
			text: `-- ${args.blob}\nlocal a = 1\n`,
			formatted: `-- ${args.blob}\nlocal a = 1\n`,
			name: args.blob ?? null,
			summary: '',
			diagnostics: [],
		} satisfies TweakView
	}),
}))

// The editor is Monaco, which the test DOM cannot draw; that it opens is enough.
vi.mock('./Tweaks', () => ({ Tweaks: () => <div>the editor</div> }))

/** Lets the awaited decodes and sends reach the store and the DOM. */
async function settle() {
	for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

const row = (key: string, current: string | null): Row => ({
	option: { key, name: key, desc: 'Base64url Lua', type: 'string', def: '' },
	current,
	changed: current !== null,
})

let rooms = 0
function mount(current: string | null, over: Partial<RoomModel> = {}) {
	const calls: Calls = []
	const room = fakeRoom({
		// A workspace lives as long as its room; each test gets a room of its own.
		log: `#tweak-row-${rooms++}`,
		io: recordingIo(calls),
		my: () =>
			myBattle({
				scriptTags:
					current === null ? {} : { 'game/modoptions/tweakdefs1': current },
			}),
		...over,
	})
	const view = render(() => (
		<RoomProvider value={room}>
			<TweakRow row={row('tweakdefs1', current)} />
		</RoomProvider>
	))
	return { calls, room, space: tweakspaceFor(room), ...view }
}

const input = (view: ReturnType<typeof mount>) =>
	view.getByLabelText('tweakdefs1 as base64url') as HTMLInputElement

describe('TweakRow', () => {
	test('shows the key, the name the tweak goes by, and its blob', async () => {
		const view = mount('TmV3')
		await settle()
		expect(view.getByText('tweakdefs1')).toBeTruthy()
		expect(view.getByText('TmV3', { selector: '.tweak-name' })).toBeTruthy()
		expect(input(view).value).toBe('TmV3')
		expect(
			(
				view.getByTitle(
					'Copy the !bSet command for tweakdefs1',
				) as HTMLButtonElement
			).disabled,
		).toBe(false)
	})

	test('an empty slot has nothing to copy, and SPADS’s 0 is empty', async () => {
		const view = mount('0')
		await settle()
		expect(input(view).value).toBe('')
		expect(
			(
				view.getByTitle(
					'Copy the !bSet command for tweakdefs1',
				) as HTMLButtonElement
			).disabled,
		).toBe(true)
		expect(view.getByTitle('Write into tweakdefs1')).toBeTruthy()
	})

	test('a pasted blob is decoded, then sent the way the editor sends', async () => {
		const view = mount('TmV3')
		await settle()
		fireEvent.change(input(view), { target: { value: '  T3RoZXI  ' } })
		await settle()
		expect(vi.mocked(invoke)).toHaveBeenCalledWith('tweak_decode', {
			blob: 'T3RoZXI',
			kind: 'defs',
		})
		expect(view.calls).toContainEqual([
			'tweakSend',
			// As it came: never minified on the way back out.
			['-- T3RoZXI\nlocal a = 1\n', { kind: 'defs', index: 1 }, true, false],
		])
	})

	test('what cannot be decoded never reaches the room, and the field goes back', async () => {
		const view = mount('TmV3')
		await settle()
		fireEvent.change(input(view), { target: { value: 'garbage' } })
		await settle()
		expect(view.calls.some(([name]) => name === 'tweakSend')).toBe(false)
		expect(input(view).value).toBe('TmV3')
		expect(chat.notices.at(-1)?.text).toContain('not base64url')
	})

	test('emptying the field is not clearing the slot', async () => {
		const view = mount('TmV3')
		await settle()
		fireEvent.change(input(view), { target: { value: '' } })
		await settle()
		expect(view.calls).toEqual([])
		expect(input(view).value).toBe('TmV3')
	})

	test('a spectator reads the blob but cannot paste over it', async () => {
		const view = mount('TmV3', {
			users: () => ({
				me: user('me', { battleStatus: status({ player: false }) }),
			}),
		})
		await settle()
		expect(input(view).readOnly).toBe(true)
	})

	test('the pen opens the editor under the row; the chevron folds it and keeps the edit', async () => {
		const view = mount('TmV3')
		await settle()
		fireEvent.click(view.getByTitle('Open tweakdefs1 in the editor'))
		expect(view.getByText('the editor')).toBeTruthy()
		expect(view.space.ws.expanded).toBe(slotId('tweakdefs1'))

		view.space.edit(slotId('tweakdefs1'), 'local mine = 1')
		fireEvent.click(
			view.getByTitle('Fold the editor away; what you typed is kept'),
		)
		expect(view.queryByText('the editor')).toBeNull()
		expect(view.getByText('unsent')).toBeTruthy()
	})
})
