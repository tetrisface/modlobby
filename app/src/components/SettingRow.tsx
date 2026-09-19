import {
  createContext,
  useContext,
  type Accessor,
  type ParentProps,
} from 'solid-js'
import { hasEveryWord } from '../lib/search'

/** What the settings search holds; every row reads it to decide whether it shows. */
export const Query = createContext<Accessor<string>>(() => '')

/** The title of the section a row sits in, which a search finds it by too. */
export const Heading = createContext('')

/**
 * A setting, or a few that belong together, with what they do written out
 * beside them.
 *
 * A search finds it by everything it shows plus its section's title, read off
 * the page itself: a description can hold markup and is never written twice.
 * Only the query is watched, so the text is read afresh on each keystroke and
 * never on its own.
 */
export function Row(props: ParentProps<{ class?: string }>) {
  const query = useContext(Query)
  const heading = useContext(Heading)
  let row: HTMLDivElement | undefined
  return (
    <div
      ref={row}
      class={props.class ? `set-row ${props.class}` : 'set-row'}
      hidden={!hasEveryWord(`${heading} ${row?.textContent ?? ''}`, query())}
    >
      {props.children}
    </div>
  )
}
