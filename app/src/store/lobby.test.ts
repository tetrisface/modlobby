import { reconcile } from 'solid-js/store'
import { afterEach, describe, expect, test } from 'vitest'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { MyBattleView } from '../ipc/bindings/MyBattleView'
import { emptyLobby, myRoom, setLobby } from './lobby'

const battle = (id: number): BattleView => ({
  id,
  founder: 'host',
  ip: '',
  port: 0,
  maxPlayers: 16,
  passworded: false,
  locked: false,
  mapHash: '',
  mapName: 'Map',
  engineName: '',
  engineVersion: '',
  title: 'Room',
  gameName: '',
  members: ['host'],
  spectatorCount: 1,
  playerCount: 0,
  layout: null,
  bots: [],
  startRects: [],
})

const mine = (id: number): MyBattleView => ({
  boss: null,
  id,
  gameHash: '',
  scriptTags: {},
  vote: null,
  history: [],
})

afterEach(() => setLobby(reconcile(emptyLobby())))

describe('myRoom', () => {
  test('nothing while not in a room', () => {
    setLobby('battles', { 5: battle(5) })
    expect(myRoom()).toBeUndefined()
  })

  test('nothing when the room is no longer listed', () => {
    setLobby('myBattle', mine(5))
    expect(myRoom()).toBeUndefined()
  })

  test('the listed room otherwise', () => {
    setLobby('battles', { 5: battle(5) })
    setLobby('myBattle', mine(5))
    expect(myRoom()?.title).toBe('Room')
  })
})
