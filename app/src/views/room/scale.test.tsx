import { MemoryRouter, Route } from '@solidjs/router'
import { cleanup, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { createStore } from 'solid-js/store'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import { Room } from '../Room'
import { SERVED, battle, fakeRoom, recordingIo, status, user } from './fixture'
import { RoomProvider, type RoomModel } from './model'

/**
 * The room at event size: a 64-way free-for-all, and four teams of 25 with
 * a hundred watching and all of them in line for a seat.
 *
 * The page was laid out for a 16-player room. What is checked here is how
 * it gives way beyond that: every name drawn once and in its place, the
 * queue numbered through, the teams as many as the shape says. Speed is
 * only bounded, not measured -- the budget is wide enough for any CI box
 * and narrow enough that work growing with the square of the room would
 * trip it.
 */
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
  convertFileSrc: (path: string, scheme: string) => `${scheme}://${path}`,
}))

beforeEach(() => {
  vi.mocked(invoke).mockImplementation(async (command: string) => {
    switch (command) {
      case 'game_modoptions':
      case 'game_ais':
      case 'game_unit_names':
        return []
      case 'engine_def_tags':
        return { weapon: [] }
      default:
        return null
    }
  })
})

afterEach(() => {
  cleanup()
  vi.clearAllMocks()
})

/**
 * Milliseconds a draw may take, whatever the crowd. Measured 2026-09-10 in
 * happy-dom on the dev box: 73 ms for the 64-way FFA, under 40 ms for the
 * other first draws, 590 ms for the burst of two hundred -- so this is a
 * tripwire for work that grows with the square of the room, not a
 * performance claim.
 */
const BUDGET_MS = 5000

async function open(model: RoomModel) {
  const started = performance.now()
  const result = render(() => (
    <MemoryRouter
      root={(props) => (
        <RoomProvider value={model}>{props.children}</RoomProvider>
      )}
    >
      <Route path='/' component={Room} />
    </MemoryRouter>
  ))
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
  return { ...result, elapsed: performance.now() - started }
}

type Crowd = {
  teams: number
  teamSize: number
  spectators: number
  queued: number
  /** Nobody's status has arrived yet: the room as it is the moment we walk in. */
  arriving?: boolean
}

/**
 * Me watching, `teams` × `teamSize` players dealt round the sides, and
 * `spectators` more watching, the first `queued` of them in line.
 */
function crowd(c: Crowd): RoomModel {
  const seat = (i: number) =>
    c.arriving ? null : status({ allyTeam: i % c.teams, team: i })
  const players = Array.from({ length: c.teams * c.teamSize }, (_, i) =>
    user(`player${i}`, { battleStatus: seat(i) }),
  )
  const watching = c.arriving ? null : status({ player: false })
  const watchers = Array.from({ length: c.spectators }, (_, i) =>
    user(`spec${i}`, { battleStatus: watching }),
  )
  const everyone = [
    user('me', { battleStatus: status({ player: false }) }),
    ...players,
    ...watchers,
  ]
  return fakeRoom({
    caps: SERVED,
    battle: () =>
      battle({
        members: everyone.map((u) => u.name).sort(),
        playerCount: players.length,
        spectatorCount: watchers.length + 1,
        layout: { teams: c.teams, teamSize: c.teamSize },
        queue: watchers.slice(0, c.queued).map((u) => u.name),
      }),
    users: () => Object.fromEntries(everyone.map((u) => [u.name, u])),
    io: recordingIo([]),
  })
}

const texts = (container: HTMLElement, selector: string) =>
  [...container.querySelectorAll(selector)].map((el) => el.textContent ?? '')

describe('the room at event size', () => {
  test('a 64-way free-for-all draws a card per side, one name each', async () => {
    const { container, elapsed } = await open(
      crowd({ teams: 64, teamSize: 1, spectators: 100, queued: 30 }),
    )
    expect(elapsed).toBeLessThan(BUDGET_MS)

    expect(container.querySelectorAll('.team')).toHaveLength(64)
    expect(container.querySelectorAll('.team .player')).toHaveLength(64)
    expect(container.querySelectorAll('.player.empty')).toHaveLength(0)

    // Every member once, nobody twice, nobody lost.
    const names = texts(container, '.rosters .pname')
    expect(names).toHaveLength(165)
    expect(new Set(names).size).toBe(165)
  })

  test('four sides of 25 with a hundred watching, all of them in line', async () => {
    const { container, elapsed } = await open(
      crowd({ teams: 4, teamSize: 25, spectators: 100, queued: 100 }),
    )
    expect(elapsed).toBeLessThan(BUDGET_MS)

    const teams = [...container.querySelectorAll('.team')]
    expect(teams).toHaveLength(4)
    for (const team of teams)
      expect(team.querySelectorAll('.player')).toHaveLength(25)

    // The queue is numbered through, in the server's order.
    expect(texts(container, '.queue-list .place')).toEqual(
      Array.from({ length: 100 }, (_, i) => `${i + 1}.`),
    )
    expect(texts(container, '.queue-list .pname')).toEqual(
      Array.from({ length: 100 }, (_, i) => `spec${i}`),
    )
    // Which leaves one spectator, and a count that includes the queue.
    expect(texts(container, '.spectator-list .pname')).toEqual(['me'])
    expect(texts(container, '.spectators .team-head .count')).toEqual([
      '100',
      '101',
    ])
  })

  test('two hundred arriving are dealt to the shape, the rest dimmed below', async () => {
    const { container, elapsed } = await open(
      crowd({
        teams: 4,
        teamSize: 25,
        spectators: 100,
        queued: 0,
        arriving: true,
      }),
    )
    expect(elapsed).toBeLessThan(BUDGET_MS)

    expect(container.querySelectorAll('.team')).toHaveLength(4)
    // Every seat the shape says is guessed at, none left empty.
    expect(container.querySelectorAll('.player.pending')).toHaveLength(100)
    expect(container.querySelectorAll('.player.empty')).toHaveLength(0)
    // The other hundred wait, dimmed, among the spectators.
    expect(container.querySelectorAll('.spectator.pending')).toHaveLength(100)
    expect(texts(container, '.rosters .pname')).toHaveLength(201)
  })

  /**
   * The join burst: one `CLIENTBATTLESTATUS` per member, each of which
   * re-arranges the whole roster. Two hundred of them, one at a time, is the
   * heaviest thing a room ever asks of the page.
   */
  test('two hundred statuses landing one at a time settle every guess', async () => {
    const model = crowd({
      teams: 4,
      teamSize: 25,
      spectators: 100,
      queued: 0,
      arriving: true,
    })
    const [users, setUsers] = createStore(model.users())
    const { container } = await open(fakeRoom({ ...model, users: () => users }))
    expect(container.querySelectorAll('.player.pending')).toHaveLength(100)

    const started = performance.now()
    for (let i = 0; i < 100; i++)
      setUsers(
        `player${i}`,
        'battleStatus',
        status({ allyTeam: i % 4, team: i }),
      )
    for (let i = 0; i < 100; i++)
      setUsers(`spec${i}`, 'battleStatus', status({ player: false }))
    expect(performance.now() - started).toBeLessThan(BUDGET_MS)

    expect(container.querySelectorAll('.player.pending')).toHaveLength(0)
    expect(container.querySelectorAll('.spectator.pending')).toHaveLength(0)
    expect(container.querySelectorAll('.team .player')).toHaveLength(100)
    expect(texts(container, '.spectator-list .pname')).toHaveLength(101)
  })
})
