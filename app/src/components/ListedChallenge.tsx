import { Show } from 'solid-js'
import type { ListedScore } from '../ipc/bindings/ListedScore'
import { challengeParts, challengeTip } from '../lib/listed'
import { FigurePair } from './FigurePair'
import { Gauge } from './icons'

/**
 * A room's last scored challenge, as a battle list row shows it: a gauge,
 * then `12–15`, the room as it is seated and the room with every seat taken,
 * the second in full colour and the first muted; one figure when there is
 * only one. The gauge says what the number is beside a map name; the scale,
 * who each figure is for and their age sit behind a hover. Nothing at all for
 * a room without a score, which is most rooms -- a dash in every row would be
 * noise.
 */
export function ListedChallenge(props: {
	score: ListedScore | undefined
	/** The room's head count now, which picks its figure from the table. */
	players: number
	now: number
}) {
	return (
		<Show when={props.score}>
			{(score) => (
				<span
					class='listed-challenge'
					title={challengeTip(score(), props.players, props.now)}
				>
					<Gauge />
					<b>
						<FigurePair parts={challengeParts(score(), props.players)} />
					</b>
				</span>
			)}
		</Show>
	)
}
