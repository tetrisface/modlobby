import { describe, expect, test } from 'vitest'
import type { BattleStatusView } from '../ipc/bindings/BattleStatusView'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { UserView } from '../ipc/bindings/UserView'
import type { BotView } from '../ipc/bindings/BotView'
import { arrange, emptySeats, freeTeam } from './roster'
import type { Skill } from './skill'

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
  queue: [],
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
    // Team 2 is empty and drawn anyway: the second team always is, so a
    // room of one side has somewhere to drag a row to.
    expect(roster.teams.map((t) => t.allyTeam)).toEqual([0, 1, 3])
    expect(roster.teams.map(emptySeats)).toEqual([0, 0, 0])
    expect(roster.teams.map((t) => t.guessed)).toEqual([[], [], []])
    expect(roster.spectatorCount).toBe(4)
  })

  test('spectators read host first, then by name with case folded', () => {
    const roster = arrange(
      room({
        members: ['Host', 'Zed', 'alice', 'bob', 'me'],
        playerCount: 0,
      }),
      byName(
        user('Host', seat(0, false), true),
        user('Zed', seat(0, false)),
        user('alice', seat(0, false)),
        user('bob', null),
        user('me', seat(0, false)),
      ),
      'me',
    )
    expect(names(roster.spectators)).toEqual(['Host', 'alice', 'me', 'Zed'])
    expect(names(roster.pending)).toEqual(['bob'])
  })

  test('with skills, the strongest watch first; the uncertain and the unrated last', () => {
    const skills: Record<string, Skill> = {
      alice: { value: 18, origin: 'exact', sigma: 1 },
      zed: { value: 31.4, origin: 'exact', sigma: 1 },
      // Too uncertain to show, so it reads `??` and sorts after every number.
      carol: { value: 40, origin: 'exact', sigma: 7 },
    }
    const roster = arrange(
      room({
        members: ['Host', 'Zed', 'alice', 'bob', 'carol', 'me'],
        playerCount: 0,
      }),
      byName(
        user('Host', seat(0, false), true),
        user('Zed', seat(0, false)),
        user('alice', seat(0, false)),
        user('bob', seat(0, false)),
        user('carol', seat(0, false)),
        user('me', seat(0, false)),
      ),
      'me',
      (name) => skills[name.toLowerCase()] ?? null,
    )
    expect(names(roster.spectators)).toEqual([
      'Host',
      'Zed',
      'alice',
      'carol',
      'bob',
      'me',
    ])
  })

  test('the queue is its own list, in the order the server gave', () => {
    const roster = arrange(
      room({
        members: ['Host', 'alice', 'bob', 'carol', 'me'],
        playerCount: 0,
        queue: ['carol', 'alice'],
      }),
      byName(
        user('Host', seat(0, false), true),
        user('alice', seat(0, false)),
        user('bob', seat(0, false)),
        // Queued before any status arrived: the server's word wins.
        user('carol', null),
        user('me', seat(0, false)),
      ),
      'me',
    )
    expect(names(roster.queue)).toEqual(['carol', 'alice'])
    expect(names(roster.spectators)).toEqual(['Host', 'bob', 'me'])
    expect(roster.pending).toEqual([])
    // Waiting is still watching.
    expect(roster.spectatorCount).toBe(5)
  })

  test('a queued name that took a seat is a player, whatever the queue says', () => {
    const roster = arrange(
      room({ members: ['alice', 'bob'], playerCount: 1, queue: ['alice'] }),
      byName(user('alice', seat(0)), user('bob', seat(0, false))),
    )
    expect(names(roster.teams[0]!.users)).toEqual(['alice'])
    expect(roster.queue).toEqual([])
  })

  test('two hundred arrivals are dealt to the shape and no further', () => {
    const members = Array.from({ length: 200 }, (_, i) => `p${i}`)
    const roster = arrange(
      room({
        members,
        playerCount: 100,
        spectatorCount: 100,
        layout: { teams: 4, teamSize: 25 },
      }),
      byName(...members.map((name) => user(name, null))),
      'p0',
    )
    expect(roster.teams).toHaveLength(4)
    expect(roster.teams.map((t) => t.guessed.length)).toEqual([25, 25, 25, 25])
    expect(roster.teams.map(emptySeats)).toEqual([0, 0, 0, 0])
    expect(roster.pending).toHaveLength(100)
    expect(roster.spectatorCount).toBe(100)
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

describe('freeTeam', () => {
  const bot = (team: number): BotView => ({
    name: `BARb${team}`,
    owner: 'me',
    status: { ...seat(1), team },
    teamColour: 0,
    ai: 'BARb',
    options: {},
  })

  test('is the lowest number no player or AI holds', () => {
    const users = byName(
      user('alice', { ...seat(0), team: 0 }),
      user('bob', { ...seat(1), team: 2 }),
    )
    expect(freeTeam(room({ bots: [bot(1)] }), users, 'me')).toBe(3)
  })

  test('does not count our own seat, so moving is not a collision', () => {
    const users = byName(
      user('me', { ...seat(0), team: 0 }),
      user('alice', { ...seat(1), team: 1 }),
    )
    expect(freeTeam(room(), users, 'me')).toBe(0)
  })

  test('spectators hold no team', () => {
    const users = byName(user('alice', seat(0, false)))
    expect(freeTeam(room(), users, 'me')).toBe(0)
  })
})
