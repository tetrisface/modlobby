import { For, Show, createEffect, createMemo, createSignal } from 'solid-js'
import {
	complete,
	preview,
	recall,
	remember,
	suggestions,
	type Piece,
	type Vocabulary,
} from '../lib/compose'

/**
 * The box you type a line into.
 *
 * Shared by the channels and the battle room because the two behaviours worth
 * having — walking back through what you sent, and finishing a word with Tab —
 * are worth having in both, and are the sort of thing that quietly diverges
 * when written twice.
 *
 * The history is per-box and per-session: a lobby is not a shell, and nobody
 * expects yesterday's `!bSet` back.
 *
 * It is a textarea rather than an input so a pasted block keeps its line
 * breaks: the runtime sends one message per line, and an input would have
 * glued a whole preset into one line the server refuses. Enter sends;
 * Shift+Enter is a line break.
 */
export function Composer(props: {
	placeholder: string
	/** What Tab may finish. Read on each press, never cached. */
	vocabulary: () => Vocabulary
	onSend: (text: string) => void
}) {
	const [text, setText] = createSignal('')
	const [caret, setCaret] = createSignal(0)
	const [history, setHistory] = createSignal<string[]>([])
	const [at, setAt] = createSignal(-1)
	let input: HTMLTextAreaElement | undefined
	let mirror: HTMLDivElement | undefined
	/** What was half-typed when the walk back started. */
	let draft = ''
	/**
	 * A run of Tab presses on one word. Kept whole so each press can rebuild
	 * from the original word rather than from the one the last press inserted.
	 */
	let cycle: {
		base: string
		baseCaret: number
		index: number
		produced: string
		producedCaret: number
	} | null = null

	/**
	 * What the first Tab would add, drawn faintly over the box. Only at the end
	 * of the line: drawn mid-line it would sit on top of what follows.
	 */
	const shadow = createMemo(() => {
		const value = text()
		if (caret() !== value.length) return null
		const first = suggestions(value, value.length, props.vocabulary())[0]
		return first ? preview(value, value.length, first) : null
	})

	// The ghost is only drawn with the caret at the end, where the textarea is
	// scrolled to its bottom, so past eight lines the copy is pinned there too.
	createEffect(() => {
		shadow()
		if (mirror) mirror.scrollTop = mirror.scrollHeight
	})

	/** Solid owns the value, so the caret has to be placed after it lands. */
	function put(next: string, caret: number) {
		setText(next)
		setCaret(caret)
		queueMicrotask(() => input?.setSelectionRange(caret, caret))
	}

	function onTab(event: KeyboardEvent) {
		event.preventDefault()
		const element = input
		if (!element) return
		const caret = element.selectionStart ?? element.value.length

		const again =
			cycle !== null &&
			element.value === cycle.produced &&
			caret === cycle.producedCaret
		const base = again ? cycle!.base : element.value
		const baseCaret = again ? cycle!.baseCaret : caret

		const options = suggestions(base, baseCaret, props.vocabulary())
		if (options.length === 0) return
		const index = again ? cycle!.index + 1 : 0
		const filled = complete(base, baseCaret, options[index % options.length]!)

		cycle = {
			base,
			baseCaret,
			index,
			produced: filled.text,
			producedCaret: filled.caret,
		}
		put(filled.text, filled.caret)
	}

	function walk(step: -1 | 1, event: KeyboardEvent) {
		if (history().length === 0) return
		event.preventDefault()
		if (at() === -1) draft = text()
		const found = recall(history(), at(), step, draft)
		setAt(found.at)
		put(found.text, found.text.length)
	}

	function onEnter(event: KeyboardEvent) {
		if (event.shiftKey) return
		submit(event)
	}

	function submit(event: Event) {
		event.preventDefault()
		const line = text()
		if (!line.trim()) return
		setHistory(remember(history(), line))
		setAt(-1)
		draft = ''
		cycle = null
		put('', 0)
		props.onSend(line)
	}

	/** Where the caret went, for the ghost: keys and clicks move it too. */
	const track = (event: { currentTarget: HTMLTextAreaElement }) =>
		setCaret(event.currentTarget.selectionStart)

	return (
		<form class='chat-input' onSubmit={submit}>
			<div class='composer-field' classList={{ merged: shadow()?.merged }}>
				<textarea
					ref={input}
					rows={1}
					value={text()}
					placeholder={props.placeholder}
					onInput={(event) => {
						setText(event.currentTarget.value)
						track(event)
						// Typing ends both the walk back and the run of completions.
						setAt(-1)
						cycle = null
					}}
					onKeyUp={track}
					onMouseUp={track}
					onKeyDown={(event) => {
						if (event.key === 'Enter') return onEnter(event)
						if (event.key === 'Tab' && !event.shiftKey) return onTab(event)
						if (event.key === 'ArrowUp') return walk(1, event)
						if (event.key === 'ArrowDown') return walk(-1, event)
					}}
				/>
				{/* The line again, so the ghost lands where the caret is. Merged, the
				    copy is what shows and the caret is drawn in it; otherwise the
				    typed part is see-through over the textarea's own. */}
				<Show when={shadow()}>
					{(picture) => (
						<div class='composer-ghost' ref={mirror} aria-hidden='true'>
							<Pieces of={picture().head} />
							<span class='composer-caret' />
							<Pieces of={picture().tail} />
						</div>
					)}
				</Show>
			</div>
			<button type='submit'>Send</button>
		</form>
	)
}

function Pieces(props: { of: Piece[] }) {
	return (
		<For each={props.of}>
			{(piece) => (
				<span classList={{ 'composer-typed': !piece.ghost }}>{piece.text}</span>
			)}
		</For>
	)
}
