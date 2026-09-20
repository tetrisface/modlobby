import { onCleanup } from 'solid-js'

/**
 * Closes something floating — a menu, a popover — on a press outside `root`
 * or on Escape, until the owner that called this is cleaned up. Call it from
 * the component, or from an effect that runs only while the thing is open.
 *
 * On the document rather than stopped at the menu: Solid delegates
 * `onMouseDown` to the document too, and stopping propagation there does not
 * reach a listener on the same node.
 */
export function dismiss(root: () => Node | undefined, close: () => void) {
	const onDown = (event: MouseEvent) => {
		if (root()?.contains(event.target as Node)) return
		close()
	}
	const onKey = (event: KeyboardEvent) => {
		if (event.key === 'Escape') close()
	}
	document.addEventListener('mousedown', onDown)
	document.addEventListener('keydown', onKey)
	onCleanup(() => {
		document.removeEventListener('mousedown', onDown)
		document.removeEventListener('keydown', onKey)
	})
}
