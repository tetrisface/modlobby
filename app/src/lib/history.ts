/**
 * Settings of games already played, side by side with a room or each other.
 *
 * Pure. Rust has already held both sides to one rule (`game-history`'s
 * `normalize`): nothing nobody set, no cleared slot, a tweak by its blob's
 * id. What is left here is presenting them: which values differ by each
 * option's own rule, under which tab, and how a list of settings filters
 * and sorts.
 */

import type { BlobInfo } from '../ipc/bindings/BlobInfo'
import type { ModOption } from '../ipc/bindings/ModOption'
import type { Role } from '../ipc/bindings/Role'
import type { SettingRow } from '../ipc/bindings/SettingRow'
import { hasEveryWord, wordsOf } from './search'
import { TWEAK_SLOTS, label, sameValue, tabs, tweakKey } from './setup'

/** One side of a comparison: a whole setting. */
export type Side = { map: string | null; values: Record<string, string> }

/** A setting, from the usual values and where it departs from them. */
export function sideOf(row: SettingRow, usual: Record<string, string>): Side {
	const values = { ...usual }
	for (const [key, value] of Object.entries(row.differs)) {
		if (value === null) delete values[key]
		else values[key] = value
	}
	return { map: row.map, values }
}

function byKey(catalogue: readonly ModOption[]): Map<string, ModOption> {
	return new Map(catalogue.map((option) => [option.key, option]))
}

/**
 * The keys whose values differ, in key order. A key one side lacks is that
 * side on its default. A tweak slot holds a blob's id, so it is compared as
 * written whatever the table declares the slot to be.
 */
export function differingKeys(
	catalogue: readonly ModOption[],
	left: Side,
	right: Side,
): string[] {
	const options = byKey(catalogue)
	const keys = new Set([
		...Object.keys(left.values),
		...Object.keys(right.values),
	])
	return [...keys]
		.filter(
			(key) =>
				!sameValue(
					tweakKey(key) === null ? options.get(key) : undefined,
					left.values[key] ?? null,
					right.values[key] ?? null,
				),
		)
		.sort()
}

export type DiffRow = {
	key: string
	option: ModOption | undefined
	left: string | null
	right: string | null
}
export type DiffSection = { name: string; rows: DiffRow[] }

/** Where a key the game does not declare goes: last. */
export const NOT_IN_GAME = 'Not in this game'

/** What differs, under the tabs a room shows, each tab's rows in BAR's order. */
export function diffSections(
	catalogue: readonly ModOption[],
	left: Side,
	right: Side,
): DiffSection[] {
	const differing = differingKeys(catalogue, left, right)
	const remaining = new Set(differing)
	const rowOf = (key: string, option: ModOption | undefined): DiffRow => ({
		key,
		option,
		left: left.values[key] ?? null,
		right: right.values[key] ?? null,
	})

	const extra = differing.filter(
		(key) => tweakKey(key) !== null && !TWEAK_SLOTS.includes(key),
	)
	const sections: DiffSection[] = []
	// Each row takes the tab's own option: the Modding tab's slots, the Map
	// tab's names, rather than the table's declarations of them.
	for (const tab of tabs([...catalogue], extra)) {
		const options = tab.groups
			.flatMap((group) => group.options)
			.filter((option) => remaining.has(option.key))
		for (const option of options) remaining.delete(option.key)
		if (options.length > 0)
			sections.push({
				name: tab.name,
				rows: options.map((option) => rowOf(option.key, option)),
			})
	}
	if (remaining.size > 0) {
		// Without the game's table nothing is known, so nothing is "not in it".
		const name = catalogue.length > 0 ? NOT_IN_GAME : ''
		sections.push({
			name,
			rows: [...remaining].map((key) => rowOf(key, undefined)),
		})
	}
	return sections
}

/**
 * Keys the game no longer declares, which a load leaves out. Tweak slots
 * past the declared two are read by any index, so they are never retired;
 * with no table to tell by, nothing is.
 */
export function retiredKeys(
	catalogue: readonly ModOption[],
	side: Side,
): string[] {
	if (catalogue.length === 0) return []
	const options = byKey(catalogue)
	return Object.keys(side.values).filter(
		(key) => !options.has(key) && tweakKey(key) === null,
	)
}

const RANK: readonly Role[] = ['unknown', 'late', 'spectated', 'played']

/** The most I was in any of a setting's games. */
export function bestRole(row: SettingRow): Role {
	return row.games.reduce<Role>(
		(best, game) =>
			RANK.indexOf(game.role) > RANK.indexOf(best) ? game.role : best,
		'unknown',
	)
}

/** The part I had in a setting's games, to filter by. */
export type Part = 'all' | 'played' | 'spectated'

/**
 * Played: any of its games. Spectated: watched and never played, a game I
 * joined after it began counting as watched, since that is what it nearly
 * always was.
 */
function hadPart(row: SettingRow, part: Part): boolean {
	const best = bestRole(row)
	if (part === 'played') return best === 'played'
	if (part === 'spectated') return best === 'spectated' || best === 'late'
	return true
}

export type Filter = {
	needle: string
	thisMap: boolean
	part: Part
}

/** The settings that pass `filter`, `map` being the room's. */
export function filterRows(
	rows: readonly SettingRow[],
	filter: Filter,
	map: string | null,
	text: (row: SettingRow) => string = (row) => row.map,
): SettingRow[] {
	const words = wordsOf(filter.needle)
	return rows.filter(
		(row) =>
			hasEveryWord(text(row), words) &&
			(!filter.thisMap || row.map === map) &&
			hadPart(row, filter.part),
	)
}

/**
 * Everyone who played a setting's games besides me, once each: the newest
 * game's first, which are the ones most likely to be remembered.
 */
export function playersOf(row: SettingRow): string[] {
	const seen = new Map<string, string>()
	for (const name of row.games.flatMap((game) => game.players)) {
		const known = name.toLowerCase()
		if (!seen.has(known)) seen.set(known, name)
	}
	return [...seen.values()]
}

/** How many names a tooltip lists before it counts the rest. */
export const PLAYERS_TIP = 30

/** Names one a line, as tall as a tooltip stays readable. */
export function playersTip(names: readonly string[]): string {
	const shown = names.slice(0, PLAYERS_TIP)
	const rest = names.length - shown.length
	return [...shown, ...(rest > 0 ? [`and ${rest} more`] : [])].join('\n')
}

export type HistoryColumn = 'played' | 'map' | 'games' | 'delta'

function sortValue(
	row: SettingRow,
	column: HistoryColumn,
	delta: (row: SettingRow) => number,
): string | number {
	switch (column) {
		case 'played':
			return row.games[0]?.started ?? 0
		case 'map':
			return row.map.toLowerCase()
		case 'games':
			return row.games.length
		case 'delta':
			return delta(row)
	}
}

export function sortRows(
	rows: readonly SettingRow[],
	column: HistoryColumn,
	descending: boolean,
	delta: (row: SettingRow) => number,
): SettingRow[] {
	return [...rows].sort((a, b) => {
		const [left, right] = [
			sortValue(a, column, delta),
			sortValue(b, column, delta),
		]
		const order =
			typeof left === 'string' && typeof right === 'string'
				? left.localeCompare(right)
				: Number(left) - Number(right)
		// The newest first among equals, so a column of ties does not shuffle.
		if (order === 0)
			return (b.games[0]?.started ?? 0) - (a.games[0]?.started ?? 0)
		return descending ? -order : order
	})
}

/**
 * What a setting does that most do not: its departures from the usual
 * values, by name. A tweak goes by its own name where it has one.
 */
export function departures(
	row: SettingRow,
	catalogue: readonly ModOption[],
	blobs: Record<string, BlobInfo>,
): string[] {
	const options = byKey(catalogue)
	return Object.entries(row.differs).map(([key, value]) => {
		const tweak =
			tweakKey(key) !== null && value !== null ? blobs[value]?.name : null
		if (tweak) return tweak
		const option = options.get(key)
		return option ? label(option) : key
	})
}
