import { createStore } from 'solid-js/store'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { EngineStatus } from '../ipc/bindings/EngineStatus'
import type { GameRunningView } from '../ipc/bindings/GameRunningView'
import type { MyBattleView } from '../ipc/bindings/MyBattleView'
import type { Phase } from '../ipc/bindings/Phase'
import type { DownloadStatus } from '../ipc/bindings/DownloadStatus'
import type { FriendsView } from '../ipc/bindings/FriendsView'
import type { UserView } from '../ipc/bindings/UserView'

/** A dumb mirror of the runtime's state; only `apply.ts` writes to it. */
export type LobbyState = {
  phase: Phase | null
  me: string | null
  users: Record<string, UserView>
  battles: Record<number, BattleView>
  myBattle: MyBattleView | null
  gameRunning: GameRunningView | null
  engine: EngineStatus
  /** Whether this machine has the room's engine, game and map. */
  content: { engine: boolean; game: boolean; map: boolean } | null
  friends: FriendsView
  download: DownloadStatus
}

export function emptyLobby(): LobbyState {
  return {
    phase: null,
    me: null,
    users: {},
    battles: {},
    myBattle: null,
    gameRunning: null,
    engine: { state: 'idle' },
    content: null,
    friends: { friends: [], requests: [] },
    download: { state: 'idle' },
  }
}

export const [lobby, setLobby] = createStore<LobbyState>(emptyLobby())
