import { describe, expect, test } from 'vitest'
import type { BattleList } from '../ipc/bindings/BattleList'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { BotView } from '../ipc/bindings/BotView'
import {
	arrange,
	battleKey,
	isVsAi,
	layoutLabel,
	matches,
	medianChevron,
	stabilize,
	type Row,
} from './battles'

let nextId = 1

function battle(over: Partial<BattleView> = {}): BattleView {
	return {
		id: nextId++,
		founder: 'Host[EU1][001]',
		ip: '',
		port: 0,
		maxPlayers: 16,
		passworded: false,
		locked: false,
		mapHash: '',
		mapName: 'Supreme Isthmus v2.1',
		engineName: 'spring',
		engineVersion: '2026.07.04',
		title: 'a room',
		gameName: 'Beyond All Reason test-31115',
		members: [],
		spectatorCount: 0,
		playerCount: 8,
		layout: null,
		bots: [],
		startRects: [],
		queue: [],
		...over,
	}
}

const row = (
	over: Partial<BattleView> = {},
	running = false,
	hasFriend = false,
	server = 's',
): Row => {
	const room = battle(over)
	return {
		server,
		key: battleKey(server, room.id),
		battle: room,
		running,
		hasFriend,
		chev: null,
	}
}

/** Held orders are by key; these tests' rooms are all on one server. */
const held = (...ids: number[]) => ids.map((id) => battleKey('s', id))

const filters = (over: Partial<BattleList> = {}): BattleList => ({
	showPassworded: true,
	showLocked: true,
	showEmpty: true,
	showRunning: true,
	friendsOnly: false,
	mode: 'all',
	sort: 'relevance',
	sortDescending: false,
	...over,
})

const titles = (rows: Row[]) => rows.map((r) => r.battle.title)

describe('search', () => {
	const room = battle({
		title: 'SuPrEmE MuFF | 8v8',
		mapName: 'Supreme Isthmus v2.1',
		founder: 'Host[US4][000]',
	})

	test('one word is a substring of any field', () => {
		expect(matches(room, 'muff')).toBe(true)
		expect(matches(room, 'isthmus')).toBe(true)
		expect(matches(room, 'us4')).toBe(true)
		expect(matches(room, 'nonsense')).toBe(false)
	})

	test('several words must all appear, in any field and any order', () => {
		// Chobby's multi-word AND: the words span title and map here.
		expect(matches(room, 'muff isthmus')).toBe(true)
		expect(matches(room, 'isthmus muff')).toBe(true)
		expect(matches(room, 'muff nonsense')).toBe(false)
	})

	test('an empty query keeps everything', () => {
		expect(matches(room, '   ')).toBe(true)
	})
})

describe('mode', () => {
	test('reads player-versus-what off the title, which is all the list has', () => {
		expect(isVsAi(battle({ title: 'Coop vs Scavengers | 4v4' }))).toBe(true)
		expect(isVsAi(battle({ title: 'BAR vs AI teams' }))).toBe(true)
		expect(isVsAi(battle({ title: 'Raptor defense PvE' }))).toBe(true)
		expect(isVsAi(battle({ title: 'SuPrEmE MuFF | 8v8' }))).toBe(false)
	})

	test('filters to one side or the other', () => {
		const rows = [
			row({ title: 'Coop vs Raptors' }),
			row({ title: 'SuPrEmE MuFF | 8v8' }),
		]
		expect(titles(arrange(rows, filters({ mode: 'pve' }), ''))).toEqual([
			'Coop vs Raptors',
		])
		expect(titles(arrange(rows, filters({ mode: 'pvp' }), ''))).toEqual([
			'SuPrEmE MuFF | 8v8',
		])
	})
})

describe('friends only', () => {
	test('narrows the list to rooms with someone you know in them', () => {
		const rows = [
			row({ title: 'with a friend' }, false, true),
			row({ title: 'strangers' }),
		]
		expect(titles(arrange(rows, filters({ friendsOnly: true }), ''))).toEqual([
			'with a friend',
		])
		expect(titles(arrange(rows, filters(), '')).length).toBe(2)
	})
})

describe('filters', () => {
	const rows = [
		row({ title: 'open' }),
		row({ title: 'locked', locked: true }),
		row({ title: 'passworded', passworded: true }),
		row({ title: 'idle', playerCount: 0 }),
		row({ title: 'running' }, true),
		row({ title: 'running but empty', playerCount: 0 }, true),
	]

	test('turning one off removes exactly that kind of room', () => {
		expect(
			titles(arrange(rows, filters({ showLocked: false }), '')),
		).not.toContain('locked')
		expect(
			titles(arrange(rows, filters({ showPassworded: false }), '')),
		).not.toContain('passworded')
		expect(
			titles(arrange(rows, filters({ showRunning: false }), '')),
		).not.toContain('running')
	})

	test('turning empty off spares the ones with a game in progress', () => {
		const kept = titles(arrange(rows, filters({ showEmpty: false }), ''))
		expect(kept).not.toContain('idle')
		expect(kept).toContain('running but empty')
	})
})

describe("relevance, which is Chobby's order", () => {
	test('open before running before locked before passworded', () => {
		const rows = [
			row({ title: 'passworded', passworded: true }),
			row({ title: 'locked', locked: true }),
			row({ title: 'running' }, true),
			row({ title: 'open' }),
		]
		expect(titles(arrange(rows, filters(), ''))).toEqual([
			'open',
			'running',
			'locked',
			'passworded',
		])
	})

	test('player count decides inside a band', () => {
		const rows = [
			row({ title: 'few', playerCount: 2 }),
			row({ title: 'many', playerCount: 14 }),
			row({ title: 'some', playerCount: 8 }),
		]
		expect(titles(arrange(rows, filters(), ''))).toEqual([
			'many',
			'some',
			'few',
		])
	})

	test('spectators break a player-count tie, busiest audience first', () => {
		const rows = [
			row({ title: 'unwatched', playerCount: 8, spectatorCount: 0, id: 3 }),
			row({ title: 'popular', playerCount: 8, spectatorCount: 5, id: 1 }),
			row({ title: 'watched', playerCount: 8, spectatorCount: 2, id: 2 }),
		]
		expect(titles(arrange(rows, filters(), ''))).toEqual([
			'popular',
			'watched',
			'unwatched',
		])
	})

	test('an idle empty room sinks below a busy one, but not below a running one', () => {
		const rows = [
			row({ title: 'idle', playerCount: 0 }),
			row({ title: 'busy', playerCount: 4 }),
			row({ title: 'running empty', playerCount: 0 }, true),
		]
		expect(titles(arrange(rows, filters(), ''))).toEqual([
			'busy',
			'running empty',
			'idle',
		])
	})

	test('passworded rooms are alphabetical among themselves', () => {
		const rows = [
			row({ title: 'zulu', passworded: true }),
			row({ title: 'alpha', passworded: true }),
		]
		expect(titles(arrange(rows, filters(), ''))).toEqual(['alpha', 'zulu'])
	})

	test('the order does not flicker when two rooms tie', () => {
		const a = row({ title: 'a', playerCount: 8 })
		const b = row({ title: 'b', playerCount: 8 })
		expect(titles(arrange([a, b], filters(), ''))).toEqual(
			titles(arrange([b, a], filters(), '')),
		)
	})
})

describe('sorting by a column', () => {
	const rows = [
		row({ title: 'beta', mapName: 'Zulu', founder: 'carol', playerCount: 4 }),
		row({
			title: 'alpha',
			mapName: 'Yankee',
			founder: 'alice',
			playerCount: 12,
		}),
		row({ title: 'gamma', mapName: 'Xray', founder: 'bob', playerCount: 8 }),
	]

	test('ascending and descending are mirror images', () => {
		expect(titles(arrange(rows, filters({ sort: 'title' }), ''))).toEqual([
			'alpha',
			'beta',
			'gamma',
		])
		expect(
			titles(
				arrange(rows, filters({ sort: 'title', sortDescending: true }), ''),
			),
		).toEqual(['gamma', 'beta', 'alpha'])
	})

	test('rank sorts on the median, unknown rooms last going down', () => {
		const ranked = [
			{ ...row({ id: 1, title: 'unknown' }), chev: null },
			{ ...row({ id: 2, title: 'low' }), chev: 0 },
			{ ...row({ id: 3, title: 'high' }), chev: 4.5 },
		]
		expect(
			titles(
				arrange(ranked, filters({ sort: 'rank', sortDescending: true }), ''),
			),
		).toEqual(['high', 'low', 'unknown'])
	})

	test('map sorts on its own field', () => {
		// Xray, Yankee, Zulu.
		expect(titles(arrange(rows, filters({ sort: 'map' }), ''))).toEqual([
			'gamma',
			'alpha',
			'beta',
		])
	})

	test('a column sort ignores the bands relevance cares about', () => {
		const banded = [
			row({ title: 'locked big', locked: true, playerCount: 16 }),
			row({ title: 'open small', playerCount: 1 }),
		]
		expect(
			titles(
				arrange(banded, filters({ sort: 'players', sortDescending: true }), ''),
			),
		).toEqual(['locked big', 'open small'])
	})
})

describe('holding the order still under the pointer', () => {
	const ids = (rows: Row[]) => rows.map((r) => r.battle.id)

	test('a fresh sort does not move rows the reader is looking at', () => {
		const resorted = [
			row({ id: 3, playerCount: 9 }),
			row({ id: 1, playerCount: 8 }),
			row({ id: 2, playerCount: 7 }),
		]
		// The reader froze the list when it read 1, 2, 3.
		expect(ids(stabilize(resorted, held(1, 2, 3)))).toEqual([1, 2, 3])
	})

	test('a room that closed drops out; the rest stay put', () => {
		const resorted = [row({ id: 3 }), row({ id: 1 })]
		expect(ids(stabilize(resorted, held(1, 2, 3)))).toEqual([1, 3])
	})

	test('a room that opened appends at the bottom, in sorted order', () => {
		const resorted = [
			row({ id: 9, playerCount: 16 }),
			row({ id: 1, playerCount: 8 }),
			row({ id: 4, playerCount: 2 }),
		]
		// 9 and 4 are new: they arrive below, keeping their order relative to
		// each other, rather than teleporting into the middle of the reading.
		expect(ids(stabilize(resorted, held(1)))).toEqual([1, 9, 4])
	})

	test('the same id on two servers is two rooms', () => {
		const both = [
			row({ id: 1 }, false, false, 'a'),
			row({ id: 1 }, false, false, 'b'),
		]
		expect(stabilize(both, ['b/1', 'a/1']).map((r) => r.server)).toEqual([
			'b',
			'a',
		])
	})

	test('holding nothing is the sorted list itself', () => {
		const resorted = [row({ id: 2 }), row({ id: 1 })]
		expect(ids(stabilize(resorted, held()))).toEqual([2, 1])
	})
})

describe('layoutLabel', () => {
	const shaped = (teams: number, teamSize: number, over = {}) =>
		layoutLabel(battle({ layout: { teams, teamSize }, ...over }))

	test('a room with two or more teams is said as a matchup', () => {
		expect(shaped(2, 8)).toBe('8v8')
		expect(shaped(4, 4)).toBe('4v4v4v4')
	})

	test('one team of several against AI is co-op, by title or by its bots', () => {
		expect(shaped(1, 6, { title: 'Raptors vs AI' })).toBe('coop')
		const raptor = { name: 'Raptors', ai: 'RaptorsDefense' } as BotView
		expect(shaped(1, 6, { bots: [raptor] })).toBe('coop')
	})

	test('one team of several without AI keeps its raw shape', () => {
		expect(shaped(1, 100)).toBe('1v100')
		expect(shaped(1, 1)).toBe('1v1')
		expect(shaped(1, 1, { title: 'vs AI' })).toBe('1v1')
	})

	test('a room that never said its shape says nothing', () => {
		expect(layoutLabel(battle())).toBe('')
	})
})

describe('a room’s median chevron', () => {
	test('is the middle one, not the average', () => {
		// The mean here is 2.5, which describes nobody in the room.
		expect(medianChevron([1, 1, 1, 1, 1, 7])).toBe(1)
	})

	test('splits the difference when the count is even', () => {
		expect(medianChevron([1, 2, 4, 5])).toBe(3)
		expect(medianChevron([2, 3])).toBe(2.5)
	})

	test('is nothing at all when nobody here is known', () => {
		expect(medianChevron([])).toBeNull()
	})
})
