import { describe, expect, test } from 'vitest'
import type { BattleStatusView } from '../ipc/bindings/BattleStatusView'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { UserView } from '../ipc/bindings/UserView'
import { arrange, emptySeats } from './roster'

const seat = (allyTeam: number, player = true): BattleStatusView => ({
  ready: false,
  team: allyTeam,
  allyTeam,
  player,
  handicap: 0,
  sync: 'synced',
  side: 0,
})

const user = (
  name: string,
  battleStatus: BattleStatusView | null,
  bot = false,
): UserView => ({
  name,
  country: 'SE',
  userId: 1,
  lobbyClient: 'modlobby',
  status: { inGame: false, away: false, rank: 0, moderator: false, bot },
  battleStatus,
  battleId: 5,
})

const room = (over: Partial<BattleView> = {}): BattleView => ({
  id: 5,
  founder: 'Host',
  ip: '1.2.3.4',
  port: 1,
  maxPlayers: 16,
  passworded: false,
  locked: false,
  mapHash: 'h',
  mapName: 'Map',
  engineName: 'recoil',
  engineVersion: '2026.07.03',
  title: 'Room',
  gameName: 'BAR',
  members: ['Host', 'alice', 'bob', 'carol', 'dave', 'me'],
  spectatorCount: 3,
  playerCount: 3,
  layout: null,
  bots: [],
  startRects: [],
  ...over,
})

const byName = (...users: UserView[]) =>
  Object.fromEntries(users.map((u) => [u.name, u]))

const names = (users: UserView[]) => users.map((u) => u.name)

const arriving = byName(
  user('Host', null, true),
  user('alice', null),
  user('bob', null),
  user('carol', null),
  user('dave', null),
  user('me', null),
)

describe('arrange', () => {
  test('before any status, the list shapes the teams and deals the names', () => {
    const roster = arrange(
      room({ layout: { teams: 2, teamSize: 2 } }),
      arriving,
      'me',
    )
    expect(roster.teams.map((t) => t.allyTeam)).toEqual([0, 1])
    // Three play, per the list: dealt round the teams, never ourselves.
    expect(roster.teams.map((t) => names(t.guessed))).toEqual([
      ['alice', 'carol'],
      ['bob'],
    ])
    expect(roster.teams.map(emptySeats)).toEqual([0, 1])
    expect(names(roster.spectators)).toEqual(['Host'])
    expect(names(roster.pending)).toEqual(['dave', 'me'])
    expect(roster.spectatorCount).toBe(3)
  })

  test('without a layout the players are split over two teams', () => {
    const roster = arrange(room(), arriving, 'me')
    expect(roster.teams.map((t) => t.expected)).toEqual([2, 2])
    expect(roster.teams.map((t) => t.guessed.length)).toEqual([2, 1])
  })

  test('a status seats for real and the guesses give way', () => {
    const roster = arrange(
      room({ layout: { teams: 2, teamSize: 2 } }),
      byName(
        user('Host', null, true),
        user('alice', seat(1)),
        user('bob', null),
        user('carol', seat(0, false)),
        user('dave', null),
        user('me', seat(0, false)),
      ),
      'me',
    )
    expect(names(roster.teams[1]!.users)).toEqual(['alice'])
    // Two seats left to fill for three players, one of them placed already.
    expect(roster.teams.map((t) => names(t.guessed))).toEqual([
      ['bob'],
      ['dave'],
    ])
    expect(roster.teams.map(emptySeats)).toEqual([1, 0])
    expect(names(roster.spectators)).toEqual(['Host', 'carol', 'me'])
    expect(roster.pending).toEqual([])
  })

  test('a settled room draws exactly what it has', () => {
    const roster = arrange(
      room({ layout: { teams: 8, teamSize: 8 }, playerCount: 2 }),
      byName(
        user('Host', seat(0, false), true),
        user('alice', seat(0)),
        user('bob', seat(3)),
        user('carol', seat(0, false)),
        user('dave', seat(0, false)),
        user('me', seat(0, false)),
      ),
      'me',
    )
    expect(roster.pending).toEqual([])
    expect(roster.teams.map((t) => t.allyTeam)).toEqual([0, 3])
    expect(roster.teams.map(emptySeats)).toEqual([0, 0])
    expect(roster.teams.map((t) => t.guessed)).toEqual([[], []])
    expect(roster.spectatorCount).toBe(4)
  })

  test('a team fuller than the layout says grows rather than hides anyone', () => {
    const roster = arrange(
      room({ layout: { teams: 2, teamSize: 1 }, playerCount: 2 }),
      byName(user('alice', seat(0)), user('bob', seat(0)), user('carol', null)),
    )
    expect(roster.teams[0]?.expected).toBe(2)
    expect(emptySeats(roster.teams[0]!)).toBe(0)
    // Both seats the list counted are taken; carol is not dealt anywhere.
    expect(names(roster.pending)).toEqual(['carol'])
  })
})
