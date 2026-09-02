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
  /** Called as the drag begins; returns the width to reckon from. */
  onStart: () => number
  onMove: (startWidth: number, startX: number, x: number) => void
  onEnd?: () => void
  label?: string
}) {
  let startWidth = 0
  let startX = 0
  let dragging = false

  function move(event: PointerEvent) {
    props.onMove(startWidth, startX, event.clientX)
  }

  function end() {
    if (!dragging) return
    dragging = false
    window.removeEventListener('pointermove', move)
    window.removeEventListener('pointerup', end)
    window.removeEventListener('pointercancel', end)
    document.body.classList.remove('resizing')
    props.onEnd?.()
  }

  function begin(event: PointerEvent) {
    if (event.button !== 0) return
    event.preventDefault()
    startWidth = props.onStart()
    startX = event.clientX
    dragging = true
    document.body.classList.add('resizing')
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', end)
    window.addEventListener('pointercancel', end)
  }

  /** The keyboard's drag: one arrow press is a sixteen pixel move. */
  function nudge(event: KeyboardEvent) {
    const dx = { ArrowLeft: -16, ArrowRight: 16 }[event.key]
    if (dx === undefined) return
    event.preventDefault()
    props.onMove(props.onStart(), 0, dx)
    props.onEnd?.()
  }

  onCleanup(end)

  return (
    <div
      class='grip'
      role='separator'
      aria-orientation='vertical'
      aria-label={props.label ?? 'Resize'}
      tabIndex={0}
      onPointerDown={begin}
      onKeyDown={nudge}
    />
  )
}
