import { Presets } from './Presets'
import { RoomProvider } from './room/model'
import { onlineRoom } from './room/online'

/**
 * The preset table as a page of its own, reachable from the nav with nobody
 * logged in. The same component the room's pane draws; only Save and Load
 * need a room, and they say so.
 *
 * The room offered here is the one on the server, which outside a room answers
 * "none" -- exactly what disables those two buttons.
 */
export function PresetsPage() {
  return (
    <section class='presets-page'>
      <RoomProvider value={onlineRoom()}>
        <Presets />
      </RoomProvider>
    </section>
  )
}
