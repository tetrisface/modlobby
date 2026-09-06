import { createContext, useContext, type Accessor } from 'solid-js'
import type { BattleView } from '../../ipc/bindings/BattleView'
import type { Book } from '../../ipc/bindings/Book'
import type { Plan } from '../../ipc/bindings/Plan'
import type { Sections } from '../../ipc/bindings/Sections'
import type { GameRunningView } from '../../ipc/bindings/GameRunningView'
import type { MyBattleView } from '../../ipc/bindings/MyBattleView'
import type { UserView } from '../../ipc/bindings/UserView'
import type { api } from '../../ipc/client'

/**
 * What the room asks Rust for.
 *
 * Handed in rather than imported, so the room never learns whether there is a
 * server behind it -- the same reason `store/tweakspace.ts` takes a `TweakIo`.
 *
 * Only the calls that read or write *this room* are here. Everything that
 * takes what it needs as arguments -- `encodeBoxes`, `gameModOptions`,
 * `describeMapOption`, the tweak formatter -- is the same answer either way and
 * is still called on `api` directly.
 */
export type RoomIo = Pick<
  typeof api,
  | 'setOption'
  | 'takeSeat'
  | 'releaseSeat'
  | 'setReady'
  | 'setSide'
  | 'addBot'
  | 'removeBot'
  | 'launch'
  | 'leaveBattle'
  | 'sayBattle'
  | 'startBoxes'
  | 'currentArrangement'
  | 'downloadMissing'
  | 'pveScore'
  | 'tweakSend'
  | 'tweakClear'
> & {
  /** Online this is `!rename`; in a room of your own it is just its name. */
  renameRoom(title: string): Promise<void>
  /** By its spring name, which is what a room and a start script both use. */
  setMap(name: string): Promise<void>
  /** 0 the map's own, 1 random, 2 chosen in the start boxes. */
  setStartPos(startPos: number): Promise<void>
  /** One of an AI's own options. An empty value restores its default. */
  setBotOption(name: string, key: string, value: string): Promise<void>
}

/**
 * Reading a preset into this room and writing one out of it.
 *
 * Apart from [`RoomIo`] because it is the one thing a room can be unable to
 * do at all: the preset page is drawn with no room behind it, and the buttons
 * that need one say so. `null` is that answer, and it is a better one than a
 * method that would fail.
 */
export type PresetIo = {
  savePreset(name: string): Promise<Book>
  applyPreset(name: string, sections: Sections): Promise<Plan>
}

/**
 * What may be done in this room.
 *
 * Asked as "may I", never as "am I offline": a component that checks a
 * capability keeps working when a third kind of room turns up, and one that
 * checks for a server does not. The names are the facts they stand for, so
 * both records below read as a description of the room rather than a list of
 * switches.
 */
export type RoomCaps = {
  /**
   * A SPADS autohost runs this room. It decides, so a change is a proposal;
   * it takes `!` commands; it rate-limits chat; and it is what hosting a room
   * of your own means. All of that is one fact, so it is one flag.
   */
  spads: boolean
  /** Other people to talk to. Off, the composer is a local command console. */
  chat: boolean
  /** A ready flag means something, because somebody is waiting on it. */
  ready: boolean
  /** The game is started from here rather than by a host. */
  startsGame: boolean
  /** There is a room to leave, as opposed to one that is simply yours. */
  leave: boolean
  /**
   * The map, game and engine are chosen here rather than by whoever set the
   * room up. Off, they are shown as what they are and changed by asking.
   */
  picksContent: boolean
}

/**
 * A battle room, whoever is behind it.
 *
 * `battle` and `my` are the wire's own view types rather than anything of this
 * seam's invention: a field added to `BattleView` for the online room has to be
 * answered for every room, and the compiler is what says so.
 */
export type RoomModel = {
  battle: Accessor<BattleView | undefined>
  my: Accessor<MyBattleView | null>
  users: Accessor<Record<string, UserView>>
  me: Accessor<string | null>
  /** Whether this machine has the room's engine, game and map. */
  content: Accessor<{ engine: boolean; game: boolean; map: boolean } | null>
  /** The room's game, while one is running. */
  running: Accessor<GameRunningView | null>
  /**
   * Where to go when there is no room here any more, or `null` to stay.
   *
   * The answer belongs to whoever knows what a missing room means -- a dropped
   * session, or nothing at all -- while the navigating stays in the view,
   * which is where the router is.
   */
  exit: Accessor<string | null>
  /** Which chat room this room's lines are filed under. */
  log: string
  caps: RoomCaps
  io: RoomIo
  /** Presets, where this room can carry them. */
  presets: Accessor<PresetIo | null>
}

const RoomContext = createContext<RoomModel>()

export const RoomProvider = RoomContext.Provider

/**
 * The room this tree is drawing. Throws outside a provider rather than
 * reaching for the online one: a component that fell out of its room should
 * say so here and not quietly start talking to the server.
 */
export function useRoom(): RoomModel {
  const room = useContext(RoomContext)
  if (room === undefined) {
    throw new Error('useRoom() outside a <RoomProvider>')
  }
  return room
}
