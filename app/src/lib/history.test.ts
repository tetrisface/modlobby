import { describe, expect, test } from 'vitest'
import type { GameRow } from '../ipc/bindings/GameRow'
import type { ModOption } from '../ipc/bindings/ModOption'
import type { SettingRow } from '../ipc/bindings/SettingRow'
import {
	NOT_IN_GAME,
	PLAYERS_TIP,
	bestRole,
	departures,
	diffSections,
	differingKeys,
	filterRows,
	playersOf,
	playersTip,
	retiredKeys,
	sideOf,
	sortRows,
	type Filter,
	type Part,
	type Side,
} from './history'
import { fixtureOptions } from './setup.fixture'

const game = (over: Partial<GameRow> = {}): GameRow => ({
	started: 1_000,
	role: 'played',
	gameSeconds: 1_200,
	ais: [],
	players: [],
	gameType: 'Beyond All Reason test-1',
	replay: 'demos/a.sdfz',
	...over,
})

const row = (over: Partial<SettingRow> = {}): SettingRow => ({
	key: 'k1',
	map: 'Forge',
	differs: {},
	games: [game()],
	...over,
})

const side = (values: Record<string, string>, map = 'Forge'): Side => ({
	map,
	values,
})

const roomMap = 'Forge'

describe('a setting, whole again', () => {
	test('is the usual values with its own departures', () => {
		const usual = { a: '1', b: '1', c: '1' }
		expect(
			sideOf(row({ differs: { b: '2', c: null, d: '4' } }), usual),
		).toEqual(side({ a: '1', b: '2', d: '4' }))
	})
})

describe('what differs between two settings', () => {
	const catalogue = fixtureOptions()

	test('is decided by each option own rule, not by spelling', () => {
		expect(
			differingKeys(
				catalogue,
				side({ startmetal: '1000.0', dynamiccheats: 'true' }),
				side({ startmetal: '1000', dynamiccheats: '1' }),
			),
		).toEqual([])
	})

	test('counts a value against the default the other side sits on', () => {
		expect(
			differingKeys(catalogue, side({ startmetal: '1000' }), side({})),
		).toEqual([])
		expect(
			differingKeys(catalogue, side({ startmetal: '2000' }), side({})),
		).toEqual(['startmetal'])
	})

	test('compares tweak slots by their blob', () => {
		expect(
			differingKeys(
				catalogue,
				side({ tweakdefs: 'aa' }),
				side({ tweakdefs: 'bb' }),
			),
		).toEqual(['tweakdefs'])
		expect(
			differingKeys(
				catalogue,
				side({ tweakdefs: 'aa' }),
				side({ tweakdefs: 'aa' }),
			),
		).toEqual([])
	})

	test("compares an option the game doesn't have as written", () => {
		expect(
			differingKeys(catalogue, side({ gone_option: '1' }), side({})),
		).toEqual(['gone_option'])
	})

	test('is grouped by the tabs a room shows, the unknown last', () => {
		const sections = diffSections(
			catalogue,
			side({ startmetal: '2000', tweakdefs: 'aa', gone_option: '1' }),
			side({ tweakdefs: 'bb' }),
		)
		const names = sections.map((section) => section.name)
		expect(names.at(-1)).toBe(NOT_IN_GAME)
		expect(names).toContain('Modding')
		const modding = sections.find((section) => section.name === 'Modding')
		expect(modding?.rows).toEqual([
			{ key: 'tweakdefs', option: expect.anything(), left: 'aa', right: 'bb' },
		])
		const gone = sections.at(-1)
		expect(gone?.rows).toEqual([
			{ key: 'gone_option', option: undefined, left: '1', right: null },
		])
	})

	test('with no table for the game, is one plain list', () => {
		const sections = diffSections([], side({ a: '1' }), side({ a: '2' }))
		expect(sections).toEqual([
			{
				name: '',
				rows: [{ key: 'a', option: undefined, left: '1', right: '2' }],
			},
		])
	})
})

describe('options a load leaves out', () => {
	test('are the ones the game no longer has, tweak slots aside', () => {
		expect(
			retiredKeys(
				fixtureOptions(),
				side({ startmetal: '1', gone_option: '1', tweakdefs7: 'aa' }),
			),
		).toEqual(['gone_option'])
	})

	test('are none when there is no table to tell by', () => {
		expect(retiredKeys([], side({ anything: '1' }))).toEqual([])
	})
})

describe('the best a setting was played', () => {
	test('is played over spectated over joined late over unknown', () => {
		const roles = (...roles: GameRow['role'][]) =>
			bestRole(row({ games: roles.map((role) => game({ role })) }))
		expect(roles('late', 'played', 'spectated')).toBe('played')
		expect(roles('late', 'spectated')).toBe('spectated')
		expect(roles('unknown', 'late')).toBe('late')
		expect(roles()).toBe('unknown')
	})
})

describe('finding a setting', () => {
	const rows = [
		row({ key: 'here', map: 'Forge' }),
		row({
			key: 'elsewhere',
			map: 'Comet Catcher',
			games: [game({ role: 'spectated' })],
		}),
	]
	const keys = (found: SettingRow[]) => found.map((found) => found.key)
	const none: Filter = {
		needle: '',
		thisMap: false,
		part: 'all',
	}

	test('by words in its text', () => {
		expect(
			keys(filterRows(rows, { ...none, needle: 'comet' }, roomMap)),
		).toEqual(['elsewhere'])
		expect(
			keys(
				filterRows(rows, { ...none, needle: 'spawner' }, roomMap, (found) =>
					found.key === 'here' ? 'Forge Sphere spawner' : found.map,
				),
			),
		).toEqual(['here'])
	})

	test("by the room's map", () => {
		expect(keys(filterRows(rows, { ...none, thisMap: true }, roomMap))).toEqual(
			['here'],
		)
	})

	test('by the part I had in its games', () => {
		const parts = [
			...rows,
			row({ key: 'late', games: [game({ role: 'late' })] }),
			row({ key: 'nobody', games: [game({ role: 'unknown' })] }),
			row({
				key: 'both',
				games: [game({ role: 'spectated' }), game({ role: 'played' })],
			}),
		]
		const part = (part: Part) =>
			keys(filterRows(parts, { ...none, part }, roomMap)).sort()
		expect(part('played')).toEqual(['both', 'here'])
		// A replay that does not list me is a game I came into after it began,
		// which is nearly always to watch.
		expect(part('spectated')).toEqual(['elsewhere', 'late'])
		expect(part('all')).toEqual(['both', 'elsewhere', 'here', 'late', 'nobody'])
	})
})

describe('who played a setting', () => {
	test('is everyone in its games once, the newest game first', () => {
		const found = row({
			games: [
				game({ players: ['Zed', 'amy'] }),
				game({ players: ['Amy', 'Bo'] }),
			],
		})
		expect(playersOf(found)).toEqual(['Zed', 'amy', 'Bo'])
	})

	test('reads as one name a line, the rest counted past the cap', () => {
		const names = Array.from({ length: PLAYERS_TIP + 3 }, (_, at) => `p${at}`)
		const tip = playersTip(names).split('\n')
		expect(tip).toHaveLength(PLAYERS_TIP + 1)
		expect(tip[0]).toBe('p0')
		expect(tip.at(-1)).toBe('and 3 more')
		expect(playersTip(['a', 'b'])).toBe('a\nb')
		expect(playersTip([])).toBe('')
	})
})

describe('ordering settings', () => {
	const rows = [
		row({ key: 'old', map: 'b', games: [game({ started: 1 })] }),
		row({
			key: 'new',
			map: 'a',
			games: [game({ started: 3 }), game({ started: 2 })],
		}),
	]
	const keys = (sorted: SettingRow[]) => sorted.map((found) => found.key)
	const delta = (found: SettingRow) => (found.key === 'old' ? 5 : 1)

	test('by when, map, games and difference', () => {
		expect(keys(sortRows(rows, 'played', true, delta))).toEqual(['new', 'old'])
		expect(keys(sortRows(rows, 'map', false, delta))).toEqual(['new', 'old'])
		expect(keys(sortRows(rows, 'games', true, delta))).toEqual(['new', 'old'])
		expect(keys(sortRows(rows, 'delta', false, delta))).toEqual(['new', 'old'])
		expect(keys(sortRows(rows, 'delta', true, delta))).toEqual(['old', 'new'])
	})
})

describe("what a setting's line says is unusual", () => {
	test('names its departures, tweaks by their own name', () => {
		const catalogue: ModOption[] = [
			{
				key: 'startmetal',
				name: 'Starting metal',
				type: 'number',
				def: '1000',
			},
		]
		const blobs = { aa: { bytes: 10, name: 'Sphere spawner' } }
		expect(
			departures(
				row({ differs: { startmetal: '2000', tweakdefs: 'aa', gone: null } }),
				catalogue,
				blobs,
			),
		).toEqual(['Starting metal', 'Sphere spawner', 'gone'])
	})
})
