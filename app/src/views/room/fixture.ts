import type { BattleStatusView } from '../../ipc/bindings/BattleStatusView'
import type { BattleView } from '../../ipc/bindings/BattleView'
import type { BotView } from '../../ipc/bindings/BotView'
import type { MyBattleView } from '../../ipc/bindings/MyBattleView'
import type { Book } from '../../ipc/bindings/Book'
import type { Plan } from '../../ipc/bindings/Plan'
import type { Prepared } from '../../ipc/bindings/Prepared'
import type { UserView } from '../../ipc/bindings/UserView'
import type { RoomCaps, RoomIo, RoomModel } from './model'

/**
 * A room made up on the spot, for tests and for anything that needs to draw
 * one without a server behind it.
 *
 * The point of the seam is that a room is describable as plain values, so this
 * is what proves it: everything here is a literal, and `io` records what the
 * room asked for rather than asking anyone.
 */

export function status(over: Partial<BattleStatusView> = {}): BattleStatusView {
  return {
    ready: false,
    team: 0,
    allyTeam: 0,
    player: true,
    handicap: 0,
    sync: 'synced',
    side: 0,
    ...over,
  }
}

export function user(name: string, over: Partial<UserView> = {}): UserView {
  return {
    name,
    country: 'SE',
    userId: 1,
    lobbyClient: 'modlobby',
    status: {
      inGame: false,
      away: false,
      rank: 1,
      moderator: false,
      bot: false,
    },
    battleStatus: status(),
    battleId: 1,
    ...over,
  }
}

export function bot(name: string, over: Partial<BotView> = {}): BotView {
  return {
    name,
    owner: 'me',
    status: status({ allyTeam: 1, team: 1, sync: 'bot' }),
    teamColour: 0x4b73f2,
    ai: name,
    options: {},
    ...over,
  }
}

export function battle(over: Partial<BattleView> = {}): BattleView {
  return {
    id: 1,
    founder: 'me',
    ip: '',
    port: 0,
    maxPlayers: 16,
    passworded: false,
    locked: false,
    mapHash: '',
    mapName: 'Comet Catcher Remake 1.8',
    engineName: 'spring',
    engineVersion: '2026.07.04',
    title: 'A room',
    gameName: 'Beyond All Reason test-31134',
    members: ['me'],
    spectatorCount: 0,
    playerCount: 1,
    layout: null,
    bots: [],
    startRects: [],
    queue: [],
    ...over,
  }
}

export function myBattle(over: Partial<MyBattleView> = {}): MyBattleView {
  return {
    boss: null,
    autoBalance: 'off',
    preset: null,
    id: 1,
    gameHash: '',
    scriptTags: {},
    vote: null,
    history: [],
    ...over,
  }
}

/** What the room asked for, in order: the name and the arguments. */
export type Calls = Array<[string, unknown[]]>

const NO_BOOK: Book = { version: 1, presets: [] }
const NO_PLAN: Plan = {
  lines: [],
  startBoxes: [],
  startBoxesUnsent: false,
  alreadySet: 0,
}
const NOTHING_PREPARED: Prepared = {
  minified: '',
  blob: '',
  command: '',
  gauge: { raw: 0, minified: 0, blob: 0, command: 0, cap: 16385, fits: true },
}

/**
 * An io that answers every call with the emptiest true thing and writes down
 * what it was asked.
 *
 * `satisfies RoomIo` rather than a cast, so the answers have to be the shapes
 * Rust really returns -- a fake that lies is worse than no fake at all.
 */
export function recordingIo(calls: Calls): RoomIo {
  const note = (name: string, ...args: unknown[]) => {
    calls.push([name, args])
  }
  return {
    setOption: async (key, value) => note('setOption', key, value),
    takeSeat: async (team, allyTeam) => note('takeSeat', team, allyTeam),
    releaseSeat: async () => note('releaseSeat'),
    setReady: async (ready) => note('setReady', ready),
    setSide: async (side) => note('setSide', side),
    addBot: async (name, ai, team, allyTeam, colour) =>
      note('addBot', name, ai, team, allyTeam, colour),
    updateBot: async (name, team, allyTeam, handicap, colour) =>
      note('updateBot', name, team, allyTeam, handicap, colour),
    removeBot: async (name) => note('removeBot', name),
    launch: async () => note('launch'),
    leaveBattle: async () => note('leaveBattle'),
    sayBattle: async (text) => note('sayBattle', text),
    renameRoom: async (title) => note('renameRoom', title),
    setMap: async (name) => note('setMap', name),
    setStartPos: async (startPos) => note('setStartPos', startPos),
    setBotOption: async (name, key, value) =>
      note('setBotOption', name, key, value),
    startBoxes: async (teams) => {
      note('startBoxes', teams)
      return null
    },
    currentArrangement: async (teams) => {
      note('currentArrangement', teams)
      return null
    },
    downloadMissing: async () => note('downloadMissing'),
    pveScore: async () => {
      note('pveScore')
      return null
    },
    tweakSend: async (lua, slot, direct) => {
      note('tweakSend', lua, slot, direct)
      return NOTHING_PREPARED
    },
    tweakClear: async (slot) => note('tweakClear', slot),
  } satisfies RoomIo
}

/** A room nobody else is in: the shape a skirmish will have. */
export const ALONE: RoomCaps = {
  spads: false,
  chat: false,
  ready: false,
  startsGame: true,
  leave: false,
  picksContent: true,
  plays: true,
}

/** A room on the server, as `onlineRoom()` describes one. */
export const SERVED: RoomCaps = {
  spads: true,
  chat: true,
  ready: true,
  startsGame: false,
  leave: true,
  picksContent: false,
  plays: true,
}

export function fakeRoom(over: Partial<RoomModel> = {}): RoomModel {
  return {
    battle: () => battle(),
    my: () => myBattle(),
    users: () => ({ me: user('me') }),
    me: () => 'me',
    content: () => ({ engine: true, game: true, map: true }),
    running: () => null,
    exit: () => null,
    log: '#battle',
    caps: SERVED,
    io: recordingIo([]),
    presets: () => null,
    ...over,
  }
}
