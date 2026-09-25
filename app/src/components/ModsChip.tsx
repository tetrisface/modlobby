import { Show } from 'solid-js'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import { age, exactly } from '../lib/age'
import { sourceWords } from '../lib/mutators'

/**
 * What the room loads on top of its game, by name and how recent each is,
 * so a player sees what the game will be. Where each comes from, commit and
 * all, is in the tooltip rather than in the way.
 */
export function ModsChip(props: { mods: readonly MutatorView[] }) {
	const shown = (mod: MutatorView) =>
		mod.date ? `${mod.title} · ${age(mod.date)}` : mod.title
	const tip = (mod: MutatorView) =>
		[
			mod.title,
			mod.source && sourceWords(mod.source),
			mod.date && `committed ${exactly(mod.date)}`,
		]
			.filter(Boolean)
			.join(', ')
	return (
		<Show when={props.mods.length > 0}>
			<span class='chip info' title={props.mods.map(tip).join('\n')}>
				Mods: {props.mods.map(shown).join(', ')}
			</span>
		</Show>
	)
}
