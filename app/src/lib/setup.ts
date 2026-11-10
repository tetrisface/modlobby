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
import type { OptionChangeView } from '../ipc/bindings/OptionChangeView'
import type { OptionValue } from '../ipc/bindings/OptionValue'
import type { ProposalView } from '../ipc/bindings/ProposalView'
import type { VoteView } from '../ipc/bindings/VoteView'
import { hasEveryWord } from './search'

export type Group = { name: string; options: ModOption[] }
export type Tab = {
	key: string
	name: string
	desc: string
	groups: Group[]
	/** What BAR writes at the head of the tab: how to play it, and its warnings. */
	intro: string[]
}

export type Row = {
	option: ModOption
	/** What the room has set, if anything. */
	current: string | null
	changed: boolean
}

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
 *
 * An option BAR declares hidden is a row like any other, in the group it is
 * declared in: Chobby leaves it out, but a host takes it, a room can hold it,
 * and a modding lobby shows what a room holds. The row says it is hidden.
 */
function introOf(options: ModOption[]): string[] {
	return options
		.filter((option) => option.type === 'subheader')
		.map((option) => (option.name ?? '').trim())
		.filter((text) => text !== '' && !text.startsWith('--'))
}

function groupsOf(options: ModOption[]): Group[] {
	const groups: Group[] = []
	let current: Group = { name: '', options: [] }

	for (const option of options) {
		// A link goes on the option it is about, or at the head of its tab: see `linksOf`.
		if (option.type === 'separator' || option.type === 'link') continue
		if (option.type === 'subheader') {
			const label = option.name ?? ''
			if (!label.startsWith('--')) continue
			if (current.options.length > 0) groups.push(current)
			current = { name: label.replace(/^--\s*/, '').trim(), options: [] }
			continue
		}
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
		intro: [],
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
		intro: [],
	}
}

/**
 * Tabs in Chobby's order: weight descending, with an unweighted section
 * treated as zero so it lands between Experimental and Cheats. Modding goes
 * next to Cheats, where the tweak slots used to live, and Map last.
 *
 * Every section with something in it is a tab, the ones Chobby hides or drops
 * (`dev`) too; BAR's `_DEV` reads as `DEV`. The map's section is the Map tab.
 *
 * `extra` is tweak slots past the twenty that the room has used.
 */
export function tabs(
	options: ModOption[],
	extra: readonly string[] = [],
): Tab[] {
	const sections = options
		.filter((option) => option.type === 'section' && option.key !== MAP_SECTION)
		.sort((a, b) => (b.weight ?? 0) - (a.weight ?? 0))

	const declared = sections
		.map((section) => {
			const own = options.filter(
				(option) => option.section === section.key && !MOVED.has(option.key),
			)
			return {
				key: section.key,
				name: (section.name || section.key).replace(/^_+/, ''),
				desc: section.desc ?? '',
				groups: groupsOf(own),
				intro: introOf(own),
			}
		})
		.filter((tab) => tab.groups.length > 0)

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

/**
 * Whether two values of an option are the same value, by the option's own
 * type: `1.0` is `1` for a number, `true` is `1` for a switch. `null` is the
 * default. An option the game does not declare is compared as written.
 */
export function sameValue(
	option: ModOption | undefined,
	a: string | null,
	b: string | null,
): boolean {
	if (option === undefined) return a === b
	// A map option is set or cleared; there is no default to sit on.
	if (isMapOption(option)) {
		const [x, y] = [a ?? '', b ?? '']
		return x === y || (isCleared(x) && isCleared(y))
	}
	const def = defaultText(option)
	const [x, y] = [a ?? def, b ?? def]
	if (option.type === 'number') return Number(x) === Number(y)
	if (option.type === 'bool') return isOn(x) === isOn(y)
	if (isEmptyByDefault(option)) return x === y || (isCleared(x) && isCleared(y))
	return x === y
}

function isChanged(option: ModOption, current: string): boolean {
	return !sameValue(option, current, null)
}

/** A text BAR starts empty, which a host can only clear by writing `0`. */
function isEmptyByDefault(option: ModOption): boolean {
	return option.type === 'string' && defaultText(option) === ''
}

/** What a row is called: BAR's name, or the key when it has none. */
export function label(option: ModOption): string {
	return option.name || option.key
}

/** Key first, the one `!bSet` and presets speak. */
export function hint(option: ModOption): string {
	return option.desc ? `${option.key}: ${option.desc}` : option.key
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

/** How rows under a heading are ordered: what differs from the default first, or by name. */
export type RowOrder = 'changed' | 'name'

/**
 * Rows in `order`, stably, so ties keep BAR's order. Under a heading, tweak
 * slots keep the order BAR runs them in, ahead of the rest; `slotsFirst`
 * false sorts them like any other row, for a list with no headings.
 */
export function sortRows(
	rows: Row[],
	order: RowOrder,
	slotsFirst = true,
): Row[] {
	const by =
		order === 'name'
			? (a: Row, b: Row) =>
					label(a.option).localeCompare(label(b.option), undefined, {
						numeric: true,
					})
			: (a: Row, b: Row) => Number(b.changed) - Number(a.changed)
	if (!slotsFirst) return [...rows].sort(by)
	const slots = rows.filter(isTweakSlot)
	const rest = rows.filter((row) => !isTweakSlot(row))
	return [...slots, ...rest.sort(by)]
}

/**
 * Headings in `order`: changed first puts those holding a change ahead,
 * stably, so ties keep BAR's order; by name leaves the headings in BAR's
 * order and sorts only the rows under them.
 */
export function sortSections<S extends { rows: Row[] }>(
	sections: S[],
	order: RowOrder,
): S[] {
	if (order === 'name') return sections
	const changed = (section: S) => section.rows.some((row) => row.changed)
	return [...sections].sort((a, b) => Number(changed(b)) - Number(changed(a)))
}

/** How rows are put under headings: as BAR sections them, by what kind of control they are, or in one list. */
export type Grouping = 'section' | 'type' | 'none'

/** The heading of the documents -- tweak slots, the map's tables -- when rows go by type. */
export const DOCUMENTS = 'Documents'

/** Type headings in the order they are drawn. */
const TYPES = [DOCUMENTS, 'Switches', 'Choices', 'Numbers', 'Text', 'Other']

/** What kind of control a row is, as its heading when rows go by type. */
export function typeOf(row: Row): string {
	if (isTweakSlot(row) || isMapOption(row.option)) return DOCUMENTS
	const named: Partial<Record<string, string>> = {
		bool: 'Switches',
		list: 'Choices',
		number: 'Numbers',
		string: 'Text',
	}
	// A control type BAR added after this build arrives as `{ other }`.
	const type = row.option.type
	return (typeof type === 'string' ? named[type] : undefined) ?? 'Other'
}

/**
 * Sections as `grouping` has them: as they come, by type, or every row in one
 * section with no name, which is drawn without a heading.
 */
export function regroup(sections: Section[], grouping: Grouping): Section[] {
	if (grouping === 'section') return sections
	const rows = sections.flatMap((section) => section.rows)
	if (grouping === 'none') return rows.length > 0 ? [{ name: '', rows }] : []
	return TYPES.map((name) => ({
		name,
		rows: rows.filter((row) => typeOf(row) === name),
	})).filter((section) => section.rows.length > 0)
}

/**
 * The list as Group and Sort have it: regrouped, the rows under each heading
 * sorted, then the headings. With no headings Sort is about the options
 * alone, tweak slots among them.
 */
export function arrange(
	sections: Section[],
	grouping: Grouping,
	order: RowOrder,
): Section[] {
	const sorted = regroup(sections, grouping).map((section) => ({
		...section,
		rows: sortRows(section.rows, order, grouping !== 'none'),
	}))
	return sortSections(sorted, order)
}

/** The order a list was drawn in: each heading, and the keys of the rows under it. */
export type Drawn = ReadonlyArray<{
	name: string
	keys: readonly string[]
}>

export function drawnOrder(sections: Section[]): Drawn {
	return sections.map((section) => ({
		name: section.name,
		keys: section.rows.map((row) => row.option.key),
	}))
}

/**
 * `sections` in the order `drawn` had them, headings and rows alike, with
 * whatever is new after the rest in its own order and whatever is gone left
 * out. The rows are the fresh ones; only their places are kept. For the
 * row under the pointer: a change made to it sorts it away otherwise, and a
 * setting that moves the moment it is set has nothing to settle on.
 */
export function keepOrder(sections: Section[], drawn: Drawn): Section[] {
	const fresh = new Map(sections.map((section) => [section.name, section]))
	const kept: Section[] = []
	for (const was of drawn) {
		const now = fresh.get(was.name)
		if (!now) continue
		fresh.delete(was.name)
		kept.push({ ...now, rows: keepRows(now.rows, was.keys) })
	}
	return [...kept, ...fresh.values()]
}

function keepRows(rows: Row[], keys: readonly string[]): Row[] {
	const fresh = new Map(rows.map((row) => [row.option.key, row]))
	const kept: Row[] = []
	for (const key of keys) {
		const row = fresh.get(key)
		if (!row) continue
		fresh.delete(key)
		kept.push(row)
	}
	return [...kept, ...fresh.values()]
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

/** A value of this option in words: an item's name, on or off, the number. */
export function wordsFor(option: ModOption, text: string): string {
	if (option.type === 'bool') return isOn(text) ? 'on' : 'off'
	if (isEmptyByDefault(option) && isCleared(text)) return 'empty'
	const item = option.items?.find((entry) => entry.key === text)
	return item?.name ?? text
}

/** What a row shows on the right: the value, or the default it is sitting on. */
export function displayText(row: Row): string {
	const { option } = row
	const text = row.current ?? defaultText(option)
	// The Map tab asks Rust for words; this is what it says until then.
	if (isMapOption(option)) return isCleared(text) ? 'none' : `${text.length} B`
	return wordsFor(option, text)
}

/** BAR's default for this option, in the words a row would show it in. */
export function defaultWords(option: ModOption): string {
	return wordsFor(option, defaultText(option))
}

/** Which of the table's facts about an option there are to tell. */
export function facts(option: ModOption): string[] {
	const told: string[] = []
	if (option.min !== null && option.min !== undefined)
		told.push(`min ${option.min}`)
	if (option.max !== null && option.max !== undefined)
		told.push(`max ${option.max}`)
	// BAR: a step of nought or less means no steps at all.
	if (option.step !== null && option.step !== undefined && option.step > 0)
		told.push(`step ${option.step}`)
	const def = defaultWords(option)
	if (def !== '') told.push(`default ${def}`)
	return told
}

/** The facts on one line, as a tooltip has room for, a list's choices first. */
export function tipFacts(option: ModOption): string {
	const named = option.type === 'list' ? choices(option) : []
	return [...named.map((choice) => choice.label), ...facts(option)].join(' · ')
}

export type Choice = { value: string; label: string; desc: string }

/** What an option can be set to, where that is a short list: a switch's two, a list's items. */
export function choices(option: ModOption): Choice[] {
	if (option.type === 'bool')
		return [
			{ value: '0', label: 'off', desc: '' },
			{ value: '1', label: 'on', desc: '' },
		]
	if (option.type !== 'list') return []
	return (option.items ?? []).map((item) => ({
		value: item.key,
		label: item.name || item.key,
		desc: item.desc ?? '',
	}))
}

/** Which of `choices` a value lights, however a switch's value arrived. */
export function litChoice(option: ModOption, text: string): string {
	if (option.type === 'bool') return isOn(text) ? '1' : '0'
	return text
}

/**
 * What to send to put an option back on its default. A host skips an empty
 * value (`sendBattleSetting`) and clears by writing `0`, the way Chobby does;
 * anywhere else an empty text is simply empty.
 */
export function resetValue(option: ModOption, host: boolean): string {
	const def = defaultText(option)
	if (def !== '') return def
	return option.type === 'string' && !host ? '' : '0'
}

/** The newest thing the lobby has seen happen to this option, if anything. */
export function latestChange(
	history: readonly OptionChangeView[],
	key: string,
): OptionChangeView | undefined {
	return history.filter((change) => change.key === key).at(-1)
}

/** What a vote under way would set this option to, if that is what it is about. */
export function voteOn(
	vote: VoteView | null | undefined,
	key: string,
): Extract<ProposalView, { type: 'setOption' }> | null {
	const proposal = vote?.proposal
	return proposal?.type === 'setOption' && proposal.key === key
		? proposal
		: null
}

/** Why an option is not in force: which option holds it off, at what value. */
export type Lock = { by: string; value: string }

export const notInForce = (lock: Lock): string =>
	`Not in force while ${lock.by} is ${lock.value}`

/** Somewhere BAR points to about an option or a tab: what it is, and which site it is on. */
export type OptionLink = { url: string; desc: string; host: string }

/** The site an address goes to; nothing for what no lobby should open. */
export function linkHost(url: string | undefined): string {
	try {
		const { protocol, hostname } = new URL(url ?? '')
		return protocol === 'https:' || protocol === 'http:' ? hostname : ''
	} catch {
		return ''
	}
}

const isLayout = (option: ModOption) =>
	option.type === 'section' ||
	option.type === 'subheader' ||
	option.type === 'separator'

/**
 * BAR's links, by what they are about. It declares most right after their
 * option, with a key that extends the option's: `evocom`, then
 * `evocom_feedback_thread_link`. Its names for them, `Feedback thread` over
 * and over, only make sense in that spot, so a link has no row of its own: it
 * goes on its option. One that follows anything else is about its whole tab.
 */
export function linksOf(options: readonly ModOption[]): {
	byKey: ReadonlyMap<string, OptionLink[]>
	bySection: ReadonlyMap<string, OptionLink[]>
} {
	const byKey = new Map<string, OptionLink[]>()
	const bySection = new Map<string, OptionLink[]>()
	const file = (
		table: Map<string, OptionLink[]>,
		key: string,
		link: OptionLink,
	) => table.set(key, [...(table.get(key) ?? []), link])

	let before: ModOption | null = null
	for (const option of options) {
		if (isLayout(option)) continue
		if (option.type !== 'link') {
			before = option
			continue
		}
		const link: OptionLink = {
			url: option.link ?? '',
			desc: (option.desc || label(option)).replace(/\.$/, ''),
			host: linkHost(option.link),
		}
		if (before !== null && option.key.startsWith(`${before.key}_`))
			file(byKey, before.key, link)
		else file(bySection, option.section ?? '', link)
	}
	return { byKey, bySection }
}

/** What the pane knows about a row beyond its value. */
export type RowNote = {
	/** It changed a moment ago. */
	recent: boolean
	/** What a vote under way would set it to, in words; empty for a document. */
	vote: string | null
	/** What holds it off, if anything does. */
	lock: Lock | null
	/** The strip under the list is pinned to it. */
	on: boolean
	/** Where BAR points to about it. */
	links: OptionLink[]
}

export const NO_NOTE: RowNote = {
	recent: false,
	vote: null,
	lock: null,
	on: false,
	links: [],
}

/**
 * The options this one holds off at this value: BAR's grammar between options,
 * as Chobby reads it (`gui_modoptions_panel.lua`, `postLock`). A list's chosen
 * item names them; a switch names one list for on and another for off; a
 * number one for on its default and another for off it.
 */
function heldOff(option: ModOption, text: string): readonly string[] {
	if (option.type === 'bool')
		return (isOn(text) ? option.lock : option.unlock) ?? []
	if (option.type === 'list')
		return option.items?.find((item) => item.key === text)?.lock ?? []
	if (option.type === 'number')
		return (
			(Number(text) === Number(defaultText(option))
				? option.unlock
				: option.lock) ?? []
		)
	return []
}

/**
 * Every option something else currently holds off, and what does. A switch that
 * names itself is BAR's way of tucking options out of Chobby's summary of what
 * changed: they stay in force, so it holds none of them off. Held by two, the
 * first is named.
 */
export function locksOf(
	options: readonly ModOption[],
	values: Record<string, string>,
): ReadonlyMap<string, Lock> {
	const locks = new Map<string, Lock>()
	for (const option of options) {
		const text = values[option.key] ?? defaultText(option)
		const held = heldOff(option, text)
		if (held.includes(option.key)) continue
		for (const key of held) {
			if (locks.has(key)) continue
			locks.set(key, { by: label(option), value: wordsFor(option, text) })
		}
	}
	return locks
}
