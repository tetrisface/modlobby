import { describe, expect, test } from 'vitest'
import {
	EVERY_ROW,
	arrange,
	GENERAL_GROUP,
	MAP_TAB,
	MODDING_TAB,
	TWEAK_SLOTS,
	byRunOrder,
	changedByTab,
	changedCount,
	changedView,
	isTweakSlot,
	nextTweak,
	rowsByGroup,
	rowsByTab,
	defaultText,
	displayText,
	drawnOrder,
	keepOrder,
	readModOptions,
	regroup,
	rowsOf,
	searchRows,
	sortRows,
	sortSections,
	tabs,
	tweakKey,
	type Grouping,
	type Row,
	type RowOrder,
	type Shown,
	type Tab,
} from './setup'
import { fixtureOptions } from './setup.fixture'

const CHANGED: Shown = (row) => row.changed
const TABS = tabs(fixtureOptions())
const byKey = (key: string): Tab => {
	const tab = TABS.find((entry) => entry.key === key)
	if (!tab) throw new Error(`no tab ${key}`)
	return tab
}
const optionKeys = (tab: Tab) =>
	tab.groups.flatMap((group) => group.options.map((option) => option.key))

describe('tabs', () => {
	test('are BAR sections by weight, then Modding, then Map', () => {
		expect(TABS.map((tab) => tab.name)).toEqual([
			'Main',
			'Raptors',
			'Scavengers',
			'Extras',
			'Experimental',
			'Other',
			'Cheats',
			// Chobby drops `_DEV`; a modding lobby shows it.
			'DEV',
			'Modding',
			'Map',
		])
	})

	test('Map holds the three metadata options, hidden or not, and nothing else', () => {
		const map = byKey(MAP_TAB)
		expect(optionKeys(map)).toEqual([
			'mapmetadata_startpos',
			'mapmetadata_startboxes_set',
			'mapmetadata_startbox_override',
		])
		expect(map.groups.map((group) => group.name)).toEqual(['Map metadata'])
		expect(map.groups[0]?.options.map((option) => option.name)).toEqual([
			'StartPos',
			'Startboxes Set',
			'Startbox Override',
		])
		for (const tab of TABS.filter((entry) => entry.key !== MAP_TAB)) {
			expect(optionKeys(tab)).not.toContain('mapmetadata_startbox_override')
		}
	})

	test('there is no Map tab for a game without the section', () => {
		const without = fixtureOptions().filter(
			(option) =>
				option.key !== 'mapmetadata' && option.section !== 'mapmetadata',
		)
		expect(tabs(without).map((tab) => tab.name)).not.toContain('Map')
	})

	test('Cheats keeps its name and its balance settings', () => {
		const cheats = byKey('options_cheats')
		const keys = optionKeys(cheats)
		expect(keys).toContain('startmetal')
		expect(keys).toContain('multiplier_buildpower')
		expect(keys).toContain('dynamiccheats')
		expect(keys.length).toBeGreaterThan(20)
	})

	test('groups inside Cheats are BAR own subheaders', () => {
		// BAR's trailing `-- Other` keeps its two hidden options once the four
		// that move to Modding have gone.
		expect(byKey('options_cheats').groups.map((group) => group.name)).toEqual([
			'AI Cheats',
			'Starting Resources',
			'Resource Multipliers',
			'Unit Parameter Multipliers',
			'Other',
		])
	})

	test('every modding option leaves its old tab exactly once', () => {
		const moved = [
			'tweakdefs',
			'tweakunits',
			'forceallunits',
			'experimentallegionfaction',
			'experimentalextraunits',
			'scavunitsforplayers',
		]
		const modding = optionKeys(byKey(MODDING_TAB))
		for (const key of moved) {
			expect(modding).toContain(key)
			const elsewhere = TABS.filter((tab) => tab.key !== MODDING_TAB).filter(
				(tab) => optionKeys(tab).includes(key),
			)
			expect(elsewhere.map((tab) => tab.name)).toEqual([])
		}
	})

	test('tweak slots are all twenty, units before defs as BAR runs them', () => {
		expect(TWEAK_SLOTS).toHaveLength(20)
		expect(TWEAK_SLOTS[0]).toBe('tweakunits')
		expect(TWEAK_SLOTS[9]).toBe('tweakunits9')
		expect(TWEAK_SLOTS[10]).toBe('tweakdefs')
		expect(TWEAK_SLOTS[19]).toBe('tweakdefs9')
	})

	test('a slot past the twenty the room used joins them, in run order', () => {
		const slots = tabs(fixtureOptions(), ['tweakdefs12', 'tweakunits20'])
			.find((tab) => tab.key === MODDING_TAB)!
			.groups[0]!.options.map((option) => option.key)
		expect(slots).toHaveLength(22)
		expect(slots.slice(8, 12)).toEqual([
			'tweakunits8',
			'tweakunits9',
			'tweakunits20',
			'tweakdefs',
		])
		expect(slots.at(-1)).toBe('tweakdefs12')
	})

	test('a tweak key is any index BAR reads, and nothing else', () => {
		expect(tweakKey('tweakunits')).toEqual({ kind: 'units', index: 0 })
		expect(tweakKey('tweakdefs29')).toEqual({ kind: 'defs', index: 29 })
		expect(tweakKey('tweakdefs01')).toBeNull()
		expect(tweakKey('tweakdefs256')).toBeNull()
		expect(tweakKey('map_tweaklava')).toBeNull()
		expect(
			['tweakdefs', 'tweakunits12', 'tweakdefs2', 'tweakunits'].sort(
				byRunOrder,
			),
		).toEqual(['tweakunits', 'tweakunits12', 'tweakdefs', 'tweakdefs2'])
	})

	test('a hidden option is a row in the group BAR declares it in, still marked', () => {
		// `holiday_events` is declared hidden in the Cheats section; Chobby drops it.
		const other = byKey('options_cheats').groups.at(-1)!
		const holiday = other.options.find(
			(option) => option.key === 'holiday_events',
		)
		expect(holiday?.hidden).toBe(true)
	})

	test('a section with nothing in it is no tab', () => {
		const empty = [
			...fixtureOptions(),
			{ key: 'modes', name: 'GameModes', type: 'section', hidden: true },
		] as ReturnType<typeof fixtureOptions>
		expect(tabs(empty).map((tab) => tab.key)).not.toContain('modes')
	})
})

describe('changes against BAR defaults', () => {
	const values = readModOptions({
		'game/modoptions/startmetal': '2000',
		'game/modoptions/multiplier_buildpower': '1.5',
		'game/modoptions/startenergy': '1000',
		'game/modoptions/dynamiccheats': '1',
		'game/modoptions/tweakdefs1': 'bG9jYWw=',
		'game/hosttype': 'SPADS',
		'game/players/bob/skill': '[14]',
	})

	test('reads only the modoption tags', () => {
		expect(values).toEqual({
			startmetal: '2000',
			multiplier_buildpower: '1.5',
			startenergy: '1000',
			dynamiccheats: '1',
			tweakdefs1: 'bG9jYWw=',
		})
	})

	test('a value equal to the default is not a change', () => {
		// startenergy's default is 1000, and dynamiccheats defaults to true.
		const cheats = byKey('options_cheats')
		const changed = cheats.groups
			.flatMap((group) => rowsOf(group, values))
			.filter((row) => row.changed)
			.map((row) => row.option.key)
		expect(changed.sort()).toEqual(['multiplier_buildpower', 'startmetal'])
	})

	test('a number compares numerically, not as text', () => {
		const rows = rowsOf(
			byKey('options_cheats').groups.find(
				(group) => group.name === 'Starting Resources',
			)!,
			readModOptions({ 'game/modoptions/startmetal': '1000.0' }),
		)
		const metal = rows.find((row) => row.option.key === 'startmetal')
		expect(metal?.changed).toBe(false)
	})

	test('the tab badge counts what the tab shows', () => {
		expect(changedCount(byKey('options_cheats'), values)).toBe(2)
		expect(changedCount(byKey('raptor_defense_options'), values)).toBe(0)
	})

	test('the All tab lists every changed setting under the tab it lives in', () => {
		const changed = changedByTab(TABS, values)
		expect(changed.map((entry) => entry.tab.name)).toEqual([
			'Cheats',
			'Modding',
		])
		expect(
			changed.map((entry) => entry.rows.map((row) => row.option.key).sort()),
		).toEqual([['multiplier_buildpower', 'startmetal'], ['tweakdefs1']])
	})

	test('a room on BAR defaults has nothing under All', () => {
		expect(changedByTab(TABS, {})).toEqual([])
	})

	test('a map option is changed when set and not when SPADS cleared it to 0', () => {
		const map = byKey(MAP_TAB)
		const set = readModOptions({
			'game/modoptions/mapmetadata_startbox_override': 'eJyrVjJS',
			'game/modoptions/mapmetadata_startboxes_set': '0',
			'game/modoptions/mapmetadata_startpos': '',
		})
		const rows = rowsOf(map.groups[0]!, set)
		expect(rows.map((row) => [row.option.key, row.changed])).toEqual([
			['mapmetadata_startpos', false],
			['mapmetadata_startboxes_set', false],
			['mapmetadata_startbox_override', true],
		])
		expect(changedCount(map, set)).toBe(1)
		expect(changedByTab(TABS, set).map((entry) => entry.tab.name)).toEqual([
			'Map',
		])
		expect(displayText(rows[2]!)).toBe('8 B')
		expect(displayText(rows[1]!)).toBe('none')
	})

	test('showing the unchanged too lists every tab with all of its rows', () => {
		const all = rowsByTab(TABS, values, EVERY_ROW)
		expect(all.map((entry) => entry.tab.name)).toEqual(
			TABS.map((tab) => tab.name),
		)
		const cheats = all.find((entry) => entry.tab.key === 'options_cheats')!
		expect(cheats.rows.length).toBeGreaterThan(20)
		expect(cheats.rows.filter((row) => row.changed)).toHaveLength(2)
		const modding = all.find((entry) => entry.tab.key === MODDING_TAB)!
		expect(modding.rows.filter(isTweakSlot)).toHaveLength(20)
		expect(cheats.rows.some(isTweakSlot)).toBe(false)
	})

	test("a tab's changes come under BAR's own groups", () => {
		const cheats = rowsByGroup(byKey('options_cheats'), values, CHANGED)
		expect(cheats.map((entry) => entry.name)).toEqual([
			'Starting Resources',
			'Resource Multipliers',
		])
		expect(
			cheats.map((entry) => entry.rows.map((row) => row.option.key)),
		).toEqual([['startmetal'], ['multiplier_buildpower']])
	})

	test('showing the unchanged inside a tab lists every group with all of its rows', () => {
		const tab = byKey('options_cheats')
		const all = rowsByGroup(tab, values, EVERY_ROW)
		expect(all.map((entry) => entry.name)).toEqual(
			tab.groups.map((group) => group.name),
		)
		expect(all.map((entry) => entry.rows.length)).toEqual(
			tab.groups.map((group) => group.options.length),
		)
		expect(
			all.flatMap((entry) => entry.rows).filter((row) => row.changed),
		).toHaveLength(2)
	})

	test('a group BAR left unnamed is called General', () => {
		const raptors = byKey('raptor_defense_options')
		expect(rowsByGroup(raptors, {}, CHANGED)).toEqual([])
		expect(
			rowsByGroup(raptors, {}, EVERY_ROW).map((entry) => [
				entry.name,
				entry.rows.length,
			]),
		).toEqual([[GENERAL_GROUP, 1]])
	})

	test('the tweak slots are rows of the Modding tab, not a grid', () => {
		const modding = byKey(MODDING_TAB)
		expect(
			rowsByGroup(modding, values, CHANGED).map((entry) => [
				entry.name,
				entry.rows.map((row) => row.option.key),
			]),
		).toEqual([['Tweak slots', ['tweakdefs1']]])
		const slots = rowsByGroup(modding, values, EVERY_ROW)[0]!
		expect(slots.name).toBe('Tweak slots')
		expect(slots.rows).toHaveLength(20)
		expect(slots.rows.every(isTweakSlot)).toBe(true)
	})
})

describe('the next tweak', () => {
	const filled = (...keys: string[]) =>
		Object.fromEntries(keys.map((key) => [key, 'bG9jYWw=']))

	test('goes after the last one filled, so it runs after them', () => {
		expect(nextTweak('defs', {})).toBe('tweakdefs')
		expect(nextTweak('defs', filled('tweakdefs', 'tweakdefs3'))).toBe(
			'tweakdefs4',
		)
		// Each kind counts its own ten.
		expect(nextTweak('units', filled('tweakdefs', 'tweakdefs3'))).toBe(
			'tweakunits',
		)
		// SPADS clears a slot to `0`, which is room.
		expect(nextTweak('defs', { tweakdefs: '0' })).toBe('tweakdefs')
	})

	test('takes the first gap once slot 9 is taken, and nothing when all ten are', () => {
		expect(nextTweak('units', filled('tweakunits', 'tweakunits9'))).toBe(
			'tweakunits1',
		)
		const full = filled(...TWEAK_SLOTS.filter((key) => key.includes('defs')))
		expect(nextTweak('defs', full)).toBeNull()
		expect(nextTweak('units', full)).toBe('tweakunits')
	})

	test('a Changed view shows each kind its empty row, below the filled ones', () => {
		const values = readModOptions({ 'game/modoptions/tweakdefs1': 'bG9jYWw=' })
		const modding = rowsByGroup(byKey(MODDING_TAB), values, changedView(values))
		expect(
			modding.map((entry) => [
				entry.name,
				entry.rows.map((row) => [row.option.key, row.changed]),
			]),
		).toEqual([
			[
				'Tweak slots',
				[
					['tweakunits', false],
					['tweakdefs1', true],
					['tweakdefs2', false],
				],
			],
		])
		// The empty rows are not changes: badges and counts leave them out.
		expect(changedCount(byKey(MODDING_TAB), values)).toBe(1)
	})

	test('a held slot stays in a Changed view after it is cleared', () => {
		const shown = changedView({}, new Set(['tweakunits4']))
		const keys = rowsByTab(TABS, {}, shown).flatMap((entry) =>
			entry.rows.map((row) => row.option.key),
		)
		expect(keys).toEqual(['tweakunits', 'tweakunits4', 'tweakdefs'])
	})
})

describe('search', () => {
	const keys = (found: ReturnType<typeof searchRows>) =>
		found.flatMap((entry) => entry.rows.map((row) => row.option.key))

	test('an empty needle finds nothing, whitespace included', () => {
		expect(searchRows(TABS, {}, '')).toEqual([])
		expect(searchRows(TABS, {}, '   ')).toEqual([])
	})

	// The fixture names every option after its key and describes none, so the
	// name and description rules get a tab of their own.
	const named: Tab = {
		key: 'named',
		name: 'Named',
		desc: '',
		groups: [
			{
				name: '',
				options: [
					{
						key: 'startmetal',
						name: 'Starting Metal',
						desc: 'Metal each player starts with.',
						type: 'number',
						def: '1000',
					},
					{ key: 'other', name: 'Other', desc: '', type: 'bool', def: '0' },
				],
			},
		],
	}

	test('matches a word of the name, in any case', () => {
		const found = searchRows([named], {}, 'STARTING METAL')
		expect(keys(found)).toEqual(['startmetal'])
		expect(found.map((entry) => entry.tab.name)).toEqual(['Named'])
	})

	test('matches the key when the name says something else', () => {
		expect(keys(searchRows(TABS, {}, 'multiplier_buildpower'))).toEqual([
			'multiplier_buildpower',
		])
	})

	test('matches the description', () => {
		expect(keys(searchRows([named], {}, 'each player'))).toEqual(['startmetal'])
	})

	test('every word has to match', () => {
		expect(keys(searchRows(TABS, {}, 'start metal'))).toContain('startmetal')
		expect(keys(searchRows(TABS, {}, 'start metal zzzz'))).toEqual([])
	})

	test('a tweak slot is found by its key', () => {
		const found = searchRows(TABS, { tweakunits2: 'e30=' }, 'tweakunits2')
		expect(keys(found)).toEqual(['tweakunits2'])
		expect(found[0]?.tab.key).toBe(MODDING_TAB)
		expect(found[0]?.rows[0]?.changed).toBe(true)
	})

	test('matches the value the room holds', () => {
		expect(keys(searchRows([named], { startmetal: '2500' }, '2500'))).toEqual([
			'startmetal',
		])
		expect(keys(searchRows([named], {}, '2500'))).toEqual([])
	})

	test('matches the value as the row shows it, and as it arrived', () => {
		const on = searchRows([named], { other: 'true' }, 'other on')
		expect(keys(on)).toEqual(['other'])
		expect(keys(searchRows([named], { other: 'true' }, 'other true'))).toEqual([
			'other',
		])
		expect(keys(searchRows([named], {}, 'other on'))).toEqual([])
	})

	test('matches a list item by its name or its key', () => {
		const values = { nowasting: 'disabled' }
		expect(keys(searchRows(TABS, values, 'nowasting Disabled'))).toEqual([
			'nowasting',
		])
		expect(keys(searchRows(TABS, values, 'nowasting disabled'))).toEqual([
			'nowasting',
		])
	})

	test('a tweak blob is never searched', () => {
		expect(keys(searchRows(TABS, { tweakunits2: 'e30=' }, 'e30'))).toEqual([])
	})
})

describe('display', () => {
	const cheats = byKey('options_cheats')
	const row = (key: string, values: Record<string, string>) =>
		cheats.groups
			.flatMap((group) => rowsOf(group, values))
			.find((entry) => entry.option.key === key)!

	test('a bool reads as on or off, however it arrived', () => {
		expect(displayText(row('dynamiccheats', { dynamiccheats: '0' }))).toBe(
			'off',
		)
		expect(displayText(row('dynamiccheats', { dynamiccheats: 'true' }))).toBe(
			'on',
		)
	})

	test('a list shows the item name, not its key', () => {
		expect(displayText(row('nowasting', { nowasting: 'disabled' }))).toBe(
			'Disabled',
		)
	})

	test('an unset row falls back to the default it sits on', () => {
		expect(displayText(row('startmetal', {}))).toBe('1000')
		expect(defaultText(row('dynamiccheats', {}).option)).toBe('1')
	})
})

describe('order', () => {
	const row = (key: string, name: string, changed = false) =>
		({ option: { key, name, type: 'bool' }, current: null, changed }) as Row

	test('changed first keeps BAR order among equals; by name reads A to Z', () => {
		const rows = [row('b', 'Beta'), row('c', 'Gamma', true), row('a', 'Alpha')]
		const keys = (order: RowOrder) =>
			sortRows(rows, order).map((r) => r.option.key)
		expect(keys('changed')).toEqual(['c', 'b', 'a'])
		expect(keys('name')).toEqual(['a', 'b', 'c'])
	})

	test('changed first leads with the headings holding a change; by name keeps BAR order', () => {
		const sections = [
			{ name: 'Main', rows: [row('a', 'Alpha')] },
			{ name: 'Cheats', rows: [row('b', 'Beta'), row('c', 'Gamma', true)] },
			{ name: 'Extras', rows: [row('d', 'Delta')] },
		]
		const names = (order: RowOrder) =>
			sortSections(sections, order).map((section) => section.name)
		expect(names('changed')).toEqual(['Cheats', 'Main', 'Extras'])
		expect(names('name')).toEqual(['Main', 'Cheats', 'Extras'])
	})

	test('a kept order holds headings and rows in place, with fresh rows', () => {
		const drawn = drawnOrder([
			{ name: 'Main', rows: [row('a', 'Alpha'), row('b', 'Beta')] },
			{ name: 'Extras', rows: [row('d', 'Delta')] },
		])
		// Beta was changed: on its own, that sorts it first and its heading too.
		const fresh = [
			{ name: 'Extras', rows: [row('d', 'Delta'), row('e', 'Epsilon', true)] },
			{ name: 'Main', rows: [row('b', 'Beta', true), row('a', 'Alpha')] },
			{ name: 'New', rows: [row('n', 'Nu')] },
		]
		const kept = keepOrder(fresh, drawn)
		expect(
			kept.map((section) => [
				section.name,
				section.rows.map((r) => r.option.key),
			]),
		).toEqual([
			['Main', ['a', 'b']],
			['Extras', ['d', 'e']],
			['New', ['n']],
		])
		// The row is the fresh one, so it shows the change it is being held for.
		expect(kept[0]!.rows[1]!.changed).toBe(true)
	})

	test('a kept order leaves out what is gone', () => {
		const drawn = drawnOrder([
			{ name: 'Main', rows: [row('a', 'Alpha'), row('b', 'Beta')] },
			{ name: 'Gone', rows: [row('g', 'Gamma')] },
		])
		const kept = keepOrder([{ name: 'Main', rows: [row('b', 'Beta')] }], drawn)
		expect(kept).toEqual([{ name: 'Main', rows: [row('b', 'Beta')] }])
	})

	test('rows regroup by type, or into one list with no heading', () => {
		const typed = (key: string, type: string, section?: string) =>
			({
				option: { key, name: key, type, section },
				current: null,
				changed: false,
			}) as Row
		const sections = [
			{
				name: 'Main',
				rows: [typed('ranked', 'bool'), typed('start', 'number')],
			},
			{
				name: 'Modding',
				rows: [typed('tweakdefs', 'string'), typed('lava', 'string')],
			},
			{
				name: 'Map',
				rows: [typed('override', 'string', 'mapmetadata'), typed('x', 'link')],
			},
		]
		const keys = (grouping: Grouping) =>
			regroup(sections, grouping).map((section) => [
				section.name,
				section.rows.map((r) => r.option.key),
			])
		expect(regroup(sections, 'section')).toBe(sections)
		expect(keys('none')).toEqual([
			['', ['ranked', 'start', 'tweakdefs', 'lava', 'override', 'x']],
		])
		expect(keys('type')).toEqual([
			['Documents', ['tweakdefs', 'override']],
			['Switches', ['ranked']],
			['Numbers', ['start']],
			['Text', ['lava']],
			['Other', ['x']],
		])
		expect(regroup([], 'none')).toEqual([])
	})

	test('ungrouped, Sort is about the options alone, tweak slots among them', () => {
		const sections = [
			{
				name: 'Main',
				rows: [row('ranked', 'Ranked'), row('metal', 'Metal', true)],
			},
			{
				name: 'Modding',
				rows: [
					row('tweakunits', 'tweakunits'),
					row('tweakdefs', 'tweakdefs', true),
				],
			},
		]
		const keys = (grouping: Grouping, order: RowOrder) =>
			arrange(sections, grouping, order).map((section) =>
				section.rows.map((r) => r.option.key),
			)
		expect(keys('none', 'name')).toEqual([
			['metal', 'ranked', 'tweakdefs', 'tweakunits'],
		])
		expect(keys('none', 'changed')).toEqual([
			['metal', 'tweakdefs', 'ranked', 'tweakunits'],
		])
		// Under a heading the slots keep BAR's run order, and headings sort too.
		expect(keys('section', 'name')).toEqual([
			['metal', 'ranked'],
			['tweakunits', 'tweakdefs'],
		])
	})

	test('tweak slots stay first, in the order BAR runs them', () => {
		const rows = [
			row('tweakunits', 'tweakunits'),
			row('tweakdefs', 'tweakdefs', true),
			row('forceallunits', 'Load every unit'),
			row('experimentallegionfaction', 'Legion', true),
		]
		expect(sortRows(rows, 'name').map((r) => r.option.key)).toEqual([
			'tweakunits',
			'tweakdefs',
			'experimentallegionfaction',
			'forceallunits',
		])
	})
})
