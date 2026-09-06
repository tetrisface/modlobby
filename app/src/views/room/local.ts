import { api } from '../../ipc/client'
import { SKIRMISH_ROOM } from '../../store/chat'
import { lobby } from '../../store/lobby'
import type { RoomCaps, RoomIo, RoomModel } from './model'

const act = api.skirmishAct

/**
 * The room's side of the seam, against a room with no server behind it.
 *
 * `satisfies RoomIo` is what keeps this honest: it has to answer everything
 * the online room answers, in the same shapes, or it does not compile — which
 * is the same trick `SkirmishView` plays on the Rust side.
 */
export const skirmishIo = {
  setOption: (key, value) => act({ type: 'setOption', key, value }),
  takeSeat: (team, allyTeam) => act({ type: 'takeSeat', team, allyTeam }),
  releaseSeat: () => act({ type: 'releaseSeat' }),
  // Nobody is waiting on you, so the seat bar does not draw this at all; it is
  // here because the seam is one shape and refusing would be a lie about what
  // happened rather than about what it means.
  setReady: () => Promise.resolve(),
  setSide: (side) => act({ type: 'setSide', side }),
  addBot: (name, ai, team, allyTeam, colour) =>
    act({ type: 'addBot', name, ai, team, allyTeam, colour }),
  removeBot: (name) => act({ type: 'removeBot', name }),
  launch: () => api.skirmishLaunch(),
  // There is no room to leave, only one to put away.
  leaveBattle: () => api.skirmishClose(),
  // The composer is a console here; the room answers in its own log.
  sayBattle: (text) => act({ type: 'say', text }),
  renameRoom: (title) => act({ type: 'setTitle', title }),
  setMap: (map) => act({ type: 'setMap', map }),
  startBoxes: (teams) => api.skirmishStartBoxes(teams),
  currentArrangement: (teams) => api.skirmishCurrentArrangement(teams),
  downloadMissing: () => api.skirmishDownloadMissing(),
  pveScore: () => api.skirmishPveScore(),
  tweakSend: (lua, slot, direct) => api.skirmishTweakSend(lua, slot, direct),
  tweakClear: (slot) => api.skirmishTweakClear(slot),
} satisfies RoomIo

/** A room nobody else is in. */
const ALONE: RoomCaps = {
  spads: false,
  chat: false,
  ready: false,
  startsGame: true,
  leave: false,
  picksContent: true,
}

/**
 * The room with no server behind it.
 *
 * Everything it draws comes from `lobby.skirmish`, which the runtime replaces
 * whole on every change and which survives a logout, a dropped connection and
 * a reloaded window — none of which have anything to do with a game against
 * AI on this machine.
 */
export function localRoom(): RoomModel {
  return {
    battle: () => lobby.skirmish?.battle,
    my: () => lobby.skirmish?.my ?? null,
    users: () => {
      const users = lobby.skirmish?.users ?? []
      return Object.fromEntries(users.map((user) => [user.name, user]))
    },
    me: () => lobby.skirmish?.me ?? null,
    content: () => lobby.skirmish?.content ?? null,
    // A skirmish's engine is the only game it has; `lobby.gameRunning` is the
    // server telling us about somebody else's, which is a different question.
    running: () => null,
    // The room is always here once it is open, and while it is opening there
    // is nowhere better to be than waiting for it.
    exit: () => null,
    log: SKIRMISH_ROOM,
    caps: ALONE,
    io: skirmishIo,
    // Reading a preset into this room is still to come; offering a button that
    // could not do it would be worse than not offering one.
    presets: () => null,
  }
}
