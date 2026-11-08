import { Show } from 'solid-js'
import { type ChallengeParts, SPAN } from '../lib/challenge'

/**
 * A room's figure as seated and with every seat taken, `now–full`: the seated
 * one muted, the full team in full colour, since that is what a lobby still
 * filling is heading for. One figure where there is only one. The battle
 * list, the room's row and its panel all write them this way; see
 * `challengePair` for when there are two.
 */
export function FigurePair(props: { parts: ChallengeParts; unit?: string }) {
	return (
		<>
			<Show when={props.parts.now}>
				{(now) => (
					<>
						<span class='challenge-now'>{now()}</span>
						{SPAN}
					</>
				)}
			</Show>
			{props.parts.main}
			{props.unit}
		</>
	)
}
