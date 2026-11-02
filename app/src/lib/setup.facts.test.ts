import { describe, expect, test } from 'vitest'
import type { ModOption } from '../ipc/bindings/ModOption'
import {
	choices,
	defaultWords,
	displayText,
	facts,
	latestChange,
	linkHost,
	linksOf,
	litChoice,
	locksOf,
	resetValue,
	rowsOf,
	tabs,
	tipFacts,
	voteOn,
} from './setup'

const number = (over: Partial<ModOption> = {}): ModOption => ({
	key: 'maxunits',
	name: 'Max Units',
	type: 'number',
	def: 2000,
	min: 500,
	max: 32000,
	step: 1,
	...over,
})

const list = (over: Partial<ModOption> = {}): ModOption => ({
	key: 'deathmode',
	name: 'Game End Mode',
	type: 'list',
	def: 'com',
	items: [
		{ key: 'com', name: 'Kill all Commanders', desc: 'No commanders left' },
		{ key: 'killall', name: 'Kill everything' },
	],
	...over,
})

const bool = (over: Partial<ModOption> = {}): ModOption => ({
	key: 'evocom',
	name: 'Evolving Commanders',
	type: 'bool',
	def: false,
	...over,
})

const row = (option: ModOption, current: string | null = null) =>
	rowsOf(
		{ name: '', options: [option] },
		current === null ? {} : { [option.key]: current },
	)[0]!

describe('what a default reads as', () => {
	test('a number is itself', () => {
		expect(defaultWords(number())).toBe('2000')
	})

	test('a list is the name of its item', () => {
		expect(defaultWords(list())).toBe('Kill all Commanders')
	})

	test('a switch is on or off', () => {
		expect(defaultWords(bool())).toBe('off')
		expect(defaultWords(bool({ def: true }))).toBe('on')
	})

	test('an empty text says so', () => {
		expect(defaultWords({ key: 's', type: 'string', def: '' })).toBe('empty')
	})
})

describe('the facts of an option', () => {
	test('a number gives its range, its step and its default', () => {
		expect(facts(number())).toEqual([
			'min 500',
			'max 32000',
			'step 1',
			'default 2000',
		])
	})

	test('a step of nought or less means no steps, so none is claimed', () => {
		expect(facts(number({ step: 0 }))).toEqual([
			'min 500',
			'max 32000',
			'default 2000',
		])
	})

	test('a bound the table does not give is left out', () => {
		expect(facts(number({ min: null, max: null, step: null }))).toEqual([
			'default 2000',
		])
	})

	test('a list gives its default by name', () => {
		expect(facts(list())).toEqual(['default Kill all Commanders'])
	})

	test('a number without a default claims none', () => {
		expect(
			facts(number({ def: null, min: null, max: null, step: null })),
		).toEqual([])
	})

	test('the tooltip spells out the choices a list has', () => {
		expect(tipFacts(list())).toBe(
			'Kill all Commanders · Kill everything · default Kill all Commanders',
		)
	})

	test('the tooltip of a number is its facts on one line', () => {
		expect(tipFacts(number())).toBe(
			'min 500 · max 32000 · step 1 · default 2000',
		)
	})

	test('an option with nothing to say says nothing', () => {
		expect(tipFacts({ key: 'x', type: 'list' })).toBe('')
	})
})

describe('the choices of an option', () => {
	test('a list offers its items, each with what it does', () => {
		expect(choices(list())).toEqual([
			{
				value: 'com',
				label: 'Kill all Commanders',
				desc: 'No commanders left',
			},
			{ value: 'killall', label: 'Kill everything', desc: '' },
		])
	})

	test('a switch offers off and on', () => {
		expect(choices(bool()).map((choice) => choice.label)).toEqual(['off', 'on'])
	})

	test('a number offers none', () => {
		expect(choices(number())).toEqual([])
	})

	test('the one lit is the value, however a switch arrived', () => {
		expect(litChoice(bool(), 'true')).toBe('1')
		expect(litChoice(bool(), '0')).toBe('0')
		expect(litChoice(list(), 'killall')).toBe('killall')
	})
})

describe('going back to the default', () => {
	test('sends the default as the protocol reads it', () => {
		expect(resetValue(number(), true)).toBe('2000')
		expect(resetValue(bool({ def: true }), true)).toBe('1')
		expect(resetValue(list(), true)).toBe('com')
	})

	test('an empty text is cleared with a 0 where a host would skip an empty value', () => {
		const text: ModOption = { key: 'map_tweaklava', type: 'string', def: '' }
		expect(resetValue(text, true)).toBe('0')
		expect(resetValue(text, false)).toBe('')
	})

	test('a switch with no default goes off', () => {
		expect(resetValue({ key: 'x', type: 'bool' }, false)).toBe('0')
	})
})

describe('a text cleared by a host', () => {
	const text: ModOption = { key: 'map_tweaklava', type: 'string', def: '' }

	test('is not a change', () => {
		expect(row(text, '0').changed).toBe(false)
	})

	test('reads as empty', () => {
		expect(displayText(row(text, '0'))).toBe('empty')
	})

	test('a text set to something is a change', () => {
		expect(row(text, 'abc').changed).toBe(true)
	})
})

describe('a link', () => {
	const evocom: ModOption = {
		key: 'evocom',
		name: 'Evolving Commanders',
		type: 'bool',
		section: 'options_extra',
	}
	const thread: ModOption = {
		key: 'evocom_feedback_thread_link',
		name: 'Feedback thread',
		desc: 'Discord discussion about Evolving Commanders.',
		type: 'link',
		section: 'options_extra',
		link: 'https://discord.com/channels/1/2',
	}
	const channel: ModOption = {
		key: 'raptors_dev_channel_link',
		name: 'Development Discussion',
		desc: 'Raptors development discussion.',
		type: 'link',
		section: 'raptor_defense_options',
		link: 'https://discord.com/channels/3/4',
	}

	test('belongs to the option it follows and whose key it extends', () => {
		const { byKey, bySection } = linksOf([evocom, thread])
		expect(byKey.get('evocom')).toEqual([
			{
				url: 'https://discord.com/channels/1/2',
				desc: 'Discord discussion about Evolving Commanders',
				host: 'discord.com',
			},
		])
		expect(bySection.size).toBe(0)
	})

	test('following anything else, it belongs to its tab', () => {
		const coop: ModOption = {
			key: 'coop',
			type: 'bool',
			section: 'options_main',
		}
		const { byKey, bySection } = linksOf([coop, channel])
		expect(byKey.size).toBe(0)
		expect(bySection.get('raptor_defense_options')?.[0]?.desc).toBe(
			'Raptors development discussion',
		)
	})

	test('a key that only starts the same is not its owner', () => {
		const evo: ModOption = {
			key: 'evo',
			type: 'bool',
			section: 'options_extra',
		}
		expect(linksOf([evo, thread]).byKey.has('evo')).toBe(false)
	})

	test('layout between an option and its link does not part them', () => {
		const gap: ModOption = {
			key: 'sub_header',
			type: 'separator',
			section: 'options_extra',
		}
		expect(linksOf([evocom, gap, thread]).byKey.has('evocom')).toBe(true)
	})

	test('with no description, it is called what BAR calls it', () => {
		const bare = { ...thread, desc: '' }
		expect(linksOf([evocom, bare]).byKey.get('evocom')?.[0]?.desc).toBe(
			'Feedback thread',
		)
	})

	test('is no row of any tab', () => {
		const section: ModOption = {
			key: 'options_extra',
			name: 'Extras',
			type: 'section',
			weight: 1,
		}
		const keys = tabs([section, evocom, thread])
			.flatMap((tab) => tab.groups)
			.flatMap((group) => group.options.map((option) => option.key))
		expect(keys).toContain('evocom')
		expect(keys).not.toContain('evocom_feedback_thread_link')
	})
})

describe('where a link goes', () => {
	test('is its host', () => {
		expect(linkHost('https://gist.github.com/RebelNode/43b9')).toBe(
			'gist.github.com',
		)
		expect(linkHost('http://example.com/a')).toBe('example.com')
	})

	test('is nowhere for what no lobby should open', () => {
		expect(linkHost('javascript:alert(1)')).toBe('')
		expect(linkHost('file:///C:/secret')).toBe('')
		expect(linkHost('not a url')).toBe('')
		expect(linkHost(undefined)).toBe('')
	})
})

describe('what the lobby has been doing with an option', () => {
	const change = (seq: number, key: string) => ({
		seq,
		key,
		from: 'a',
		to: 'b',
		by: null,
		at: seq,
	})

	test('the latest change to a key is the last one', () => {
		const history = [change(1, 'x'), change(2, 'y'), change(3, 'x')]
		expect(latestChange(history, 'x')?.seq).toBe(3)
		expect(latestChange(history, 'z')).toBeUndefined()
	})

	const vote = (proposal: unknown) =>
		({
			command: 'bSet x 1',
			by: null,
			proposal,
			yes: 1,
			yesNeeded: 2,
			no: 0,
			noNeeded: 2,
			remainingSecs: 30,
		}) as never

	test('a vote to set this option is that vote', () => {
		const held = vote({ type: 'setOption', key: 'x', value: '1' })
		expect(voteOn(held, 'x')).toEqual({
			type: 'setOption',
			key: 'x',
			value: '1',
		})
	})

	test('a vote to set another, or to do something else, is not', () => {
		expect(
			voteOn(vote({ type: 'setOption', key: 'y', value: '1' }), 'x'),
		).toBeNull()
		expect(voteOn(vote({ type: 'other' }), 'x')).toBeNull()
		expect(voteOn(null, 'x')).toBeNull()
	})
})

describe('options another option switches off', () => {
	const mode = list({
		items: [
			{ key: 'com', name: 'Kill all Commanders', lock: ['threshold'] },
			{ key: 'dominion', name: 'Dominion', unlock: ['threshold'] },
		],
	})

	test('the chosen item locks what it names', () => {
		const locks = locksOf([mode], {})
		expect(locks.get('threshold')).toEqual({
			by: 'Game End Mode',
			value: 'Kill all Commanders',
		})
	})

	test('choosing another item lets it go', () => {
		expect(locksOf([mode], { deathmode: 'dominion' }).has('threshold')).toBe(
			false,
		)
	})

	test('a switch locks its `unlock` list while off and its `lock` list while on', () => {
		const lava = bool({
			key: 'lava',
			name: 'Water Is Lava',
			unlock: ['tide'],
			lock: ['calm'],
		})
		expect(locksOf([lava], {}).get('tide')).toEqual({
			by: 'Water Is Lava',
			value: 'off',
		})
		expect(locksOf([lava], {}).has('calm')).toBe(false)

		const on = locksOf([lava], { lava: '1' })
		expect(on.has('tide')).toBe(false)
		expect(on.get('calm')).toEqual({ by: 'Water Is Lava', value: 'on' })
	})

	test('a number locks its `unlock` list on its default and its `lock` list off it', () => {
		const amount = number({
			key: 'amount',
			name: 'Amount',
			unlock: ['a'],
			lock: ['b'],
		})
		expect(locksOf([amount], {}).has('a')).toBe(true)
		expect(locksOf([amount], {}).has('b')).toBe(false)
		expect(locksOf([amount], { amount: '2000.0' }).has('a')).toBe(true)
		const moved = locksOf([amount], { amount: '3000' })
		expect(moved.has('a')).toBe(false)
		expect(moved.get('b')).toEqual({ by: 'Amount', value: '3000' })
	})

	test('a switch that names itself only tucks options away, and holds none of them off', () => {
		// BAR's `dummyboolfeelfreetotouch`, which hides what it names from
		// Chobby's summary: all of those are in force whatever it says.
		const dummy = bool({
			key: 'dummy',
			name: 'Dummy',
			unlock: ['dummy', 'other'],
		})
		const locks = locksOf([dummy], {})
		expect(locks.has('dummy')).toBe(false)
		expect(locks.has('other')).toBe(false)
	})

	test('a key held by two options names the first', () => {
		const one = bool({ key: 'one', name: 'One', unlock: ['x'] })
		const two = bool({ key: 'two', name: 'Two', unlock: ['x'] })
		expect(locksOf([one, two], {}).get('x')?.by).toBe('One')
	})
})

describe("a tab's own prose", () => {
	const section: ModOption = {
		key: 'options_cheats',
		name: 'Cheats',
		type: 'section',
		weight: 1,
	}
	const sub = (name: string): ModOption => ({
		key: 'sub_header',
		name,
		section: 'options_cheats',
		type: 'subheader',
	})
	const setting: ModOption = {
		key: 'startmetal',
		section: 'options_cheats',
		type: 'number',
	}
	const cheats = (options: ModOption[]) =>
		tabs([section, ...options]).find((tab) => tab.key === 'options_cheats')!

	test('is what BAR writes between the group names, in its order', () => {
		const tab = cheats([
			sub('Warning: at your own risk!'),
			sub('No guarantee they work.'),
			sub('-- Starting Resources'),
			setting,
		])
		expect(tab.intro).toEqual([
			'Warning: at your own risk!',
			'No guarantee they work.',
		])
		expect(tab.groups.map((group) => group.name)).toEqual([
			'Starting Resources',
		])
	})

	test('leaves out the blank ones BAR uses as spacing', () => {
		expect(cheats([sub(''), sub('   '), setting]).intro).toEqual([])
	})

	test('is nothing on the tabs we make ourselves', () => {
		const made = tabs([section, setting]).filter(
			(tab) => tab.key === 'modding' || tab.key === 'map',
		)
		for (const tab of made) expect(tab.intro).toEqual([])
	})
})
