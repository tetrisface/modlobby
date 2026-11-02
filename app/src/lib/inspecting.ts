import { createSignal, onCleanup } from 'solid-js'

/** How long the pointer rests on a row before it is the one being read. */
export const REST_MS = 150

/**
 * Which row the strip under a list is describing. Hover previews, a press pins.
 *
 * The rest keeps a pointer crossing the list, on its way down to the strip's
 * own controls, from flicking it through every row it passes. A pin is what
 * the strip goes back to once the pointer has gone; with none it keeps the
 * last row read, so looking away does not empty it.
 */
export function createInspecting() {
	const [rested, setRested] = createSignal<string | null>(null)
	const [pinned, setPinned] = createSignal<string | null>(null)

	let timer: ReturnType<typeof setTimeout> | undefined
	/** Whichever row is still waiting out its rest goes unread. */
	function stop() {
		clearTimeout(timer)
		timer = undefined
	}
	onCleanup(stop)

	function pin(key: string) {
		stop()
		setRested(null)
		setPinned(key)
	}

	return {
		/** The row to describe, if any has been picked yet. */
		key: () => rested() ?? pinned(),
		pinned,
		/** The pointer is over this row, or over no row at all. */
		point(key: string | null) {
			stop()
			if (key === null || key === rested()) return
			timer = setTimeout(() => setRested(key), REST_MS)
		},
		stop,
		/** The pointer has gone from the list and the strip alike. */
		leave() {
			stop()
			if (pinned() !== null) setRested(null)
		},
		pin,
		/**
		 * A press on the row itself: pinned, or let go if it already was. Let go,
		 * it stays the row being read: the pointer that pressed it is still there.
		 */
		toggle(key: string) {
			if (pinned() !== key) return pin(key)
			setPinned(null)
			setRested(key)
		},
	}
}
