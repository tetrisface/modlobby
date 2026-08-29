import { describe, expect, test } from 'vitest'
import type { BattleList } from '../ipc/bindings/BattleList'
import type { BattleView } from '../ipc/bindings/BattleView'
import { arrange, isVsAi, matches, type Row } from './battles'

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
    ...over,
  }
}

const row = (over: Partial<BattleView> = {}, running = false): Row => ({
  battle: battle(over),
  running,
})

const filters = (over: Partial<BattleList> = {}): BattleList => ({
  showPassworded: true,
  showLocked: true,
  showEmpty: true,
  showRunning: true,
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

  test('map and host sort on their own field', () => {
    // Xray, Yankee, Zulu.
    expect(titles(arrange(rows, filters({ sort: 'map' }), ''))).toEqual([
      'gamma',
      'alpha',
      'beta',
    ])
    expect(titles(arrange(rows, filters({ sort: 'host' }), ''))).toEqual([
      'alpha',
      'gamma',
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
