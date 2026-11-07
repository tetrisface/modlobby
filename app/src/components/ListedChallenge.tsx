import { Show } from 'solid-js'
import type { ListedScore } from '../ipc/bindings/ListedScore'
import { challengeLabel, challengeTip } from '../lib/listed'

/**
 * A room's last scored challenge, as a battle list row shows it: the figure,
 * with its scale and age behind a hover. Nothing at all for a room without
 * one, which is most rooms -- a dash in every row would be noise.
 */
export function ListedChallenge(props: {
	score: ListedScore | undefined
	now: number
}) {
	return (
		<Show when={props.score}>
			{(score) => (
				<span title={challengeTip(score(), props.now)}>
					{challengeLabel(score())}
				</span>
			)}
		</Show>
	)
}
