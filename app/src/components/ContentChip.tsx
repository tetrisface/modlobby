import { For, createUniqueId } from 'solid-js'
import type { CheckView } from '../ipc/bindings/CheckView'
import type { ContentCheckView } from '../ipc/bindings/ContentCheckView'

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
		{ what: 'Engine', name: props.engine, check: null },
		{ what: 'Game', name: props.game, check: props.check.game },
		{ what: 'Map', name: props.map, check: props.check.map },
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
								{said(part.check)}
							</span>
						</span>
					)}
				</For>
			</span>
		</span>
	)
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
