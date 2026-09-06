import { createEffect, createRoot, on } from 'solid-js'
import { api } from '../ipc/client'
import { NO_ASSIST } from '../lib/assist'
import { readModOptions } from '../lib/setup'
import type { RoomModel } from '../views/room/model'
import { createTweakspace, type Tweakspace } from './tweakspace'

/**
 * The workspace for one room, made the first time that room asks for one.
 *
 * One per kind of room rather than one for the app: a workspace holds unsent
 * buffers and an undo history, and carrying those from a room on the server
 * into a skirmish would offer to send somebody's half-written tweak to the
 * wrong place. Keyed by the room's own log, which is the stable name a kind of
 * room already has.
 *
 * Kept apart from `createTweakspace` so that a test importing the factory does
 * not, as a side effect, start watching a real room.
 */
const spaces = new Map<string, Tweakspace>()

export function tweakspaceFor(room: RoomModel): Tweakspace {
  const held = spaces.get(room.log)
  if (held !== undefined) return held
  const made = wire(room)
  spaces.set(room.log, made)
  return made
}

function wire(room: RoomModel): Tweakspace {
  // Everything but sending is the same answer whoever is behind the room:
  // decoding, formatting and checking a tweak take it as an argument. Only
  // the two that put it somewhere come from the room.
  const space = createTweakspace(
    { ...api, tweakSend: room.io.tweakSend, tweakClear: room.io.tweakClear },
    () => readModOptions(room.my()?.scriptTags),
  )

  /**
   * What the room's game and engine can tell the editor, fetched when either
   * changes. An answer for a room that has since moved on is dropped.
   */
  createRoot(() => {
    createEffect(
      on(
        () => {
          const battle = room.battle()
          return [
            battle?.gameName ?? null,
            battle?.engineVersion ?? null,
          ] as const
        },
        ([game, engine]) => {
          if (game === null) {
            space.setAssist(NO_ASSIST)
            return
          }
          void Promise.all([
            api.gameUnitNames(game).catch(() => []),
            engine
              ? api.engineDefTags(engine).catch(() => ({ weapon: [] }))
              : Promise.resolve({ weapon: [] }),
          ]).then(([units, tags]) => {
            if (room.battle()?.gameName !== game) return
            space.setAssist({ units, weaponTags: tags.weapon })
          })
        },
      ),
    )
  })

  return space
}
