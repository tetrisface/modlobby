import { Index } from 'solid-js'

export type Segment<T extends string> = {
	value: T
	label: string
	title?: string
	/** Classes of its own, where a segment says something about itself. */
	classList?: Record<string, boolean>
}

/**
 * One of a few, side by side with a hairline between them: the app's enum
 * toggle. Exactly one is on. Greyed as a whole when the choice does not apply
 * where it is shown, with `title` saying why.
 *
 * Drawn by position, so a label that changes -- a count in it -- keeps its
 * button, and the focus on it.
 */
export function Segmented<T extends string>(props: {
	/** What is being chosen, for a screen reader. */
	label: string
	value: T
	options: readonly Segment<T>[]
	onChange: (value: T) => void
	disabled?: boolean
	title?: string
}) {
	return (
		<div
			class='choice'
			role='group'
			aria-label={props.label}
			title={props.title}
		>
			<Index each={props.options}>
				{(option) => (
					<button
						type='button'
						classList={{
							...option().classList,
							on: option().value === props.value,
						}}
						aria-pressed={option().value === props.value}
						disabled={props.disabled}
						title={option().title}
						onClick={() => props.onChange(option().value)}
					>
						{option().label}
					</button>
				)}
			</Index>
		</div>
	)
}
