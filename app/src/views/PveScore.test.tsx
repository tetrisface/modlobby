import { cleanup, render } from '@solidjs/testing-library'
import { invoke } from '@tauri-apps/api/core'
import { reconcile } from 'solid-js/store'
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { BotView } from '../ipc/bindings/BotView'
import type { Score } from '../ipc/bindings/Score'
import type { UserView } from '../ipc/bindings/UserView'
import { STAGGER_STEP } from '../lib/stagger'
import { emptyLobby, setLobby } from '../store/lobby'
import { PveScore, QUIET_FOR } from './PveScore'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

/** A promise answered from the test, standing in for a slow service. */
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((yes, no) => {
    resolve = yes
    reject = no
  })
  return { promise, resolve, reject }
}

/** Lets an awaited answer reach the component and the DOM. */
async function settle() {
  for (let turn = 0; turn < 8; turn++) await Promise.resolve()
}

function bot(ai: string, handicap = 0): BotView {
  return {
    name: ai,
    owner: 'host',
    status: {
      ready: true,
      team: 1,
      allyTeam: 1,
      player: true,
      handicap,
      sync: 'bot',
      side: 0,
    },
    teamColour: 0,
    ai,
  }
}

function room(bots: BotView[]): BattleView {
  return {
    id: 7,
    founder: 'host',
    ip: '',
    port: 0,
    maxPlayers: 16,
    passworded: false,
    locked: false,
    mapHash: '',
    mapName: 'Comet Catcher Remake 1.8',
    engineName: 'spring',
    engineVersion: '',
    title: 'raptors',
    gameName: 'BAR',
    members: ['host', 'me'],
    spectatorCount: 0,
    playerCount: 1,
    layout: null,
    bots,
    startRects: [],
  }
}

function user(
  name: string,
  held: { player?: boolean; rank?: number; bot?: boolean } = {},
): UserView {
  return {
    name,
    country: '??',
    userId: null,
    lobbyClient: 'modlobby',
    status: {
      inGame: false,
      away: false,
      rank: held.rank ?? 0,
      moderator: false,
      bot: held.bot ?? false,
    },
    battleStatus: {
      ready: false,
      team: 0,
      allyTeam: 0,
      player: held.player ?? false,
      handicap: 0,
      sync: 'synced',
      side: 0,
    },
    battleId: 7,
  }
}

/**
 * A room where this client asks first: the host is a bot and holds no
 * place, so nothing waits. `others` puts people ahead of us.
 */
function enter(bots: BotView[], others: UserView[] = []) {
  const next = emptyLobby()
  next.me = 'me'
  next.battles[7] = room(bots)
  next.battles[7].members = ['host', 'me', ...others.map((held) => held.name)]
  next.users.host = user('host', { bot: true })
  next.users.me = user('me', { player: true })
  for (const held of others) next.users[held.name] = held
  next.myBattle = {
    boss: null,
    id: 7,
    gameHash: '',
    scriptTags: { 'game/modoptions/raptor_endless': '0' },
    vote: null,
    history: [],
  }
  setLobby(reconcile(next))
}

const scored: Score = {
  challenge: 21.5,
  percentile: 88,
  winChance: 0.31,
  evidenceGames: 4200,
  bestEffort: false,
}

const asked = vi.mocked(invoke)

describe('PveScore', () => {
  beforeEach(() => {
    // Date too: the pace between asks is measured on the clock.
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] })
    asked.mockReset()
  })
  afterEach(() => {
    // Without globals, nothing unmounts the last test's panel for us, and a
    // panel left behind would ask about this test's room too.
    cleanup()
    vi.useRealTimers()
    setLobby(reconcile(emptyLobby()))
  })

  test('every figure shows dots in its own slot while the service is asked', () => {
    asked.mockReturnValue(deferred<Score>().promise)
    enter([bot('RaptorsAI')])
    const { container } = render(() => <PveScore />)
    vi.advanceTimersByTime(0)

    expect(asked).toHaveBeenCalledWith('pve_score')
    const slots = container.querySelectorAll('.pve-figure b')
    expect(slots).toHaveLength(3)
    // Still ones: the panel is looked at for minutes, and a cycle there is noise.
    for (const slot of slots)
      expect(slot.querySelector('.thinking.still')).not.toBeNull()
    // The labels stay put around them, and nothing claims anything yet.
    expect(container.textContent).toContain('Challenge')
    expect(container.textContent).toContain('Win')
    expect(container.textContent).not.toContain('unplaced')
    expect(container.textContent).not.toContain('—')
  })

  test('the dots are there before the ask goes out, not a verdict', () => {
    asked.mockReturnValue(deferred<Score>().promise)
    // Someone outranks us, so our ask waits its turn.
    enter([bot('BARb')], [user('boss', { player: true, rank: 8 })])
    const { container } = render(() => <PveScore />)
    expect(asked).not.toHaveBeenCalled()
    expect(container.querySelectorAll('.pve-figure b .thinking')).toHaveLength(
      3,
    )
    expect(container.textContent).not.toContain('unplaced')

    vi.advanceTimersByTime(STAGGER_STEP)
    expect(asked).toHaveBeenCalledTimes(1)
  })

  test('the numbers land in their slots', async () => {
    asked.mockResolvedValue(scored)
    enter([bot('RaptorsAI')])
    const { container } = render(() => <PveScore />)
    vi.advanceTimersByTime(0)
    await settle()

    const slots = [...container.querySelectorAll('.pve-figure b')].map(
      (slot) => slot.textContent,
    )
    expect(slots).toEqual(['21.5/34', '31%', '88%'])
    expect(container.querySelector('.thinking')).toBeNull()
  })

  test('a setup the service cannot place gets the same blank as any other missing figure', async () => {
    asked.mockResolvedValue({ ...scored, challenge: null, percentile: null })
    enter([bot('BARb')])
    const { container } = render(() => <PveScore />)
    vi.advanceTimersByTime(0)
    await settle()

    const slots = [...container.querySelectorAll('.pve-figure b')].map(
      (slot) => slot.textContent,
    )
    expect(slots).toEqual(['—', '31%', '—'])
    expect(container.textContent).not.toContain('unplaced')
  })

  test('a change during a slow first ask is asked about once, afterwards', async () => {
    // The first ask is a cold start: the service is still thinking.
    const cold = deferred<Score>()
    asked.mockReturnValueOnce(cold.promise).mockResolvedValue(scored)
    enter([bot('RaptorsAI')])
    render(() => <PveScore />)
    vi.advanceTimersByTime(0)
    expect(asked).toHaveBeenCalledTimes(1)

    // Two settings change while it is out; each settles on its own.
    setLobby('myBattle', 'scriptTags', 'game/modoptions/raptor_endless', '1')
    vi.advanceTimersByTime(QUIET_FOR)
    setLobby(
      'myBattle',
      'scriptTags',
      'game/modoptions/raptor_graceperiod',
      '5',
    )
    vi.advanceTimersByTime(QUIET_FOR)
    // Nothing goes out alongside a question still being answered.
    expect(asked).toHaveBeenCalledTimes(1)

    cold.resolve({ ...scored, challenge: 5 })
    await settle()
    vi.advanceTimersByTime(0)
    // One follow-up covers both changes: Rust reads the room afresh.
    expect(asked).toHaveBeenCalledTimes(2)
  })

  test('a room that stops changing is asked about again after it settles', async () => {
    asked.mockResolvedValue(scored)
    enter([bot('ScavengersAI')])
    render(() => <PveScore />)
    vi.advanceTimersByTime(0)
    await settle()
    expect(asked).toHaveBeenCalledTimes(1)

    setLobby('myBattle', 'scriptTags', 'game/modoptions/scav_endless', '1')
    vi.advanceTimersByTime(QUIET_FOR - 1)
    expect(asked).toHaveBeenCalledTimes(1)
    vi.advanceTimersByTime(1)
    // The settle timer hands over to the pace's own zero-length timer, which
    // fake timers push one millisecond out when it is set inside a timer.
    vi.advanceTimersByTime(1)
    expect(asked).toHaveBeenCalledTimes(2)
  })

  test('a failure is said rather than hidden', async () => {
    asked.mockRejectedValue({ code: 'pve', message: 'pve stats answered 504' })
    const quiet = vi.spyOn(console, 'warn').mockImplementation(() => {})
    enter([bot('RaptorsAI')])
    const { container } = render(() => <PveScore />)
    vi.advanceTimersByTime(0)
    await settle()

    const said = container.querySelector('[title="pve stats answered 504"]')
    expect(said?.textContent).toBe('unavailable')
    quiet.mockRestore()
  })

  test('a room without a PvE opponent asks nothing and shows nothing', () => {
    enter([])
    const { container } = render(() => <PveScore />)
    expect(asked).not.toHaveBeenCalled()
    expect(container.querySelector('.pve-score')).toBeNull()
  })

  test('a room the setting turns off shows nothing', async () => {
    asked.mockResolvedValue(null)
    enter([bot('RaptorsAI')])
    const { container } = render(() => <PveScore />)
    vi.advanceTimersByTime(0)
    await settle()
    expect(container.querySelector('.pve-score')).toBeNull()
  })
})
