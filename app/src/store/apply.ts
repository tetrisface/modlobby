import { produce, reconcile } from 'solid-js/store'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { Delta } from '../ipc/bindings/Delta'
import type { Snapshot } from '../ipc/bindings/Snapshot'
import type { UiMessage } from '../ipc/bindings/UiMessage'
import {
  applyChannel,
  applyDirectory,
  clearChat,
  pushLine,
  pushNotice,
} from './chat'
import { emptyLobby, lobby, setLobby, type LobbyState } from './lobby'

export function applyMessage(message: UiMessage): void {
  if (message.type === 'snapshot') applySnapshot(message.data)
  else for (const delta of message.data) applyDelta(delta)
}

export function applySnapshot(snapshot: Snapshot): void {
  const next: LobbyState = {
    ...emptyLobby(),
    phase: snapshot.phase,
    me: snapshot.me,
    myBattle: snapshot.myBattle,
    gameRunning: snapshot.gameRunning,
    engine: snapshot.engine,
  }
  for (const user of snapshot.users) next.users[user.name] = user
  for (const battle of snapshot.battles) next.battles[battle.id] = battle
  setLobby(reconcile(next))
  if (snapshot.phase === null) clearChat()
  // Membership is replayed on reconnect; the chat backlog is not, so whatever
  // the front end still holds stays put.
  else
    for (const channel of snapshot.channels) applyChannel(channel.name, channel)
}

/** Mirrors `lobby-core`: members minus spectators, the host bot counting as one. */
function playerCount(battle: BattleView): number {
  return Math.max(0, battle.members.length - battle.spectatorCount)
}

export function applyDelta(delta: Delta): void {
  switch (delta.type) {
    case 'phase':
      setLobby('phase', delta.data)
      if (delta.data === null) {
        setLobby(reconcile(emptyLobby()))
      }
      return
    case 'userAdded':
      setLobby('users', delta.data.name, delta.data)
      return
    case 'userRemoved':
      setLobby(
        'users',
        produce((users) => {
          delete users[delta.data.name]
        }),
      )
      return
    case 'userStatus':
      if (lobby.users[delta.data.name]) {
        setLobby('users', delta.data.name, 'status', delta.data.status)
      }
      return
    case 'battleOpened':
      setLobby('battles', delta.data.id, delta.data)
      return
    case 'battleClosed':
      setLobby(
        'battles',
        produce((battles) => {
          delete battles[delta.data.id]
        }),
      )
      return
    case 'battleInfo': {
      const { id, spectatorCount, locked, mapHash, mapName } = delta.data
      if (!lobby.battles[id]) return
      setLobby(
        'battles',
        id,
        produce((battle) => {
          battle.spectatorCount = spectatorCount
          battle.locked = locked
          battle.mapHash = mapHash
          battle.mapName = mapName
          battle.playerCount = playerCount(battle)
        }),
      )
      return
    }
    case 'battleTitle':
      if (lobby.battles[delta.data.id]) {
        setLobby('battles', delta.data.id, 'title', delta.data.title)
      }
      return
    case 'battleLayout':
      if (lobby.battles[delta.data.id]) {
        setLobby('battles', delta.data.id, 'layout', delta.data.layout)
      }
      return
    case 'member': {
      const { id, name, joined } = delta.data
      if (lobby.battles[id]) {
        setLobby(
          'battles',
          id,
          produce((battle) => {
            const members = battle.members.filter((m) => m !== name)
            if (joined) members.push(name)
            battle.members = members.sort()
            battle.playerCount = playerCount(battle)
          }),
        )
      }
      if (lobby.users[name]) {
        setLobby('users', name, 'battleId', joined ? id : null)
      }
      return
    }
    case 'memberStatus':
      if (lobby.users[delta.data.name]) {
        setLobby('users', delta.data.name, 'battleStatus', delta.data.status)
      }
      return
    case 'bot': {
      const { id, name, bot } = delta.data
      if (!lobby.battles[id]) return
      setLobby(
        'battles',
        id,
        'bots',
        produce((bots) => {
          const index = bots.findIndex((b) => b.name === name)
          if (bot === null) {
            if (index >= 0) bots.splice(index, 1)
          } else if (index >= 0) {
            bots[index] = bot
          } else {
            bots.push(bot)
          }
        }),
      )
      return
    }
    case 'startRect': {
      const id = lobby.myBattle?.id
      if (id === undefined || !lobby.battles[id]) return
      const { allyTeam, rect } = delta.data
      setLobby(
        'battles',
        id,
        'startRects',
        produce((rects) => {
          const index = rects.findIndex((r) => r.allyTeam === allyTeam)
          if (rect === null) {
            if (index >= 0) rects.splice(index, 1)
          } else if (index >= 0) {
            rects[index] = rect
          } else {
            rects.push(rect)
          }
        }),
      )
      return
    }
    case 'scriptTags':
      if (!lobby.myBattle) return
      setLobby(
        'myBattle',
        'scriptTags',
        produce((tags) => {
          for (const [key, value] of delta.data.set) tags[key] = value
          for (const key of delta.data.removed) delete tags[key]
        }),
      )
      return
    case 'modOption': {
      if (!lobby.myBattle) return
      const { key, value, change } = delta.data
      setLobby(
        'myBattle',
        produce((my) => {
          if (!my) return
          my.scriptTags[`game/modoptions/${key}`] = value
          if (!change) return
          const index = my.history.findIndex((h) => h.seq === change.seq)
          if (index >= 0) my.history[index] = change
          else my.history.push(change)
        }),
      )
      return
    }
    case 'vote':
      if (lobby.myBattle) setLobby('myBattle', 'vote', delta.data)
      return
    case 'myBattle':
      setLobby('myBattle', delta.data)
      return
    case 'gameRunning':
      setLobby('gameRunning', delta.data)
      return
    case 'engine':
      setLobby('engine', delta.data)
      return
    case 'content':
      setLobby('content', delta.data)
      return
    case 'chat':
      pushLine(delta.data)
      return
    case 'channel':
      applyChannel(delta.data.name, delta.data.channel)
      return
    case 'directory':
      applyDirectory(delta.data)
      return
    case 'notice':
      pushNotice(delta.data.level, delta.data.text)
      return
  }
}
