import { For, createUniqueId } from 'solid-js'
import type { CheckView } from '../ipc/bindings/CheckView'
import type { ContentCheckView } from '../ipc/bindings/ContentCheckView'
import { exactly } from '../lib/age'
import { sourceWords } from '../lib/mutators'

/**
 * The room's content, all of it here: one chip, and on hover or focus each
 * part by name with what is known of it. Where the room announced a checksum,
 * the copy on this disk is held to it -- the same files, or not -- so that
 * nobody has to wonder whether they are running the right game. A part that
 * differs turns the chip into a warning.
 */
export function ContentChip(props: {
	engine: string
	game: string
	map: string
	check: ContentCheckView
}) {
	const tip = createUniqueId()
	const parts = () => [
		{ what: 'Engine', name: props.engine, check: null, built: null },
		{ what: 'Game', name: props.game, check: props.check.game, built: null },
		{ what: 'Map', name: props.map, check: props.check.map, built: null },
		...props.check.mutators.map((mutator) => ({
			what: 'Mutator',
			name: mutator.title,
			check: mutator.check,
			built: mutator.source && builtFrom(mutator.source, mutator.date),
		})),
	]
	const differs = () =>
		parts().some((part) => part.check?.verdict === 'differs')
	return (
		<span
			class={`chip content-chip ${differs() ? 'warn' : 'ok'}`}
			tabindex='0'
			aria-describedby={tip}
		>
			{differs() ? 'Content differs' : 'Content ready'}
			<span class='content-tip' role='tooltip' id={tip}>
				<For each={parts()}>
					{(part) => (
						<span class='content-tip-row'>
							<span class='content-tip-what'>{part.what}</span>
							<span>{part.name}</span>
							<span
								class='content-tip-said'
								classList={{ differs: part.check?.verdict === 'differs' }}
							>
								{part.built ?? said(part.check)}
							</span>
						</span>
					)}
				</For>
			</span>
		</span>
	)
}

/**
 * A mutator built here from the commit its room pinned: that commit, whose
 * every file was checked against git's own hash as it was built.
 */
function builtFrom(source: string, date: string | null): string {
	const when = date ? `, committed ${exactly(date)}` : ''
	return `built from ${sourceWords(source)}${when}`
}

/** What is known of one part. An engine carries no checksum on the wire. */
function said(check: CheckView | null): string {
	if (!check) return 'installed'
	switch (check.verdict) {
		case 'checking':
			return 'checking…'
		case 'same':
			return `same files as the room · checksum ${check.hash}`
		case 'differs':
			return `different files · checksum ${check.ours}, the room's ${check.room}`
		case 'unchecked':
			return `installed, not checked: ${check.why}`
	}
}
