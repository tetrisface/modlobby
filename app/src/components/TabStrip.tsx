import { For, Show, createSignal } from 'solid-js'
import { move } from '../lib/reorder'

export type Tab = {
  /** What identifies this tab to the caller. */
  key: string
  label: string
  /** A count to show, when there is unread work here. */
  badge?: number
  /** Whether that badge should shout. */
  urgent?: boolean
  /** Absent means this tab cannot be closed. */
  closable?: boolean
  title?: string
}

/**
 * The open conversations, as tabs you can rearrange.
 *
 * VS Code's bargain: what is open is a row you can reorder by dragging and
 * close individually, and the order is yours rather than the order things
 * happened to arrive in. Which matters here for the same reason it does there
 * — the room you are actually watching should be where you left it, not
 * wherever the alphabet or the join order puts it.
 *
 * Uses the platform's own drag events rather than a library: this is one row
 * of buttons, and pointer-level drag handling is a great deal of code to own
 * for something the browser already does, including the keyboard and
 * accessibility behaviour that comes with it.
 */
export function TabStrip(props: {
  tabs: Tab[]
  active: string
  onSelect: (key: string) => void
  onClose?: (key: string) => void
  /** Called with the new order when a drag finishes somewhere new. */
  onReorder?: (keys: string[]) => void
}) {
  const [dragging, setDragging] = createSignal<number | null>(null)
  const [over, setOver] = createSignal<number | null>(null)

  function drop(to: number) {
    const from = dragging()
    setDragging(null)
    setOver(null)
    if (from === null || from === to) return
    props.onReorder?.(move(props.tabs, from, to).map((tab) => tab.key))
  }

  return (
    <div class='tab-strip' role='tablist'>
      <For each={props.tabs}>
        {(tab, index) => (
          <div
            class='tab'
            classList={{
              on: tab.key === props.active,
              dragging: dragging() === index(),
              over: over() === index() && dragging() !== index(),
            }}
            draggable={props.onReorder !== undefined}
            onDragStart={(event) => {
              setDragging(index())
              event.dataTransfer?.setData('text/plain', tab.key)
              if (event.dataTransfer) event.dataTransfer.effectAllowed = 'move'
            }}
            onDragOver={(event) => {
              // Without this the drop is refused and the tab springs back.
              event.preventDefault()
              setOver(index())
            }}
            onDragLeave={() => setOver((at) => (at === index() ? null : at))}
            onDrop={(event) => {
              event.preventDefault()
              drop(index())
            }}
            onDragEnd={() => {
              setDragging(null)
              setOver(null)
            }}
          >
            <button
              type='button'
              role='tab'
              aria-selected={tab.key === props.active}
              title={tab.title ?? tab.label}
              onClick={() => props.onSelect(tab.key)}
            >
              <span class='tab-label'>{tab.label}</span>
              <Show when={tab.badge}>
                <span class='badge' classList={{ named: tab.urgent }}>
                  {tab.badge}
                </span>
              </Show>
            </button>
            <Show when={tab.closable && props.onClose}>
              <button
                type='button'
                class='tab-close'
                title={`Close ${tab.label}`}
                aria-label={`Close ${tab.label}`}
                onClick={() => props.onClose?.(tab.key)}
              >
                ×
              </button>
            </Show>
          </div>
        )}
      </For>
    </div>
  )
}
