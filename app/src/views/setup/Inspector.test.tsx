import { cleanup, fireEvent, render } from '@solidjs/testing-library'
import { createSignal } from 'solid-js'
import { afterEach, describe, expect, test, vi } from 'vitest'
import { Query } from '../../components/SettingRow'
import type { ModOption } from '../../ipc/bindings/ModOption'
import type { OptionChangeView } from '../../ipc/bindings/OptionChangeView'
import type { VoteView } from '../../ipc/bindings/VoteView'
import { rowsOf, type Lock, type Row } from '../../lib/setup'
import { fakeRoom, recordingIo, type Calls } from '../room/fixture'
import { RoomProvider } from '../room/model'
import { Inspector } from './Inspector'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }))

afterEach(() => {
	cleanup()
	vi.useRealTimers()
})

const MAX_UNITS: ModOption = {
	key: 'maxunits',
	name: 'Max Units',
	desc: 'How many there may be',
	type: 'number',
	def: 2000,
	min: 500,
	max: 32000,
	step: 1,
}

const MODE: ModOption = {
	key: 'deathmode',
	name: 'Game End Mode',
	desc: 'What it takes',
	type: 'list',
	def: 'com',
	items: [
		{ key: 'com', name: 'Kill all Commanders', desc: 'No commanders left' },
		{ key: 'killall', name: 'Kill everything' },
	],
}

const RANKED: ModOption = {
	key: 'ranked_game',
	name: 'Ranked Game',
	type: 'bool',
	def: true,
}

const rowOf = (option: ModOption, current: string | null = null): Row =>
	rowsOf(
		{ name: '', options: [option] },
		current === null ? {} : { [option.key]: current },
	)[0]!

function draw(
	row: Row | null,
	over: {
		refusal?: string | null
		change?: OptionChangeView
		vote?: VoteView | null
		lock?: Lock | null
		needle?: string
	} = {},
) {
	const calls: Calls = []
	const room = fakeRoom({ io: recordingIo(calls) })
	const { container } = render(() => (
		<RoomProvider value={room}>
			<Query.Provider value={() => over.needle ?? ''}>
				<Inspector
					row={row}
					fallback='Rest on a setting'
					refusal={over.refusal ?? null}
					change={over.change}
					vote={over.vote ?? null}
					lock={over.lock ?? null}
				/>
			</Query.Provider>
		</RoomProvider>
	))
	const text = (selector: string) =>
		container.querySelector(selector)?.textContent
	const pill = (label: string) =>
		[...container.querySelectorAll<HTMLButtonElement>('.choice button')].find(
			(button) => button.textContent === label,
		)!
	return { calls, container, text, pill }
}

describe('with no row picked yet', () => {
	test('says what it is for instead', () => {
		const { text, container } = draw(null)
		expect(text('.inspector-empty')).toBe('Rest on a setting')
		expect(container.querySelector('.inspector-name')).toBeNull()
	})
})

describe('a number', () => {
	test('gives its name, key, description, value, range and default', () => {
		const { text } = draw(rowOf(MAX_UNITS, '10000'))
		expect(text('.inspector-name')).toBe('Max Units')
		expect(text('.inspector-key')).toBe('maxunits')
		expect(text('.inspector-desc')).toBe('How many there may be')
		expect(text('.inspector-value')).toBe('10000')
		expect(text('.inspector-facts')).toBe(
			'min 500 · max 32000 · step 1 · default 2000',
		)
	})

	test('offers no choices', () => {
		const { container } = draw(rowOf(MAX_UNITS))
		expect(container.querySelector('.choice')).toBeNull()
	})
})

describe('a list', () => {
	test('offers its items as pills, the value lit', () => {
		const { pill } = draw(rowOf(MODE, 'killall'))
		expect(pill('Kill everything').getAttribute('aria-pressed')).toBe('true')
		expect(pill('Kill all Commanders').getAttribute('aria-pressed')).toBe(
			'false',
		)
	})

	test('says what the chosen item does', () => {
		const { text } = draw(rowOf(MODE))
		expect(text('.inspector-choice')).toBe('No commanders left')
	})

	test('says nothing more when the chosen item has no words of its own', () => {
		const { container } = draw(rowOf(MODE, 'killall'))
		expect(container.querySelector('.inspector-choice')).toBeNull()
	})

	test('a pill pressed sends that choice', () => {
		const { calls, pill } = draw(rowOf(MODE))
		fireEvent.click(pill('Kill everything'))
		expect(calls).toEqual([['setOption', ['deathmode', 'killall']]])
	})

	test('the lit pill pressed sends nothing', () => {
		const { calls, pill } = draw(rowOf(MODE))
		fireEvent.click(pill('Kill all Commanders'))
		expect(calls).toEqual([])
	})

	test('pills are greyed for somebody the host would refuse, and say why', () => {
		const refusal = 'Only the room’s boss can change settings'
		const { calls, pill } = draw(rowOf(MODE), { refusal })
		expect(pill('Kill everything').disabled).toBe(true)
		expect(pill('Kill everything').title).toBe(refusal)
		expect(pill('Kill all Commanders').title).toBe(
			`No commanders left\n${refusal}`,
		)
		fireEvent.click(pill('Kill everything'))
		expect(calls).toEqual([])
	})

	test('a pill says what its item does on hover when it may be pressed', () => {
		const { pill } = draw(rowOf(MODE))
		expect(pill('Kill all Commanders').title).toBe('No commanders left')
	})
})

describe('a switch', () => {
	test('is two pills', () => {
		const { pill } = draw(rowOf(RANKED))
		expect(pill('on').getAttribute('aria-pressed')).toBe('true')
		expect(pill('off').getAttribute('aria-pressed')).toBe('false')
	})

	test('pressed off, sends what the protocol reads as off', () => {
		const { calls, pill } = draw(rowOf(RANKED))
		fireEvent.click(pill('off'))
		expect(calls).toEqual([['setOption', ['ranked_game', '0']]])
	})
})

describe('a document', () => {
	const SLOT: ModOption = {
		key: 'tweakdefs1',
		name: 'tweakdefs1',
		desc: 'Base64url Lua carried as a modoption.',
		type: 'string',
		def: '',
	}

	test('says how long its blob is, never the blob', () => {
		const { text, container } = draw(rowOf(SLOT, 'QUJD'))
		expect(text('.inspector-facts')).toBe('4 characters of base64url')
		expect(container.textContent).not.toContain('QUJD')
	})

	test('says when it is empty', () => {
		expect(draw(rowOf(SLOT)).text('.inspector-facts')).toBe('empty')
		expect(draw(rowOf(SLOT, '0')).text('.inspector-facts')).toBe('empty')
	})
})

describe('a link', () => {
	test('says where it goes', () => {
		const { text } = draw(
			rowOf({
				key: 'changelog',
				name: 'Changelog',
				type: 'link',
				link: 'https://example.com/changes',
			}),
		)
		expect(text('.inspector-facts')).toBe('https://example.com/changes')
	})
})

describe('a hidden option', () => {
	test('is tagged as the row is', () => {
		const { text } = draw(rowOf({ ...MAX_UNITS, hidden: true }))
		expect(text('.opt-hidden')).toBe('hidden')
	})
})

describe('what has happened to it', () => {
	const change = (over: Partial<OptionChangeView> = {}): OptionChangeView => ({
		seq: 1,
		key: 'maxunits',
		from: '2000',
		to: '10000',
		by: 'Alice',
		at: 1_800_000_000,
		...over,
	})

	test('says who set it, when and from what', () => {
		vi.useFakeTimers()
		vi.setSystemTime(1_800_000_000 * 1000 + 300_000)
		const { text } = draw(rowOf(MAX_UNITS, '10000'), { change: change() })
		expect(text('.inspector-trace')).toBe('Set by Alice 5m ago, was 2000')
	})

	test('an author the host has not named yet is left out', () => {
		vi.useFakeTimers()
		vi.setSystemTime(1_800_000_000 * 1000)
		const { text } = draw(rowOf(MAX_UNITS, '10000'), {
			change: change({ by: null }),
		})
		expect(text('.inspector-trace')).toBe('Set just now, was 2000')
	})

	test('says what a list came from by name', () => {
		vi.useFakeTimers()
		vi.setSystemTime(1_800_000_000 * 1000)
		const { text } = draw(rowOf(MODE, 'killall'), {
			change: change({ key: 'deathmode', from: 'com', to: 'killall' }),
		})
		expect(text('.inspector-trace')).toBe(
			'Set by Alice just now, was Kill all Commanders',
		)
	})

	test('a value not set before was on its default', () => {
		vi.useFakeTimers()
		vi.setSystemTime(1_800_000_000 * 1000)
		const { text } = draw(rowOf(MAX_UNITS, '10000'), {
			change: change({ from: '' }),
		})
		expect(text('.inspector-trace')).toBe('Set by Alice just now, was 2000')
	})

	test('a document never spells out what it was', () => {
		vi.useFakeTimers()
		vi.setSystemTime(1_800_000_000 * 1000)
		const { text } = draw(
			rowOf(
				{ key: 'tweakdefs1', name: 'tweakdefs1', type: 'string', def: '' },
				'QUJD',
			),
			{ change: change({ key: 'tweakdefs1', from: 'WFla', to: 'QUJD' }) },
		)
		expect(text('.inspector-trace')).toBe('Set by Alice just now')
	})

	test('nothing seen, nothing said', () => {
		const { container } = draw(rowOf(MAX_UNITS, '10000'))
		expect(container.querySelector('.inspector-trace')).toBeNull()
	})
})

describe('a vote on it', () => {
	const vote: VoteView = {
		command: 'bSet maxunits 32000',
		by: 'Bob',
		proposal: { type: 'setOption', key: 'maxunits', value: '32000' },
		yes: 3,
		yesNeeded: 5,
		no: 1,
		noNeeded: 5,
		remainingSecs: 40,
	}

	test('says what it would set, how it stands and how long it has', () => {
		const { text } = draw(rowOf(MAX_UNITS, '10000'), { vote })
		expect(text('.inspector-vote')).toBe(
			'Bob called a vote to set 32000: yes 3/5, no 1/5, 40s left',
		)
	})

	test('counts down between what the host says', () => {
		vi.useFakeTimers()
		const { text } = draw(rowOf(MAX_UNITS, '10000'), { vote })
		vi.advanceTimersByTime(3000)
		expect(text('.inspector-vote')).toBe(
			'Bob called a vote to set 32000: yes 3/5, no 1/5, 37s left',
		)
	})

	test('says a list item by name', () => {
		const { text } = draw(rowOf(MODE), {
			vote: {
				...vote,
				proposal: { type: 'setOption', key: 'deathmode', value: 'killall' },
			},
		})
		expect(text('.inspector-vote')).toContain('to set Kill everything:')
	})

	test('never spells out a document', () => {
		const { text } = draw(
			rowOf({ key: 'tweakdefs1', name: 'tweakdefs1', type: 'string', def: '' }),
			{
				vote: {
					...vote,
					proposal: { type: 'setOption', key: 'tweakdefs1', value: 'QUJD' },
				},
			},
		)
		expect(text('.inspector-vote')).toBe(
			'Bob called a vote to change this: yes 3/5, no 1/5, 40s left',
		)
	})
})

describe('an option another holds off', () => {
	test('says what holds it off', () => {
		const { text } = draw(rowOf(MAX_UNITS), {
			lock: { by: 'Game End Mode', value: 'Dominion' },
		})
		expect(text('.inspector-off')).toBe(
			'Not in force while Game End Mode is Dominion',
		)
	})

	test('nothing holding it, nothing said', () => {
		const { container } = draw(rowOf(MAX_UNITS))
		expect(container.querySelector('.inspector-off')).toBeNull()
	})
})

describe('a search', () => {
	test('marks what it found in the name and the description', () => {
		const { container } = draw(rowOf(MAX_UNITS), { needle: 'many max' })
		const marks = [...container.querySelectorAll('mark')].map(
			(mark) => mark.textContent,
		)
		expect(marks).toEqual(['Max', 'many'])
	})
})

describe('as the row it describes changes', () => {
	test('follows it', () => {
		const [row, setRow] = createSignal<Row | null>(rowOf(MAX_UNITS))
		const room = fakeRoom({ io: recordingIo([]) })
		const { container } = render(() => (
			<RoomProvider value={room}>
				<Inspector
					row={row()}
					fallback=''
					refusal={null}
					change={undefined}
					vote={null}
					lock={null}
				/>
			</RoomProvider>
		))
		setRow(rowOf(MODE))
		expect(container.querySelector('.inspector-name')?.textContent).toBe(
			'Game End Mode',
		)
		expect(container.querySelector('.inspector-facts')?.textContent).toBe(
			'default Kill all Commanders',
		)
	})
})
