import {
	For,
	createEffect,
	createMemo,
	createSignal,
	onCleanup,
} from 'solid-js'
import {
	GRIPS,
	INSETS,
	type Grip,
	dragInsets,
	fitInsets,
	readInsets,
	writeInsets,
} from '../lib/card'
import { localStore } from '../lib/resize'

/** Where the card's edges are remembered. */
const KEY = 'modlobby.overlayCard'

const viewport = () => ({
	width: window.innerWidth,
	height: window.innerHeight,
})

/**
 * The lobby card's edges over a game, to drag as a window's are.
 *
 * The window covers the game's monitor and keeps still, its own edges off:
 * what a person sees as the window is the card, so the card's are the ones
 * that move. They are published as `--card-*` on the page, which the card
 * and these grips both draw from, and are kept for the next game.
 *
 * Inside `.shell` in the markup, fixed to the viewport so the card's clipping
 * does not cut them off: a press on one is a press in the lobby, never a
 * click on the game behind it (`clickLeavesOverlay`). A double click on one,
 * or on the title bar, puts the card back as it was at first.
 */
export function CardGrips() {
	const [saved, setSaved] = createSignal(
		readInsets(localStore(), KEY) ?? INSETS,
	)
	const [view, setView] = createSignal(viewport())
	const shown = createMemo(() => fitInsets(saved(), view()))

	const measure = () => setView(viewport())
	window.addEventListener('resize', measure)

	const page = document.documentElement.style
	createEffect(() => {
		for (const [side, inset] of Object.entries(shown()))
			page.setProperty(`--card-${side}`, `${inset}px`)
	})

	/** Ends a drag in progress; nothing while there is none. */
	let stop = () => {}
	onCleanup(() => {
		stop()
		window.removeEventListener('resize', measure)
		for (const side of Object.keys(INSETS))
			page.removeProperty(`--card-${side}`)
	})

	function begin(event: PointerEvent, grip: Grip) {
		if (event.button !== 0) return
		event.preventDefault()
		const start = shown()
		const [x0, y0] = [event.clientX, event.clientY]
		const move = (at: PointerEvent) =>
			setSaved(
				dragInsets(start, grip, at.clientX - x0, at.clientY - y0, view()),
			)
		const end = () => {
			window.removeEventListener('pointermove', move)
			window.removeEventListener('pointerup', end)
			window.removeEventListener('pointercancel', end)
			document.body.classList.remove('resizing-card')
			document.body.style.removeProperty('cursor')
			writeInsets(localStore(), KEY, saved())
			stop = () => {}
		}
		stop = end
		// The grip's cursor everywhere the pointer goes until the release.
		document.body.style.cursor = getComputedStyle(
			event.currentTarget as Element,
		).cursor
		document.body.classList.add('resizing-card')
		window.addEventListener('pointermove', move)
		window.addEventListener('pointerup', end)
		window.addEventListener('pointercancel', end)
	}

	function reset() {
		setSaved(INSETS)
		writeInsets(localStore(), KEY, null)
	}

	/**
	 * A double click where a window's would maximize -- straight on a drag
	 * region, the nav's own empty space -- since the overlay cannot be
	 * maximized. Caught on its way down: Tauri's drag script stops that press
	 * at the document, before any handler delegated there.
	 */
	const titleBar = (event: MouseEvent) => {
		const target = event.target
		if (event.button !== 0 || event.detail !== 2) return
		if (!(target instanceof Element)) return
		// Bare or `true`, as Tauri reads it: the element itself, not inside it.
		const region = target.getAttribute('data-tauri-drag-region')
		if (region === '' || region === 'true') reset()
	}
	window.addEventListener('mousedown', titleBar, true)
	onCleanup(() => window.removeEventListener('mousedown', titleBar, true))

	return (
		<For each={GRIPS}>
			{(grip) => (
				<div
					class={`card-grip ${grip}`}
					aria-hidden='true'
					onPointerDown={(event) => begin(event, grip)}
					onDblClick={reset}
				/>
			)}
		</For>
	)
}
