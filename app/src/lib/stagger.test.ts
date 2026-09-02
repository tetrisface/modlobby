import { describe, expect, test } from 'vitest'
import type { UserView } from '../ipc/bindings/UserView'
import { STAGGER_CAP, STAGGER_STEP, askDelay, askOrder } from './stagger'

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

function users(...list: UserView[]): Record<string, UserView> {
  return Object.fromEntries(list.map((held) => [held.name, held]))
}

describe('askOrder', () => {
  test('the boss goes first, whatever else is true of them', () => {
    const room = {
      members: ['ann', 'bob', 'cy'],
      boss: 'cy',
      users: users(
        user('ann', { player: true, rank: 7 }),
        user('bob', { player: true, rank: 5 }),
        user('cy', { player: false, rank: 0 }),
      ),
    }
    expect(askOrder(room)).toEqual(['cy', 'ann', 'bob'])
  })

  test('players before spectators, then higher rank first', () => {
    const room = {
      members: ['ann', 'bob', 'cy', 'dee'],
      boss: null,
      users: users(
        user('ann', { player: false, rank: 8 }),
        user('bob', { player: true, rank: 2 }),
        user('cy', { player: true, rank: 6 }),
        user('dee', { player: false, rank: 1 }),
      ),
    }
    expect(askOrder(room)).toEqual(['cy', 'bob', 'ann', 'dee'])
  })

  test('ties fall back to the lobby order', () => {
    const room = {
      members: ['bob', 'ann', 'cy'],
      boss: null,
      users: users(
        user('bob', { player: true, rank: 3 }),
        user('ann', { player: true, rank: 3 }),
        user('cy', { player: true, rank: 3 }),
      ),
    }
    expect(askOrder(room)).toEqual(['bob', 'ann', 'cy'])
  })

  test('bots do not hold a place, the host among them', () => {
    const room = {
      members: ['[teh]host', 'ann'],
      boss: null,
      users: users(
        user('[teh]host', { bot: true }),
        user('ann', { player: true }),
      ),
    }
    expect(askOrder(room)).toEqual(['ann'])
  })

  test('someone the lobby has not described yet still has a place', () => {
    const room = {
      members: ['ann', 'zed'],
      boss: null,
      users: users(user('ann', { player: true })),
    }
    expect(askOrder(room)).toEqual(['ann', 'zed'])
  })
})

describe('askDelay', () => {
  test('one step per place, from nothing for the first', () => {
    const room = {
      members: ['ann', 'bob', 'cy'],
      boss: null,
      users: users(
        user('ann', { player: true, rank: 3 }),
        user('bob', { player: true, rank: 2 }),
        user('cy', { player: true, rank: 1 }),
      ),
    }
    expect(askDelay({ ...room, me: 'ann' })).toBe(0)
    expect(askDelay({ ...room, me: 'bob' })).toBe(STAGGER_STEP)
    expect(askDelay({ ...room, me: 'cy' })).toBe(2 * STAGGER_STEP)
  })

  test('a big room does not wait forever', () => {
    const members = Array.from({ length: 30 }, (_, index) => `p${index}`)
    const room = { members, boss: null, users: {}, me: 'p29' }
    expect(askDelay(room)).toBe(STAGGER_CAP)
  })

  test('nobody in particular waits for nothing', () => {
    expect(
      askDelay({ me: null, members: ['ann'], boss: null, users: {} }),
    ).toBe(0)
    expect(
      askDelay({ me: 'ghost', members: ['ann'], boss: null, users: {} }),
    ).toBe(0)
  })
})
