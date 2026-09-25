import { Show } from 'solid-js'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import { age, exactly } from '../lib/age'
import { sourceWords } from '../lib/mutators'

/**
 * What the room loads on top of its game, by name and how recent each is,
 * so a player sees what the game will be. Where each comes from, commit and
 * all, is in the tooltip rather than in the way.
 */
export function MutatorsChip(props: { mutators: readonly MutatorView[] }) {
	const shown = (mutator: MutatorView) =>
		mutator.date ? `${mutator.title} · ${age(mutator.date)}` : mutator.title
	const tip = (mutator: MutatorView) =>
		[
			mutator.title,
			mutator.source && sourceWords(mutator.source),
			mutator.date && `committed ${exactly(mutator.date)}`,
		]
			.filter(Boolean)
			.join(', ')
	return (
		<Show when={props.mutators.length > 0}>
			<span class='chip info' title={props.mutators.map(tip).join('\n')}>
				Mutators: {props.mutators.map(shown).join(', ')}
			</span>
		</Show>
	)
}
