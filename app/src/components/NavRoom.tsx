import { A } from '@solidjs/router'
import { Show } from 'solid-js'
import type { BattleView } from '../ipc/bindings/BattleView'
import { TILES } from '../lib/maps'
import { MapPicture } from './MapPicture'

/** What the card cuts or leaves out, in the room page's own words. */
export function glance(b: BattleView, spectators: number): string {
	return `${b.title}\n${b.mapName}\n${b.playerCount} players · ${spectators} spectators`
}

/**
 * The room you are in, as a nav item: a way there and a glance at it.
 *
 * The count is the battle list's, since that is where you came from: the
 * players over the seats, and the people watching where there are any. The
 * nav row measures itself, so the card growing by a count costs nothing
 * (`NavTabs`).
 */
export function NavRoom(props: {
	battle: BattleView
	/** The people watching, the autohost left out; see `lib/battles`. */
	spectators: number
	/** The card's own element, for a row that measures what it holds. */
	ref?: HTMLAnchorElement | ((el: HTMLAnchorElement) => void)
}) {
	return (
		<A
			href='/room'
			class='nav-room'
			title={glance(props.battle, props.spectators)}
			ref={props.ref}
		>
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
					<Show when={props.spectators > 0}>
						<span class='nav-room-spectators'> +{props.spectators}</span>
					</Show>
				</span>
			</span>
		</A>
	)
}
