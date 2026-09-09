import { splitProps, type JSX } from 'solid-js'

/**
 * The one way a choice is drawn.
 *
 * Every dropdown in the app is this, so they all look the same and change
 * together; `Select.test.tsx` refuses a bare `<select>` anywhere else. The
 * children are the `<option>`s, as in HTML, and every other attribute goes
 * straight through.
 */
export function Select(props: JSX.SelectHTMLAttributes<HTMLSelectElement>) {
  const [own, rest] = splitProps(props, ['class'])
  return (
    <select class={own.class ? `select ${own.class}` : 'select'} {...rest} />
  )
}
