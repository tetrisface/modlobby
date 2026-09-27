/**
 * Choosing which rooms to show, and in what order.
 *
 * The default order is Chobby's, ported from `BattleListWindow:CompareItems`
 * (`battle_list_window.lua:960`), because a room's position in that list is
 * something people navigate by muscle memory. What we add on top is the
 * ability to sort by a column instead, which Chobby has no way to do.
 */

import type { BattleView } from '../ipc/bindings/BattleView'
import type { BattleList } from '../ipc/bindings/BattleList'
import type { SortKey } from '../ipc/bindings/SortKey'
import type { SortStep } from '../ipc/bindings/SortStep'
import type { ModeFilter } from '../ipc/bindings/ModeFilter'
import { ordered } from './reorder'
import { hasEveryWord } from './search'

export type Row = {
	/** Which server the room is on. */
	server: string
	/** The room across every server; see `battleKey`. */
	key: string
	battle: BattleView
	running: boolean
	/** Whether anyone in the room is a friend. */
	hasFriend: boolean
	/** The room's median rank, 0-7; see `medianChevron`. */
	chev: number | null
	/** Mods on top of the game; see `isModded`. */
	modded: boolean
}

/** A room's name across every server: two servers can each have a battle 12. */
export const battleKey = (server: string, id: number) => `${server}/${id}`

/**
 * How a room's shape is said out loud: `2x8` is `8v8`. One team of several is
 * co-op when it plays AI; people against people in that shape keep the raw
 * `1v16`, which at least claims nothing the room is not.
 */
export function layoutLabel(battle: BattleView): string {
	const { layout } = battle
	if (!layout) return ''
	const { teams, teamSize } = layout
	if (teams >= 2) return Array(teams).fill(teamSize).join('v')
	// Bots are only known for the room we are in; the title is the rest.
	const vsAi = battle.bots.length > 0 || isVsAi(battle)
	return teamSize > 1 && vsAi ? 'coop' : `1v${teamSize}`
}

/**
 * Whether a room looks like it is against AI.
 *
 * A room's bots only arrive once you have joined it (`spring_out.ex:651`, sent
 * with the join reply), so the title is genuinely all the list has to go on.
 * Chobby matches three phrases; this matches the same three case-insensitively
 * and adds the ones BAR's autohosts actually use.
 */
const VS_AI =
	/\bvs\.?\s*(ai|scavengers?|raptors?|chickens?|bots?)\b|\bpve\b|\bcoop\b/i

export function isVsAi(battle: BattleView): boolean {
	return VS_AI.test(battle.title)
}

/**
 * Chobby's search: one word is a substring of any field, several words must
 * each appear somewhere, in any field and any order
 * (`battle_list_window.lua:803-845`).
 */
export function matches(battle: BattleView, query: string): boolean {
	return hasEveryWord(
		[battle.title, battle.mapName, battle.founder, battle.gameName].join(' '),
		query,
	)
}

export function keep(row: Row, filters: BattleList, query: string): boolean {
	const { battle } = row
	if (filters.friendsOnly && !row.hasFriend) return false
	if (!filters.showPassworded && battle.passworded) return false
	if (!filters.showLocked && battle.locked) return false
	if (!filters.showRunning && row.running) return false
	// A running room with nobody in it is still worth watching; an idle one is
	// what people mean by empty. Chobby's comparator draws the same line, so the
	// filter follows it.
	if (!filters.showEmpty && battle.playerCount === 0 && !row.running)
		return false
	if (!matchesMode(battle, filters.mode)) return false
	return matches(battle, query)
}

function matchesMode(battle: BattleView, mode: ModeFilter): boolean {
	if (mode === 'all') return true
	return mode === 'pve' ? isVsAi(battle) : !isVsAi(battle)
}

/**
 * Whether a room plays with mods on top of its game.
 *
 * The list never sees a room's script tags, which is where a host announces
 * its mods, so this reads what it can see: the room is on the mods server,
 * whose hosts all run them; its game is not BAR at all; or its title says
 * so, which is how our autohosts name their rooms and how any host can opt
 * in -- the same reading the PvE filter takes off a title.
 */
const SAYS_MODS = /\bmutators?\b|\bmods\b|\bmodded\b/i

export function isModded(
	battle: BattleView,
	server: string,
	modsServer: string | null,
): boolean {
	return (
		server === modsServer ||
		!/^beyond all reason\b/i.test(battle.gameName) ||
		SAYS_MODS.test(battle.title)
	)
}

/**
 * The steps with `key` in front and on: what "browse mod battles" asks of
 * the list. The rest keep their order and switches.
 */
export function leadWith(steps: readonly SortStep[], key: SortKey): SortStep[] {
	return [{ by: key, on: true }, ...steps.filter((step) => step.by !== key)]
}

/** What a row is read for, a number or a lowercase name, larger meaning the
 * notable thing: open, unlocked, with people, not started, full, watched,
 * modded, high ranked. */
type Read = (row: Row) => number | string

const open: Read = (row) => (row.battle.passworded ? 0 : 1)
const unlocked: Read = (row) => (row.battle.locked ? 0 : 1)
const active: Read = (row) =>
	!row.running && row.battle.playerCount === 0 ? 0 : 1
const waiting: Read = (row) => (row.running ? 0 : 1)
const players: Read = (row) => row.battle.playerCount
const watchers: Read = (row) => row.battle.spectatorCount
const modded: Read = (row) => (row.modded ? 1 : 0)

/**
 * What each step compares, in order: Chobby's bands; the modded rooms ahead
 * of everything, then those bands, since an empty modded autohost under
 * three hundred busy rooms is as good as hidden; then the columns.
 */
const CHAIN: Record<SortKey, readonly Read[]> = {
	relevance: [open, unlocked, active, waiting, players, watchers],
	modded: [modded, open, unlocked, active, waiting, players, watchers],
	players: [players],
	rank: [(row) => row.chev ?? -1],
	title: [(row) => row.battle.title.toLowerCase()],
	map: [(row) => row.battle.mapName.toLowerCase()],
}

/** Names read best from A; everything else with the notable rooms first. */
const largerFirst = (key: SortKey) => key !== 'title' && key !== 'map'

/**
 * The steps that are on, in order: the first that tells two rooms apart
 * decides, and the id breaks whatever tie is left so the order never
 * flickers between updates.
 */
export function compare(a: Row, b: Row, steps: readonly SortStep[]): number {
	for (const step of steps) {
		if (!step.on) continue
		for (const read of CHAIN[step.by]) {
			const left = read(a)
			const right = read(b)
			if (left === right) continue
			const order = left < right ? -1 : 1
			return largerFirst(step.by) ? -order : order
		}
	}
	return a.battle.id - b.battle.id
}

export function arrange(
	rows: Row[],
	filters: BattleList,
	query: string,
): Row[] {
	return rows
		.filter((row) => keep(row, filters, query))
		.sort((a, b) => compare(a, b, filters.sort))
}

/** Every step, with what its chip says and what a hover explains. */
export const SORT_KEYS: ReadonlyArray<{
	key: SortKey
	label: string
	tip: string
}> = [
	{
		key: 'relevance',
		label: 'Relevance',
		tip: "Chobby's order: open rooms, then the fuller and the more watched",
	},
	{
		key: 'modded',
		label: 'Mods',
		tip: 'Relevance, with rooms that play with mods ahead of the rest',
	},
	{ key: 'players', label: 'Players', tip: 'How many are playing' },
	{ key: 'rank', label: 'Rank', tip: "The room's median rank" },
	{ key: 'title', label: 'Title', tip: 'By name' },
	{ key: 'map', label: 'Map', tip: 'By map' },
]

/**
 * Every step in the order it comes untouched, all on: Chobby's order, then
 * it again with the modded rooms ahead, then the columns, each only
 * breaking ties.
 */
export function chobbySort(): SortStep[] {
	return SORT_KEYS.map(({ key }) => ({ by: key, on: true }))
}

/** The steps with `at` switched the other way. */
export function toggled(steps: readonly SortStep[], at: number): SortStep[] {
	return steps.map((step, index) =>
		index === at ? { ...step, on: !step.on } : step,
	)
}

export const MODES: ReadonlyArray<{ key: ModeFilter; label: string }> = [
	{ key: 'all', label: 'All' },
	{ key: 'pve', label: 'PvE' },
	{ key: 'pvp', label: 'PvP' },
]

/**
 * The rows in a held order, for while the pointer is over the list.
 *
 * On a busy evening the list re-sorts every few seconds, which means the room
 * you are reaching for jumps away as you reach. So while the pointer is inside
 * the list the *order* holds still and only the rows' contents update; the
 * fresh order applies the moment the pointer leaves. `held` is the order being
 * preserved, as battle keys:
 *
 * - a held key whose room has closed simply drops out — a row cannot outlive
 *   its room, and the collapse is the one movement that cannot be helped;
 * - a room the held order does not know is appended at the bottom, in its own
 *   sorted order, rather than teleporting into the middle.
 */
export function stabilize(sorted: Row[], held: readonly string[]): Row[] {
	const byKey = new Map(sorted.map((row) => [row.key, row]))
	// The same "saved order, applied to what is actually there" rule the chat
	// tabs follow, over battle keys instead of room names.
	return ordered([...byKey.keys()], held).flatMap((key) => byKey.get(key) ?? [])
}

/**
 * The middle chevron of the people in a room.
 *
 * Chevrons are the only measure of a player the list is ever given. Nothing
 * on the wire carries a rating: `ADDUSER` is name and country, `CLIENTSTATUS`
 * is a 0-7 rank, `s.user.whois` answers with a colour and an icon, and real
 * OpenSkill reaches a client only as script tags for the one room it is in.
 * So this is what a room's strength has to be read off, and it is a poor
 * proxy: teiserver computes the rank from hours played plus contributor role
 * (`cache_user.ex:1137`), which is how long people have been here, not how
 * well they play.
 *
 * Median rather than mean, because one 7 among five 1s averages to a room
 * that nobody in it resembles. An even count lands on a half — the hours
 * behind the chevrons never leave the server, so there is nothing here to
 * break the tie with, and `3.5` says that where rounding would pick a side.
 */
export function medianChevron(ranks: readonly number[]): number | null {
	if (ranks.length === 0) return null
	const sorted = [...ranks].sort((a, b) => a - b)
	const mid = sorted.length >> 1
	if (sorted.length % 2 === 1) return sorted[mid]!
	return (sorted[mid - 1]! + sorted[mid]!) / 2
}
