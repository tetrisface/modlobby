import { For } from 'solid-js'
import type { Sections } from '../ipc/bindings/Sections'

const CHOICES = [
	['map', 'Map'],
	['modoptions', 'Settings'],
	['battle', 'Room'],
	['startBoxes', 'Start boxes'],
	['bots', 'AIs'],
] as const

/** A part this room cannot take, and why. */
export type Unavailable = Partial<Record<keyof Sections, string>>

/**
 * Which parts of a setup a Load sends, as a row of chips, and the one that
 * decides whether two loads stack. A part in `unavailable` is greyed, its
 * reason the tooltip.
 */
export function SectionChips(props: {
	sections: Sections
	onChange: (next: Sections) => void
	unavailable?: Unavailable
}) {
	const flip = (key: keyof Sections) =>
		props.onChange({ ...props.sections, [key]: !props.sections[key] })
	const reason = (key: keyof Sections) => props.unavailable?.[key]
	const resetHint = () =>
		props.sections.reset
			? 'Sends !preset first, so the room is reset to this preset alone'
			: 'Leaves what the room already has, so presets stack'

	return (
		<div class='preset-sections'>
			<span class='setup-bar-label'>Load</span>
			<For each={CHOICES}>
				{([key, label]) => (
					<button
						class='chip-choice'
						classList={{ on: props.sections[key] && !reason(key) }}
						disabled={reason(key) !== undefined}
						title={reason(key)}
						onClick={() => flip(key)}
					>
						{label}
					</button>
				)}
			</For>
			<button
				class='chip-choice'
				classList={{ on: props.sections.reset && !reason('reset') }}
				disabled={reason('reset') !== undefined}
				title={reason('reset') ?? resetHint()}
				onClick={() => flip('reset')}
			>
				Reset lobby
			</button>
		</div>
	)
}

/** What a Load sends: the chosen parts this room can take. */
export function sendable(
	sections: Sections,
	unavailable: Unavailable,
): Sections {
	const off = Object.fromEntries(
		Object.keys(unavailable).map((key) => [key, false]),
	)
	return { ...sections, ...off }
}
