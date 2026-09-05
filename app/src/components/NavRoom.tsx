import { A } from '@solidjs/router'
import type { BattleView } from '../ipc/bindings/BattleView'
import { TILES } from '../lib/maps'
import { MapPicture } from './MapPicture'

/** What the card cuts or leaves out, in the room page's own words. */
export function glance(b: BattleView): string {
  return `${b.title}\n${b.mapName}\n${b.playerCount} players · ${b.spectatorCount} spectators`
}

/**
 * The room you are in, as a nav item: a way there and a glance at it.
 *
 * The count is the battle list's, since that is where you came from. `+0`
 * stays so the line keeps its width when the first spectator arrives.
 */
export function NavRoom(props: { battle: BattleView }) {
  return (
    <A href='/room' class='nav-room' title={glance(props.battle)}>
      <MapPicture
        class='nav-room-pic'
        mapName={props.battle.mapName}
        width={TILES.nav.width}
        height={TILES.nav.height}
      />
      <span class='nav-room-text'>
        <span class='nav-room-title'>{props.battle.title}</span>
        <span class='nav-room-count'>
          {props.battle.playerCount}/{props.battle.maxPlayers}
          <span class='nav-room-spectators'>
            {' '}
            +{props.battle.spectatorCount}
          </span>
        </span>
      </span>
    </A>
  )
}
