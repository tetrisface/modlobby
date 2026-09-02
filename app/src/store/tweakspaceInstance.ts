import { createEffect, createRoot, on } from 'solid-js'
import { api } from '../ipc/client'
import { NO_ASSIST } from '../lib/assist'
import { readModOptions } from '../lib/setup'
import { lobby } from './lobby'
import { createTweakspace } from './tweakspace'

/**
 * The one workspace, wired to Rust and to the room we are in.
 *
 * Apart from the factory so that a test importing `createTweakspace` does
 * not, as a side effect, start watching the real lobby.
 */
export const tweakspace = createTweakspace(api, () =>
  readModOptions(lobby.myBattle?.scriptTags),
)

/**
 * What the room's game and engine can tell the editor, fetched when either
 * changes. An answer for a room we have since left is dropped.
 */
createRoot(() => {
  createEffect(
    on(
      () => {
        const battle = lobby.battles[lobby.myBattle?.id ?? -1]
        return [
          battle?.gameName ?? null,
          battle?.engineVersion ?? null,
        ] as const
      },
      ([game, engine]) => {
        if (game === null) {
          tweakspace.setAssist(NO_ASSIST)
          return
        }
        void Promise.all([
          api.gameUnitNames(game).catch(() => []),
          engine
            ? api.engineDefTags(engine).catch(() => ({ weapon: [] }))
            : Promise.resolve({ weapon: [] }),
        ]).then(([units, tags]) => {
          const now = lobby.battles[lobby.myBattle?.id ?? -1]
          if (now?.gameName !== game) return
          tweakspace.setAssist({ units, weaponTags: tags.weapon })
        })
      },
    ),
  )
})
