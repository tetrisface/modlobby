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
import type { LayoutView } from '../ipc/bindings/LayoutView'
import type { BattleSort } from '../ipc/bindings/BattleSort'
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
}

/** A room's name across every server: two servers can each have a battle 12. */
export const battleKey = (server: string, id: number) => `${server}/${id}`

/**
 * How a room's shape is said out loud: `2x8` is `8v8`, and one team of more
 * than one is co-op, whoever it is they are all playing against.
 */
export function layoutLabel(layout: LayoutView | null): string {
	if (!layout) return ''
	const { teams, teamSize } = layout
	if (teams < 2) return teamSize > 1 ? 'coop' : '1v1'
	return Array(teams).fill(teamSize).join('v')
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
 * Chobby's bands, outermost first: open rooms, then running, then locked, then
 * passworded. Player count decides inside a band, and the id breaks the tie so
 * the order never flickers between updates.
 */
function relevance(a: Row, b: Row): number {
	if (a.battle.passworded !== b.battle.passworded)
		return a.battle.passworded ? 1 : -1
	if (a.battle.passworded)
		return a.battle.title.toLowerCase() < b.battle.title.toLowerCase() ? -1 : 1

	if (a.battle.locked !== b.battle.locked) return a.battle.locked ? 1 : -1

	const idle = (row: Row) => !row.running && row.battle.playerCount === 0
	if (idle(a) !== idle(b)) return idle(a) ? 1 : -1

	if (a.running !== b.running) return a.running ? 1 : -1

	if (a.battle.playerCount !== b.battle.playerCount)
		return b.battle.playerCount - a.battle.playerCount

	return watchers(a, b) || b.battle.id - a.battle.id
}

/**
 * Relevance's second key: more spectators first.
 *
 * Among rooms with the same number of players, the one people are watching is
 * the more interesting one. The column sorts keep their plain id tie-break —
 * this is the default order's opinion, not a rule about every column.
 */
function watchers(a: Row, b: Row): number {
	return b.battle.spectatorCount - a.battle.spectatorCount
}

const BY: Record<
	Exclude<BattleSort, 'relevance'>,
	(row: Row) => string | number
> = {
	players: (row) => row.battle.playerCount,
	title: (row) => row.battle.title.toLowerCase(),
	map: (row) => row.battle.mapName.toLowerCase(),
	rank: (row) => row.chev ?? -1,
}

export function compare(a: Row, b: Row, sort: BattleSort, descending: boolean) {
	if (sort === 'relevance') return relevance(a, b)

	const key = BY[sort]
	const left = key(a)
	const right = key(b)
	if (left === right) return a.battle.id - b.battle.id

	const order = left < right ? -1 : 1
	return descending ? -order : order
}

export function arrange(
	rows: Row[],
	filters: BattleList,
	query: string,
): Row[] {
	return rows
		.filter((row) => keep(row, filters, query))
		.sort((a, b) => compare(a, b, filters.sort, filters.sortDescending))
}

export const SORTS: ReadonlyArray<{ key: BattleSort; label: string }> = [
	{ key: 'relevance', label: 'Relevance' },
	{ key: 'players', label: 'Players' },
	{ key: 'title', label: 'Title' },
	{ key: 'map', label: 'Map' },
	{ key: 'rank', label: 'Rank' },
]

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
