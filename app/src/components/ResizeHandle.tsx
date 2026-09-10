import { onCleanup } from 'solid-js'

/**
 * A grip you drag to resize the pane it sits on.
 *
 * The component owns only the pointer choreography. What the width was when
 * the drag began, and what to do with the pointer's position, are the
 * caller's (see `lib/resize.ts` for the arithmetic) -- so this is one grip
 * for any edge, and the pane's own rules about size stay with the pane.
 *
 * Listens on the window rather than capturing the pointer: the grip is six
 * pixels wide and the pointer leaves it on the first frame of every drag.
 */
export function ResizeHandle(props: {
  /** Called as the drag begins; returns the size to reckon from. */
  onStart: () => number
  /** The start size, where the pointer began on the axis, and where it is. */
  onMove: (start: number, from: number, to: number) => void
  onEnd?: () => void
  label?: string
  /** `x` for a grip on a side edge (the default), `y` for one on the bottom. */
  axis?: 'x' | 'y'
}) {
  let start = 0
  let from = 0
  let dragging = false
  const along = (event: PointerEvent) =>
    props.axis === 'y' ? event.clientY : event.clientX
  const resizing = () => (props.axis === 'y' ? 'resizing-y' : 'resizing')

  function move(event: PointerEvent) {
    props.onMove(start, from, along(event))
  }

  function end() {
    if (!dragging) return
    dragging = false
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', end)
    window.removeEventListener('pointercancel', end)
    document.body.classList.remove(resizing())
    props.onEnd?.()
  }

  function begin(event: PointerEvent) {
    if (event.button !== 0) return
    event.preventDefault()
    start = props.onStart()
    from = along(event)
    dragging = true
    document.body.classList.add(resizing())
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', end)
    window.addEventListener('pointercancel', end)
  }

  /** The keyboard's drag: one arrow press is a sixteen pixel move. */
  function nudge(event: KeyboardEvent) {
    const keys: Record<string, number> =
      props.axis === 'y'
        ? { ArrowUp: -16, ArrowDown: 16 }
        : { ArrowLeft: -16, ArrowRight: 16 }
    const delta = keys[event.key]
    if (delta === undefined) return
    event.preventDefault()
    props.onMove(props.onStart(), 0, delta)
    props.onEnd?.()
  }

  onCleanup(end)

  return (
    <div
      class={props.axis === 'y' ? 'grip grip-y' : 'grip'}
      role='separator'
      // A grip on a side edge is a vertical bar, and the other way round.
      aria-orientation={props.axis === 'y' ? 'horizontal' : 'vertical'}
      aria-label={props.label ?? 'Resize'}
      tabIndex={0}
      onPointerDown={begin}
      onKeyDown={nudge}
    />
  )
}
