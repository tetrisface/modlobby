import {
	For,
	Show,
	createEffect,
	createSignal,
	on,
	onCleanup,
	onMount,
} from 'solid-js'
import { Linkify } from '../../components/Linkify'

/**
 * What the room says to whoever joins it, on the room's card.
 *
 * teiserver's Coordinator sends it as private messages on every join. The
 * runtime reads them into the room, and the Coordinator's conversation keeps
 * them without counting them as unread, so this is the one place it is told.
 * Three lines show; a longer welcome opens in place.
 */
export function Welcome(props: { lines: readonly string[] }) {
	const [open, setOpen] = createSignal(false)
	const [clipped, setClipped] = createSignal(false)
	let text: HTMLDivElement | undefined

	// Measured rather than counted: how many lines a paragraph takes is the
	// card's width to decide.
	const measure = () => {
		if (!text || open()) return
		setClipped(text.scrollHeight > text.clientHeight + 1)
	}
	createEffect(on(() => props.lines.join('\n'), measure))
	onMount(() => {
		if (!text || typeof ResizeObserver === 'undefined') return
		const watching = new ResizeObserver(measure)
		watching.observe(text)
		onCleanup(() => watching.disconnect())
	})

	return (
		<aside class='room-welcome' aria-label='Welcome from the Coordinator'>
			<span class='room-welcome-head'>
				Welcome <span>@Coordinator</span>
			</span>
			<div
				class='room-welcome-text'
				classList={{ open: open() }}
				ref={(el) => {
					text = el
				}}
			>
				<For each={props.lines}>
					{(line) => (
						<p>
							<Linkify text={line} />
						</p>
					)}
				</For>
			</div>
			<Show when={clipped() || open()}>
				<button
					type='button'
					class='room-welcome-more'
					aria-expanded={open()}
					onClick={() => setOpen((was) => !was)}
				>
					{open() ? 'less' : 'more'}
				</button>
			</Show>
		</aside>
	)
}
