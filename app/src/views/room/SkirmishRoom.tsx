import { Show, onMount } from 'solid-js'
import { api, describeError } from '../../ipc/client'
import { pushNotice } from '../../store/chat'
import { lobby } from '../../store/lobby'
import { Room } from '../Room'
import { localRoom } from './local'
import { RoomProvider } from './model'

/**
 * A game against AI, set up in the room the lobby already draws.
 *
 * The room is opened on arrival if there is not one already, on the newest
 * game, map and engine this machine has — a room you can start a game from
 * without touching it, which is what somebody who came here to play wants.
 * Everything in it can then be changed.
 *
 * It is opened once and never closed on the way out: what is set up here
 * outlives leaving the page, a logout and a dropped connection, and coming
 * back to a room half-arranged is the whole point of keeping it.
 */
export function SkirmishRoom() {
  onMount(() => {
    if (lobby.skirmish !== null) return
    void api
      .skirmishOpen(null, null, null)
      .catch((error) => pushNotice('warning', describeError(error)))
  })

  return (
    <Show
      when={lobby.skirmish}
      fallback={<p class='muted empty-list'>Looking at what is installed…</p>}
    >
      <RoomProvider value={localRoom()}>
        <Room />
      </RoomProvider>
    </Show>
  )
}
