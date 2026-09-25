import { Show } from 'solid-js'
import type { MutatorView } from '../ipc/bindings/MutatorView'
import { age, exactly } from '../lib/age'
import { sourceWords } from '../lib/mutators'

/**
 * What the room loads on top of its game, so a player sees what the game
 * will be. One mod reads with how recent it is; several read as a count and
 * their names, cut short at the chip's width -- ten would otherwise run the
 * header over two rows. The tooltip has each on a line of its own: how
 * recent, and where it comes from, commit and all.
 */
export function ModsChip(props: { mods: readonly MutatorView[] }) {
	const text = () => {
		const [only] = props.mods
		if (props.mods.length === 1 && only)
			return only.date
				? `Mods: ${only.title} · ${age(only.date)}`
				: `Mods: ${only.title}`
		return `${props.mods.length} mods: ${props.mods.map((mod) => mod.title).join(', ')}`
	}
	const tip = (mod: MutatorView) =>
		[
			mod.title,
			mod.date && age(mod.date),
			mod.source && sourceWords(mod.source),
			mod.date && `committed ${exactly(mod.date)}`,
		]
			.filter(Boolean)
			.join(' · ')
	return (
		<Show when={props.mods.length > 0}>
			<span class='chip info mods-chip' title={props.mods.map(tip).join('\n')}>
				{text()}
			</span>
		</Show>
	)
}
