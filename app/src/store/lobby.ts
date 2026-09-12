import { createStore } from 'solid-js/store'
import type { BattleView } from '../ipc/bindings/BattleView'
import type { EngineStatus } from '../ipc/bindings/EngineStatus'
import type { GameRunningView } from '../ipc/bindings/GameRunningView'
import type { MyBattleView } from '../ipc/bindings/MyBattleView'
import type { Phase } from '../ipc/bindings/Phase'
import type { SkirmishView } from '../ipc/bindings/SkirmishView'
import type { LanView } from '../ipc/bindings/LanView'
import type { DownloadStatus } from '../ipc/bindings/DownloadStatus'
import type { PasteStatus } from '../ipc/bindings/PasteStatus'
import type { FriendsView } from '../ipc/bindings/FriendsView'
import type { UserView } from '../ipc/bindings/UserView'

/** A dumb mirror of the runtime's state; only `apply.ts` writes to it. */
export type LobbyState = {
  phase: Phase | null
  /**
   * When the runtime next tries the last credentials on its own, as a
   * `Date.now()` moment, while it means to. Made here from the seconds the
   * runtime sends, so the corner can count down without another round trip.
   */
  retryAt: number | null
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
  /** A multi-line paste on its way to the room. */
  paste: PasteStatus
  /**
   * The room with no server behind it.
   *
   * Its own branch rather than a row in `battles`, because it is not the
   * session's: it is still here after a logout, a dropped connection or a
   * reconnect, all of which clear everything above.
   */
  skirmish: SkirmishView | null
  /**
   * The games being hosted on this network, and who else is on it.
   *
   * Its own branch for the same reason `skirmish` is: a LAN game needs no
   * server, so it is there whether or not anyone is logged in and survives
   * everything a dropped session clears. On macOS, where the only engine
   * available may not reach the community servers, it is the whole of
   * multiplayer.
   */
  lan: LanView
}

export function emptyLobby(): LobbyState {
  return {
    phase: null,
    retryAt: null,
    me: null,
    users: {},
    battles: {},
    myBattle: null,
    gameRunning: null,
    engine: { state: 'idle' },
    content: null,
    friends: { friends: [], requests: [], ignored: [] },
    download: { state: 'idle' },
    paste: { state: 'idle' },
    skirmish: null,
    lan: { games: [], people: [], listening: false },
  }
}

export const [lobby, setLobby] = createStore<LobbyState>(emptyLobby())

/**
 * The room you are in, if it is still on the list.
 *
 * `myBattle` can point at a row that is gone — closed under us, or not yet
 * replayed after a reconnect — so callers get nothing rather than a room
 * with empty lines.
 */
export function myRoom(): BattleView | undefined {
  const id = lobby.myBattle?.id
  return id === undefined ? undefined : lobby.battles[id]
}
