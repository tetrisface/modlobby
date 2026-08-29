import { describe, expect, test } from 'vitest'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { Delta } from '../ipc/bindings/Delta'
import type { Snapshot } from '../ipc/bindings/Snapshot'
import type { UserView } from '../ipc/bindings/UserView'
import { applyDelta, applySnapshot } from './apply'
import { chat } from './chat'
import { lobby } from './lobby'

const user = (name: string, battleId: number | null = null): UserView => ({
  name,
  country: 'SE',
  userId: 1,
  lobbyClient: 'LuaLobby Chobby',
  status: { inGame: false, away: false, rank: 0, moderator: false, bot: false },
  battleStatus: null,
  battleId,
})

const battle = (id: number, members: string[]): BattleView => ({
  id,
  founder: 'host',
  ip: '1.2.3.4',
  port: 8452,
  maxPlayers: 16,
  passworded: false,
  locked: false,
  mapHash: 'h',
  mapName: 'Map',
  engineName: 'spring',
  engineVersion: '2026.07.04',
  title: 'Room',
  gameName: 'BAR',
  members,
  spectatorCount: 1,
  playerCount: Math.max(0, members.length - 1),
  layout: null,
  bots: [],
  startRects: [],
})

const snapshot: Snapshot = {
  phase: 'ready',
  me: 'me',
  users: [user('me'), user('host', 5), user('alice')],
  battles: [battle(5, ['host'])],
  myBattle: null,
  gameRunning: null,
  engine: { state: 'idle' },
  channels: [],
  friends: { friends: [], requests: [], ignored: [] },
  download: { state: 'idle' },
}

describe('apply', () => {
  test('snapshot then deltas keep the mirror consistent', () => {
    applySnapshot(snapshot)
    expect(Object.keys(lobby.users)).toHaveLength(3)
    expect(lobby.battles[5]?.playerCount).toBe(0)

    const deltas: Delta[] = [
      { type: 'member', data: { id: 5, name: 'alice', joined: true } },
      {
        type: 'battleInfo',
        data: {
          id: 5,
          spectatorCount: 1,
          locked: true,
          mapHash: 'h',
          mapName: 'Map v2',
        },
      },
      {
        type: 'userStatus',
        data: {
          name: 'host',
          status: {
            inGame: true,
            away: false,
            rank: 0,
            moderator: false,
            bot: true,
          },
        },
      },
      {
        type: 'chat',
        data: {
          seq: 1,
          room: '#battle',
          from: 'host',
          text: 'hi',
          kind: 'announcement',
        },
      },
      { type: 'userRemoved', data: { name: 'alice' } },
    ]
    for (const delta of deltas) applyDelta(delta)

    expect(lobby.battles[5]?.members).toEqual(['alice', 'host'])
    expect(lobby.battles[5]?.playerCount).toBe(1)
    expect(lobby.battles[5]?.locked).toBe(true)
    expect(lobby.battles[5]?.mapName).toBe('Map v2')
    expect(lobby.users.host?.status.inGame).toBe(true)
    expect(lobby.users.alice).toBeUndefined()
    expect(chat.rooms['#battle']!.at(-1)?.text).toBe('hi')
  })

  test('disconnect resets the mirror', () => {
    applySnapshot(snapshot)
    applyDelta({ type: 'phase', data: null })
    expect(lobby.phase).toBeNull()
    expect(Object.keys(lobby.battles)).toHaveLength(0)
  })
})
