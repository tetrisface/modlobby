import { Presets } from './Presets'

/**
 * The preset table as a page of its own, reachable from the nav with nobody
 * logged in. The same component the room's pane draws; only Save and Load
 * need a room, and they say so.
 */
export function PresetsPage() {
  return (
    <section class='presets-page'>
      <Presets />
    </section>
  )
}
