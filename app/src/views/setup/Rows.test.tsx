import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { createSignal } from 'solid-js'
import { afterEach, describe, expect, test, vi } from 'vitest'
import type { ModOption } from '../../ipc/bindings/ModOption'
import { NO_NOTE, rowsOf, type Row, type RowNote } from '../../lib/setup'
import {
	ALONE,
	fakeRoom,
	myBattle,
	recordingIo,
	type Calls,
} from '../room/fixture'
import { RoomProvider, type RoomModel } from '../room/model'
import { Rows } from './Rows'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

afterEach(() => {
	cleanup()
	vi.mocked(invoke).mockReset()
})

const MAX_UNITS: ModOption = {
	key: 'maxunits',
	name: 'Max Units',
	desc: 'How many',
	type: 'number',
	def: 2000,
	min: 500,
	max: 32000,
	step: 1,
}

const MODE: ModOption = {
	key: 'deathmode',
	name: 'Game End Mode',
	type: 'list',
	def: 'com',
	items: [
		{ key: 'com', name: 'Kill all Commanders' },
		{ key: 'killall', name: 'Kill everything' },
	],
}

const rowOf = (option: ModOption, current: string | null = null): Row =>
	rowsOf(
		{ name: '', options: [option] },
		current === null ? {} : { [option.key]: current },
	)[0]!

/** Rows in a room where we may set things, recording what they send. */
function draw(
	rows: Row[],
	over: {
		room?: Partial<RoomModel>
		editable?: boolean
		note?: (row: Row) => RowNote
	} = {},
) {
	const calls: Calls = []
	const room = fakeRoom({ io: recordingIo(calls), ...over.room })
	const { container } = render(() => (
		<RoomProvider value={room}>
			<Rows rows={rows} editable={over.editable ?? true} note={over.note} />
		</RoomProvider>
	))
	const row = (key: string) =>
		container.querySelector<HTMLElement>(`[data-key="${key}"]`)!
	const reset = (key: string) =>
		row(key).querySelector<HTMLButtonElement>('.act-cell button')!
	return { calls, container, row, reset }
}

describe('a row that moves', () => {
	const row = (key: string, name: string, changed = false): Row => ({
		option: { key, name, type: 'number', def: 1 } as ModOption,
		current: null,
		changed,
	})

	test('takes its input with it, so a commit lands on its own setting', () => {
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

describe('the default beside a value', () => {
	test('a changed row says what it left, in the words the row uses', () => {
		const { row } = draw([rowOf(MAX_UNITS, '10000'), rowOf(MODE, 'killall')])
		expect(row('maxunits').querySelector('.opt-default')?.textContent).toBe(
			'default 2000',
		)
		expect(row('deathmode').querySelector('.opt-default')?.textContent).toBe(
			'default Kill all Commanders',
		)
	})

	test('a row on its default has nothing to add', () => {
		const { row } = draw([rowOf(MAX_UNITS)])
		expect(row('maxunits').querySelector('.opt-default')).toBeNull()
	})
})

describe('going back to the default', () => {
	test('sends the default', () => {
		const { calls, reset } = draw([rowOf(MAX_UNITS, '10000')])
		fireEvent.click(reset('maxunits'))
		expect(calls).toEqual([['setOption', ['maxunits', '2000']]])
	})

	test('is greyed on a row already there, and says so', () => {
		const { reset } = draw([rowOf(MAX_UNITS)])
		expect(reset('maxunits').disabled).toBe(true)
		expect(reset('maxunits').title).toBe('Already on its default')
	})

	test('names the default it goes back to', () => {
		const { reset } = draw([rowOf(MAX_UNITS, '10000')])
		expect(reset('maxunits').disabled).toBe(false)
		expect(reset('maxunits').title).toBe('Back to its default, 2000')
	})

	test('is greyed for somebody the host would refuse, with the reason', () => {
		const { reset } = draw([rowOf(MAX_UNITS, '10000')], {
			editable: false,
			room: { my: () => myBattle({ boss: 'Alice' }) },
		})
		expect(reset('maxunits').disabled).toBe(true)
		expect(reset('maxunits').title).toBe(
			'Only the room’s boss can change settings',
		)
	})

	test('clears an empty text with a 0 where a host would skip an empty value', () => {
		const lava: ModOption = { key: 'lava', type: 'string', def: '' }
		const { calls, reset } = draw([rowOf(lava, 'abc')])
		fireEvent.click(reset('lava'))
		expect(calls).toEqual([['setOption', ['lava', '0']]])
	})

	test('empties an empty text where nobody is there to skip it', () => {
		const lava: ModOption = { key: 'lava', type: 'string', def: '' }
		const { calls, reset } = draw([rowOf(lava, 'abc')], {
			room: { caps: ALONE },
		})
		fireEvent.click(reset('lava'))
		expect(calls).toEqual([['setOption', ['lava', '']]])
	})
})

describe('a number being edited', () => {
	const input = (row: HTMLElement) =>
		row.querySelector<HTMLInputElement>('input[type="number"]')!

	test('shows its default when emptied and names its range', () => {
		const { row } = draw([rowOf(MAX_UNITS, '10000')])
		expect(input(row('maxunits')).placeholder).toBe('2000')
		expect(input(row('maxunits')).title).toBe(
			'min 500 · max 32000 · step 1 · default 2000',
		)
	})

	test('a typo it cannot read as a number goes back to what it held, and sends nothing', () => {
		const { calls, row } = draw([rowOf(MAX_UNITS, '10000')])
		const field = input(row('maxunits'))
		// What a number field reports for "1e": an empty value, and this.
		Object.defineProperty(field, 'validity', { value: { badInput: true } })
		fireEvent.change(field, { target: { value: '' } })
		expect(calls).toEqual([])
		expect(field.value).toBe('10000')
	})

	test('left empty goes back to that default rather than sending nothing', () => {
		const { calls, row } = draw([rowOf(MAX_UNITS, '10000')])
		fireEvent.change(input(row('maxunits')), { target: { value: '' } })
		expect(calls).toEqual([['setOption', ['maxunits', '2000']]])
	})
})

describe('a list being edited', () => {
	const WORDY: ModOption = {
		...MODE,
		items: [
			{ key: 'com', name: 'Kill all Commanders', desc: 'No commanders left' },
			{ key: 'killall', name: 'Kill everything' },
		],
	}

	afterEach(() => {
		vi.unstubAllGlobals()
	})

	test('says what each choice does, where the webview can draw that', () => {
		vi.stubGlobal('CSS', { supports: () => true })
		const { row } = draw([rowOf(WORDY)])
		const options = row('deathmode').querySelectorAll('option')
		expect(options[0]!.querySelector('.select-note')?.textContent).toBe(
			'No commanders left',
		)
		expect(options[1]!.querySelector('.select-note')).toBeNull()
		// What keeps those words out of the closed box.
		expect(row('deathmode').querySelector('select > button')).not.toBeNull()
	})

	test('keeps to their names where it cannot, since a native list would run the words together', () => {
		vi.stubGlobal('CSS', { supports: () => false })
		const { row } = draw([rowOf(WORDY)])
		const options = row('deathmode').querySelectorAll('option')
		expect(options[0]!.textContent).toBe('Kill all Commanders')
		expect(row('deathmode').querySelector('select > button')).toBeNull()
	})
})

describe('a link', () => {
	const LINK: ModOption = {
		key: 'changelog',
		name: 'Changelog',
		type: 'link',
		link: 'https://example.com/changes',
	}

	test('shows where it goes and opens it in the system browser', () => {
		vi.mocked(invoke).mockResolvedValue(undefined)
		const { row } = draw([rowOf(LINK)])
		expect(row('changelog').querySelector('.v')?.textContent).toBe(
			'https://example.com/changes',
		)
		fireEvent.click(row('changelog').querySelector('.act-cell button')!)
		expect(invoke).toHaveBeenCalledWith('open_url', {
			url: 'https://example.com/changes',
		})
	})

	test('has nothing to reset or edit', () => {
		const { row } = draw([rowOf(LINK)])
		expect(row('changelog').querySelectorAll('.act-cell button')).toHaveLength(
			1,
		)
		expect(row('changelog').querySelector('input')).toBeNull()
	})
})

describe('what the pane knows about a row', () => {
	test('a change a moment ago lights it', () => {
		const { row } = draw([rowOf(MAX_UNITS, '10000'), rowOf(MODE)], {
			note: (r) => ({ ...NO_NOTE, recent: r.option.key === 'maxunits' }),
		})
		expect(row('maxunits').classList.contains('recent')).toBe(true)
		expect(row('deathmode').classList.contains('recent')).toBe(false)
	})

	test('a vote under way says what it would set', () => {
		const { row } = draw([rowOf(MAX_UNITS, '10000'), rowOf(MODE)], {
			note: (r) => ({
				...NO_NOTE,
				vote: r.option.key === 'maxunits' ? '32000' : null,
			}),
		})
		expect(row('maxunits').querySelector('.opt-vote')?.textContent).toBe(
			'vote 32000',
		)
		expect(row('deathmode').querySelector('.opt-vote')).toBeNull()
	})

	test('an option held off by another stays, dimmed and saying by what', () => {
		const { row } = draw([rowOf(MAX_UNITS, '10000')], {
			note: () => ({
				...NO_NOTE,
				lock: { by: 'Game End Mode', value: 'Dominion' },
			}),
		})
		expect(row('maxunits').classList.contains('off')).toBe(true)
		const tag = row('maxunits').querySelector<HTMLElement>('.opt-off')
		expect(tag?.textContent).toBe('off')
		expect(tag?.title).toBe('Not in force while Game End Mode is Dominion')
	})

	test('the pinned row is marked', () => {
		const { row } = draw([rowOf(MAX_UNITS)], {
			note: () => ({ ...NO_NOTE, on: true }),
		})
		expect(row('maxunits').classList.contains('on')).toBe(true)
	})
})
