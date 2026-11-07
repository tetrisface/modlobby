import { Show } from 'solid-js'
import type { ListedScore } from '../ipc/bindings/ListedScore'
import { SPAN } from '../lib/challenge'
import { challengeParts, challengeTip } from '../lib/listed'

/**
 * A room's last scored challenge, as a battle list row shows it: `12–15`,
 * the room as it was and the room with every seat taken, the second in full
 * colour and the first muted; one figure when there is only one. The scale,
 * who each is for and their age sit behind a hover. Nothing at all for a room
 * without a score, which is most rooms -- a dash in every row would be noise.
 */
export function ListedChallenge(props: {
	score: ListedScore | undefined
	now: number
}) {
	return (
		<Show when={props.score}>
			{(score) => {
				const parts = () => challengeParts(score())
				return (
					<span title={challengeTip(score(), props.now)}>
						<Show when={parts().now}>
							{(now) => (
								<>
									<span class='challenge-now'>{now()}</span>
									{SPAN}
								</>
							)}
						</Show>
						{parts().main}
					</span>
				)
			}}
		</Show>
	)
}
