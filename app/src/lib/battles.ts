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
import type { BattleSort } from '../ipc/bindings/BattleSort'
import type { ModeFilter } from '../ipc/bindings/ModeFilter'

export type Row = { battle: BattleView; running: boolean }

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
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return true

  const haystack = [
    battle.title,
    battle.mapName,
    battle.founder,
    battle.gameName,
  ]
    .join(' ')
    .toLowerCase()

  return words.every((word) => haystack.includes(word))
}

export function keep(row: Row, filters: BattleList, query: string): boolean {
  const { battle } = row
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

  return b.battle.id - a.battle.id
}

const BY: Record<
  Exclude<BattleSort, 'relevance'>,
  (row: Row) => string | number
> = {
  players: (row) => row.battle.playerCount,
  title: (row) => row.battle.title.toLowerCase(),
  map: (row) => row.battle.mapName.toLowerCase(),
  host: (row) => row.battle.founder.toLowerCase(),
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
  { key: 'host', label: 'Host' },
]

export const MODES: ReadonlyArray<{ key: ModeFilter; label: string }> = [
  { key: 'all', label: 'All' },
  { key: 'pve', label: 'PvE' },
  { key: 'pvp', label: 'PvP' },
]
