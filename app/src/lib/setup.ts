/**
 * BAR's modoptions, arranged into the tabs a room shows.
 *
 * The table is the game's own `modoptions.lua`, read out of the copy already
 * installed on this machine and parsed by the `modoptions` crate. It is not
 * shipped with the app: the descriptions in it are BAR's writing under GPL v2,
 * and a lobby has no need to redistribute them when every player already has
 * the file. bar-lobby reads it the same way (`game-provider.ts:200`), and
 * Chobby asks the engine's Lua VM for it.
 *
 * The happy side effect is that the table can never be out of date with the
 * game a room is actually running.
 *
 * Tab order, group names and defaults are all BAR's own — the one thing we
 * impose is the Modding tab.
 */

import type { ModOption } from '../ipc/bindings/ModOption'
import type { OptionValue } from '../ipc/bindings/OptionValue'
import { hasEveryWord } from './search'

export type Group = { name: string; options: ModOption[] }
export type Tab = { key: string; name: string; desc: string; groups: Group[] }

export type Row = {
	option: ModOption
	/** What the room has set, if anything. */
	current: string | null
	changed: boolean
}

/** Chobby nulls this section outright (`gui_modoptions_panel.lua:1242`). */
const DROPPED_SECTION = 'dev'

/**
 * The six options we lift into a Modding tab, with the groups they land in.
 *
 * They are one mechanism: each decides which unit definitions exist. BAR's own
 * `forceallunits` description — "Load all UnitDefs even if ais or options for
 * them aren't enabled" — exists to serve the other five and the tweak slots.
 * `section` is only a lobby display hint ("so lobbies can order options in
 * categories/panels"), so moving them changes nothing on the wire.
 */
const MODDING_GROUPS: ReadonlyArray<readonly [string, readonly string[]]> = [
	[
		'Unit packs',
		[
			'experimentallegionfaction',
			'experimentalextraunits',
			'scavunitsforplayers',
		],
	],
	['Loading', ['forceallunits']],
]

/**
 * The two slots BAR declares by hand, both in Cheats. Slots 1-29 come from
 * `for` loops, which the parser does not read (it reads the `local options`
 * table), so they appear nowhere else.
 */
const DECLARED_TWEAK_SLOTS = ['tweakdefs', 'tweakunits']

const MOVED = new Set([
	...MODDING_GROUPS.flatMap(([, keys]) => keys),
	...DECLARED_TWEAK_SLOTS,
])

/**
 * The 20 slots written from here, in the order BAR runs them since #6597:
 * every tweakunits, then every tweakdefs. BAR declares 1-29 (hidden) and reads
 * any index; a higher slot somebody else set is shown too, never written.
 */
export const TWEAK_SLOTS: readonly string[] = ['units', 'defs'].flatMap(
	(kind) =>
		['', '1', '2', '3', '4', '5', '6', '7', '8', '9'].map(
			(index) => `tweak${kind}${index}`,
		),
)

/** `tweakunits`, `tweakdefs7`, `tweakdefs29`; a leading zero is some other key. */
const TWEAK_KEY = /^tweak(units|defs)([1-9]\d*)?$/

/** A tweak key's kind and index, or nothing for any other key. */
export function tweakKey(
	key: string,
): { kind: 'units' | 'defs'; index: number } | null {
	const match = TWEAK_KEY.exec(key)
	if (!match) return null
	const index = match[2] === undefined ? 0 : Number(match[2])
	// The index travels to Rust as a byte.
	if (index > 255) return null
	return { kind: match[1] as 'units' | 'defs', index }
}

/** Tweak keys in the order BAR runs them: units, then defs, each by index. */
export function byRunOrder(a: string, b: string): number {
	const [x, y] = [tweakKey(a), tweakKey(b)]
	if (!x || !y) return 0
	return (
		Number(x.kind === 'defs') - Number(y.kind === 'defs') || x.index - y.index
	)
}

export const MODDING_TAB = 'modding'

/**
 * BAR's `mapmetadata` section: the three modoptions SPADS and the lobby set
 * from the map's metadata (start boxes, a custom box arrangement, fixed start
 * positions). BAR declares the section and every option in it hidden, since
 * their values are base64 blobs no row can show. Rather than drop them we
 * give them a tab of their own, with each blob described in words.
 */
export const MAP_TAB = 'map'
const MAP_SECTION = 'mapmetadata'

export function isMapOption(option: ModOption): boolean {
	return option.section === MAP_SECTION
}

/**
 * Not one of BAR's tabs: the one that shows what is changed in all of them.
 * A room's four changed settings are spread over three tabs, and finding
 * them was a tour.
 */
export const ALL_TAB = 'all'

/**
 * Groups come from BAR's own `-- Name` subheaders. Plain subheaders are prose
 * for the tab, separators are spacing, and both are layout rather than
 * settings, so neither becomes a row.
 */
function groupsOf(options: ModOption[]): Group[] {
	const groups: Group[] = []
	let current: Group = { name: '', options: [] }

	for (const option of options) {
		if (option.type === 'separator') continue
		if (option.type === 'subheader') {
			const label = option.name ?? ''
			if (!label.startsWith('--')) continue
			if (current.options.length > 0) groups.push(current)
			current = { name: label.replace(/^--\s*/, '').trim(), options: [] }
			continue
		}
		if (option.hidden) continue
		current.options.push(option)
	}

	if (current.options.length > 0) groups.push(current)
	return groups
}

function moddingTab(options: ModOption[], extra: readonly string[]): Tab {
	const byKey = new Map(options.map((option) => [option.key, option]))
	const groups: Group[] = [
		{
			name: 'Tweak slots',
			options: [...TWEAK_SLOTS, ...extra].sort(byRunOrder).map((key) => ({
				key,
				name: key,
				desc: 'Base64url Lua carried as a modoption.',
				type: 'string',
				def: '',
			})),
		},
	]

	for (const [name, keys] of MODDING_GROUPS) {
		const options = keys
			.map((key) => byKey.get(key))
			.filter((option): option is ModOption => option !== undefined)
		if (options.length > 0) groups.push({ name, options })
	}

	return {
		key: MODDING_TAB,
		name: 'Modding',
		desc: 'What unit definitions the game loads, and the Lua that rewrites them.',
		groups,
	}
}

/**
 * The Map tab, when the game declares the section: its options in file
 * order, hidden or not, named without BAR's `Map Metadata: ` prefix since
 * the tab already says so. Nothing when the game has no such section.
 */
function mapTab(options: ModOption[]): Tab | null {
	const rows = options
		.filter(
			(option) =>
				isMapOption(option) &&
				option.type !== 'subheader' &&
				option.type !== 'separator',
		)
		.map((option) => ({
			...option,
			name: (option.name ?? option.key).replace(/^Map Metadata:\s*/i, ''),
		}))
	if (rows.length === 0) return null
	return {
		key: MAP_TAB,
		name: 'Map',
		desc: 'What the map brings to the room: its start boxes, any custom arrangement, and fixed start positions.',
		groups: [{ name: 'Map metadata', options: rows }],
	}
}

/**
 * Tabs in Chobby's order: weight descending, with an unweighted section
 * treated as zero so it lands between Experimental and Cheats. Modding goes
 * next to Cheats, where the tweak slots used to live, and Map last.
 *
 * `extra` is tweak slots past the twenty that the room has used.
 */
export function tabs(
	options: ModOption[],
	extra: readonly string[] = [],
): Tab[] {
	const sections = options
		.filter(
			(option) =>
				option.type === 'section' &&
				!option.hidden &&
				option.key !== DROPPED_SECTION,
		)
		.sort((a, b) => (b.weight ?? 0) - (a.weight ?? 0))

	const declared = sections.map((section) => ({
		key: section.key,
		name: section.name ?? section.key,
		desc: section.desc ?? '',
		groups: groupsOf(
			options.filter(
				(option) => option.section === section.key && !MOVED.has(option.key),
			),
		),
	}))

	const map = mapTab(options)
	return [
		...declared,
		moddingTab(options, extra),
		...(map === null ? [] : [map]),
	]
}

/** Modoptions the room has set, keyed without the `game/modoptions/` prefix. */
export function readModOptions(
	scriptTags: Record<string, string> | undefined,
): Record<string, string> {
	const values: Record<string, string> = {}
	if (!scriptTags) return values

	for (const [key, value] of Object.entries(scriptTags)) {
		const name = /^game\/modoptions\/(.+)$/.exec(key)?.[1]
		if (name !== undefined) values[name] = value
	}
	return values
}

/** How Lua's default reads once it has been through the protocol. */
export function defaultText(option: ModOption): string {
	const def: OptionValue | null | undefined = option.def
	if (def === null || def === undefined) return ''
	if (typeof def === 'boolean') return def ? '1' : '0'
	return String(def)
}

export function isOn(text: string): boolean {
	return text === '1' || text.toLowerCase() === 'true'
}

/** SPADS empties a slot by writing `0` (`sendBattleSetting` skips `''`). */
export function isCleared(text: string): boolean {
	return text === '' || text === '0'
}

function isChanged(option: ModOption, current: string): boolean {
	// A map option is set or cleared; there is no default to sit on.
	if (isMapOption(option)) return !isCleared(current)
	const def = defaultText(option)
	if (option.type === 'number') return Number(current) !== Number(def)
	if (option.type === 'bool') return isOn(current) !== isOn(def)
	return current !== def
}

/** What a row is called: BAR's name, or the key when it has none. */
export function label(option: ModOption): string {
	return option.name || option.key
}

export function rowsOf(group: Group, values: Record<string, string>): Row[] {
	return group.options.map((option) => {
		const current = values[option.key] ?? null
		return {
			option,
			current,
			changed: current !== null && isChanged(option, current),
		}
	})
}

export type Changed = { tab: Tab; rows: Row[] }

/** Which rows a view draws. */
export type Shown = (row: Row) => boolean

export const EVERY_ROW: Shown = () => true

/**
 * Where a new tweak of this kind goes: after the last one filled, so it runs
 * after them -- BAR runs each kind in index order -- or, once slot 9 is taken,
 * the first gap. Nothing when all ten are.
 */
export function nextTweak(
	kind: 'defs' | 'units',
	values: Record<string, string>,
): string | null {
	const keys = TWEAK_SLOTS.filter((key) => key.startsWith(`tweak${kind}`))
	const filled = keys.map((key) => !isCleared(values[key] ?? ''))
	const after = filled.lastIndexOf(true) + 1
	if (after < keys.length) return keys[after]!
	const gap = filled.indexOf(false)
	return gap === -1 ? null : keys[gap]!
}

/**
 * What a Changed view draws: what differs from BAR's default, the slot each
 * kind's next tweak would go to, and whatever `held` names -- a slot that is
 * open or holds an unsent edit, which must not vanish when somebody clears it.
 */
export function changedView(
	values: Record<string, string>,
	held: ReadonlySet<string> = new Set(),
): Shown {
	const next = new Set([nextTweak('defs', values), nextTweak('units', values)])
	return (row) =>
		row.changed || next.has(row.option.key) || held.has(row.option.key)
}

/**
 * Every setting by the tab it lives in, as far as `shown` draws it. Tabs with
 * nothing to show are left out.
 */
export function rowsByTab(
	tabs: Tab[],
	values: Record<string, string>,
	shown: Shown,
): Changed[] {
	return tabs
		.map((tab) => ({
			tab,
			rows: tab.groups.flatMap((group) => rowsOf(group, values)).filter(shown),
		}))
		.filter((entry) => entry.rows.length > 0)
}

/** Rows under one heading: a tab's on the All tab, a group's inside a tab. */
export type Section = { name: string; rows: Row[] }

/** What a group BAR left unnamed is called, in the nav and over its rows. */
export const GENERAL_GROUP = 'General'

/**
 * A tab's settings by the group they live in, as far as `shown` draws them.
 * Groups with nothing to show are left out.
 */
export function rowsByGroup(
	tab: Tab,
	values: Record<string, string>,
	shown: Shown,
): Section[] {
	return tab.groups
		.map((group) => ({
			name: group.name || GENERAL_GROUP,
			rows: rowsOf(group, values).filter(shown),
		}))
		.filter((entry) => entry.rows.length > 0)
}

/**
 * Every setting whose name, key, description or value carries every word of
 * the needle, by the tab it lives in. Nothing for an empty needle: that is
 * the tabs' job. Tweak slots have only a key, which is what people type for
 * them; their blob is base64 and stays out of it.
 */
export function searchRows(
	tabs: Tab[],
	values: Record<string, string>,
	needle: string,
): Changed[] {
	if (needle.trim() === '') return []
	return rowsByTab(tabs, values, EVERY_ROW)
		.map((entry) => ({
			tab: entry.tab,
			rows: entry.rows.filter((row) => hasEveryWord(searchText(row), needle)),
		}))
		.filter((entry) => entry.rows.length > 0)
}

/**
 * What a row can be found by: the value as the row shows it -- `on`, an
 * item's name, the number -- and as the room holds it, so a list item's key
 * and `true` count too.
 */
function searchText(row: Row): string {
	const { option } = row
	const value =
		isTweakSlot(row) || isMapOption(row.option)
			? ''
			: `${displayText(row)} ${row.current ?? ''}`
	return `${label(option)} ${option.key} ${option.desc ?? ''} ${value}`
}

/** Every setting that differs from BAR's default, by the tab it lives in. */
export function changedByTab(
	tabs: Tab[],
	values: Record<string, string>,
): Changed[] {
	return rowsByTab(tabs, values, (row) => row.changed)
}

/** Whether a row is a tweak slot, whatever its index. */
export function isTweakSlot(row: Row): boolean {
	return tweakKey(row.option.key) !== null
}

/** How many of a tab's settings differ from BAR's default. */
export function changedCount(tab: Tab, values: Record<string, string>): number {
	return tab.groups.reduce(
		(total, group) =>
			total + rowsOf(group, values).filter((row) => row.changed).length,
		0,
	)
}

/** What a row shows on the right: the value, or the default it is sitting on. */
export function displayText(row: Row): string {
	const text = row.current ?? defaultText(row.option)
	// The Map tab asks Rust for words; this is what it says until then.
	if (isMapOption(row.option))
		return isCleared(text) ? 'none' : `${text.length} B`
	if (row.option.type === 'bool') return isOn(text) ? 'on' : 'off'
	if (row.option.type === 'string' && text === '') return 'empty'
	const item = row.option.items?.find((entry) => entry.key === text)
	return item?.name ?? text
}
