import { Show, createSignal, onMount } from 'solid-js'

/**
 * Asking for one line of text without stopping the lobby.
 *
 * Never `window.prompt`: it blocks the whole page until it is answered — chat,
 * the battle list, every timer — and WebKitGTK, the webview everywhere that is
 * not Windows, refuses it outright and hands back `null`.
 */
export function Ask(props: {
	title: string
	hint?: string
	initial?: string
	confirm?: string
	/** Why the text as typed will not do, or `null` when it will. */
	problem?: (text: string) => string | null
	onAnswer: (text: string) => void
	onCancel: () => void
}) {
	const [text, setText] = createSignal(props.initial ?? '')
	const problem = () => props.problem?.(text()) ?? null
	let field: HTMLInputElement | undefined

	onMount(() => {
		field?.focus()
		field?.select()
	})

	return (
		<div class='sheet' onMouseDown={props.onCancel}>
			<form
				class='sheet-card'
				onMouseDown={(event) => event.stopPropagation()}
				onSubmit={(event) => {
					event.preventDefault()
					const answer = text().trim()
					if (answer && !problem()) props.onAnswer(answer)
				}}
			>
				<h2>{props.title}</h2>
				<Show when={props.hint}>{(hint) => <p class='muted'>{hint()}</p>}</Show>
				<input
					ref={field}
					value={text()}
					onInput={(event) => setText(event.currentTarget.value)}
					onKeyDown={(event) => {
						if (event.key === 'Escape') props.onCancel()
					}}
				/>
				<Show when={text().trim() && problem()}>
					{(why) => <p class='error'>{why()}</p>}
				</Show>
				<div class='sheet-actions'>
					<button type='button' onClick={props.onCancel}>
						Cancel
					</button>
					<button
						class='primary'
						type='submit'
						disabled={!text().trim() || problem() !== null}
					>
						{props.confirm ?? 'OK'}
					</button>
				</div>
			</form>
		</div>
	)
}
