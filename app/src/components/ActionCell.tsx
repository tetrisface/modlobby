import type { JSX } from 'solid-js'
import { Glyph } from './icons'

/**
 * A bordered cell of one or more small actions: the look the tweak slots
 * introduced — mono, quiet until hovered, always drawn — shared by everything
 * that puts a pen or a bin beside a name.
 */
export function ActionCell(props: { filled?: boolean; children: JSX.Element }) {
  return (
    <span class='act-cell' classList={{ filled: props.filled }}>
      {props.children}
    </span>
  )
}

/**
 * One action in the cell: a glyph, then whatever the caller puts after it.
 * `title` is the tooltip and, unless `label` says more, the accessible name.
 */
export function CellButton(props: {
  icon: string
  title: string
  label?: string
  class?: string
  disabled?: boolean
  onClick: (event: MouseEvent) => void
  children?: JSX.Element
}) {
  return (
    <button
      type='button'
      class={props.class}
      title={props.title}
      aria-label={props.label ?? props.title}
      disabled={props.disabled}
      onClick={(event) => props.onClick(event)}
    >
      <Glyph id={props.icon} />
      {props.children}
    </button>
  )
}
