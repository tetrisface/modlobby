import { createSignal } from 'solid-js'

/**
 * Press and drag a player onto a team; release without moving to open the menu.
 * The mods list reorders with the same press and the same lifted copy.
 *
 * Pointer events rather than the platform's drag-and-drop, which is what the
 * tab strip uses and what this codebase otherwise prefers. The difference is
 * that there the browser's own threshold is exactly right, and here one press
 * has to resolve into *either* a move *or* a menu -- which means owning the
 * threshold rather than letting the browser decide when a drag began.
 * `MapEditor` reads the pointer directly for its own reasons too.
 */

/** How far the pointer travels before a press is a drag and not a click. */
const THRESHOLD = 5

/** Whether a row is in flight, so the teams can show that they take drops. */
export const [dragging, setDragging] = createSignal(false)

/** The team box under the pointer, by the mark the room puts on each. */
function allyUnder(x: number, y: number): number | null {
	const at = document.elementFromPoint(x, y)?.closest('[data-ally]')
	const value = at?.getAttribute('data-ally')
	if (value === null || value === undefined) return null
	const ally = Number(value)
	return Number.isFinite(ally) ? ally : null
}

/**
 * The row in flight: a copy under the pointer, held where it was grabbed.
 * The row itself stays put, dimmed, so nothing in the lists moves until the
 * drop is real -- a cancelled drag then has nothing to jump back from.
 */
function lift(row: HTMLElement, at: PointerEvent) {
	const rect = row.getBoundingClientRect()
	const ghost = row.cloneNode(true) as HTMLElement
	ghost.classList.add('drag-ghost')
	ghost.style.width = `${rect.width}px`
	ghost.style.height = `${rect.height}px`
	const dx = at.clientX - rect.left
	const dy = at.clientY - rect.top
	document.body.append(ghost)
	row.classList.add('lifted')
	return {
		follow(to: PointerEvent) {
			ghost.style.transform = `translate(${to.clientX - dx}px, ${to.clientY - dy}px)`
		},
		drop() {
			ghost.remove()
			row.classList.remove('lifted')
		},
	}
}

/**
 * Press and drag a row into another place in its list, the other rows making
 * room as it goes; let go to drop it there. The roster's feel: a copy under
 * the pointer and the row itself, dimmed, where it would land.
 *
 * Where it lands is read against the rows as they stood when it lifted --
 * past the middle of a row is past that row -- so the list making room under
 * the pointer never changes the answer, and nothing jitters. The list's
 * children are its rows.
 */
export function reorderGesture(options: {
	/** Where the pressed row is in the list. */
	from: () => number
	/** Where it came from and would land now; `null` once dropped or let go. */
	onOver: (flight: { from: number; to: number } | null) => void
	onDrop: (from: number, to: number) => void
}): (event: PointerEvent) => void {
	return (event: PointerEvent) => {
		if (event.button !== 0) return
		if ((event.target as Element | null)?.closest('button, input, select'))
			return
		event.preventDefault()
		const row = event.currentTarget as HTMLElement
		const from = options.from()
		const fromX = event.clientX
		const fromY = event.clientY
		let flight: ReturnType<typeof lift> | undefined
		let middles: number[] = []
		let to = from

		const move = (at: PointerEvent) => {
			if (!flight) {
				if (Math.hypot(at.clientX - fromX, at.clientY - fromY) < THRESHOLD)
					return
				middles = [...(row.parentElement?.children ?? [])].map((child) => {
					const rect = child.getBoundingClientRect()
					return rect.top + rect.height / 2
				})
				flight = lift(row, at)
			}
			flight.follow(at)
			const next = middles.filter(
				(middle, index) => index !== from && middle < at.clientY,
			).length
			if (next === to) return
			to = next
			options.onOver({ from, to })
		}

		const end = (dropped: boolean) => () => {
			window.removeEventListener('pointermove', move)
			window.removeEventListener('pointerup', up)
			window.removeEventListener('pointercancel', cancel)
			if (!flight) return
			flight.drop()
			options.onOver(null)
			if (dropped && to !== from) options.onDrop(from, to)
		}
		const up = end(true)
		const cancel = end(false)

		window.addEventListener('pointermove', move)
		window.addEventListener('pointerup', up)
		window.addEventListener('pointercancel', cancel)
	}
}

export function rowGesture(options: {
	/** Whether this row may be dragged; a row that may not still opens a menu. */
	canMove: () => boolean
	onMove: (allyTeam: number) => void
	onMenu: (event: MouseEvent) => void
	/**
	 * A press released where it began, given the press: true if the row took
	 * it as an action of its own, in which case no menu opens. A press that
	 * moves is a drag whatever it began on.
	 */
	onTap?: (down: PointerEvent) => boolean
}): (event: PointerEvent) => void {
	return (event: PointerEvent) => {
		if (event.button !== 0) return
		// A press on a control in the row -- copy, options, remove -- is that
		// control's, not a press on the row.
		if ((event.target as Element | null)?.closest('button, input, select'))
			return
		// The rows are `user-select: none`, which stops their own text being
		// selected but not a selection anchored at them: Chromium falls back to
		// the nearest selectable position, so a drag across the roster paints
		// the chat blue. Cancelling the press is what never starts one -- and
		// it takes `mousedown` with it, which is why `dismiss` listens for the
		// pointer instead.
		event.preventDefault()
		const row = event.currentTarget as HTMLElement
		const fromX = event.clientX
		const fromY = event.clientY
		let flight: ReturnType<typeof lift> | undefined

		const move = (at: PointerEvent) => {
			if (flight) {
				flight.follow(at)
				return
			}
			if (!options.canMove()) return
			if (Math.hypot(at.clientX - fromX, at.clientY - fromY) < THRESHOLD) return
			flight = lift(row, at)
			flight.follow(at)
			setDragging(true)
		}

		const up = (at: PointerEvent) => {
			window.removeEventListener('pointermove', move)
			window.removeEventListener('pointerup', up)
			window.removeEventListener('pointercancel', up)
			if (!flight) {
				if (!options.onTap?.(event)) options.onMenu(at)
				return
			}
			flight.drop()
			setDragging(false)
			// Dropped on nothing -- the chat, the map, outside the window -- is a
			// cancelled drag rather than a move to somewhere unnamed.
			const ally = allyUnder(at.clientX, at.clientY)
			if (ally !== null) options.onMove(ally)
		}

		window.addEventListener('pointermove', move)
		window.addEventListener('pointerup', up)
		window.addEventListener('pointercancel', up)
	}
}
