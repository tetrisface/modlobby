import { api } from '../../ipc/client'
import { BATTLE_ROOM } from '../../store/chat'
import { lobby, myRoom } from '../../store/lobby'
import type { RoomCaps, RoomModel } from './model'

/** A room on the server: someone else's, run by SPADS, full of people. */
const ONLINE: RoomCaps = {
  spads: true,
  chat: true,
  ready: true,
  // SPADS starts the game; what this end does is join the one that started.
  startsGame: false,
  leave: true,
  // The host's room, and `!map` is how anyone asks it to change.
  picksContent: false,
}

/**
 * The room we are in on the server.
 *
 * A thin reading of the mirrored state, so that what the room draws is what it
 * has always drawn -- this exists to be swapped for another implementation,
 * not to change anything.
 */
export function onlineRoom(): RoomModel {
  return {
    battle: () => myRoom(),
    my: () => lobby.myBattle,
    users: () => lobby.users,
    me: () => lobby.me,
    content: () => lobby.content,
    running: () => lobby.gameRunning,
    exit,
    log: BATTLE_ROOM,
    caps: ONLINE,
    io: {
      ...api,
      renameRoom: (title) => api.sayBattle(`!rename ${title}`),
      setMap: (name) => api.sayBattle(`!map ${name}`),
      setStartPos: (startPos) => api.sayBattle(`!set startPosType ${startPos}`),
      // SPADS keeps no per-AI options, and there is no command that would
      // set one. The room does not offer it, and this says so rather than
      // pretending.
      setBotOption: () =>
        Promise.reject(new Error('a room on the server keeps no AI options')),
    },
    // A preset is read from and written to a room, so outside one there is
    // nothing to offer -- which is what greys Save and Load on the page.
    presets: () => (myRoom() === undefined ? null : api),
  }
}

function exit(): string | null {
  // No connection at all -- logged out, or a launch that reopened on a stale
  // `#/room` hash. Either way there is no room here to be in. Waiting for
  // `ready` instead would leave an empty shell on screen indefinitely.
  if (lobby.phase === null) return '/'
  // Connected and in no room. Gated on `ready` so a reconnect, which has not
  // replayed `myBattle` yet, does not throw you out of the room you are
  // standing in.
  if (lobby.phase === 'ready' && !lobby.myBattle) return '/battles'
  return null
}
