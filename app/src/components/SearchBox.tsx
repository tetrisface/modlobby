import { Show } from 'solid-js'

/**
 * A search field with a clear button; Escape clears it too. Clearing puts the
 * caret back in the field, so the next search can be typed straight away.
 */
export function SearchBox(props: {
	value: string
	onInput: (value: string) => void
	placeholder: string
	/** Where it sits, on top of how it looks. */
	class?: string
}) {
	let input: HTMLInputElement | undefined
	return (
		<div class={`search-box ${props.class ?? ''}`}>
			<input
				ref={input}
				placeholder={props.placeholder}
				aria-label={props.placeholder}
				value={props.value}
				onInput={(event) => props.onInput(event.currentTarget.value)}
				onKeyDown={(event) => {
					if (event.key === 'Escape') props.onInput('')
				}}
			/>
			<Show when={props.value.trim() !== ''}>
				<button
					type='button'
					class='clear'
					title='Clear search'
					aria-label='Clear search'
					onClick={() => {
						props.onInput('')
						input?.focus()
					}}
				>
					×
				</button>
			</Show>
		</div>
	)
}
