import { For, Show } from 'solid-js'
import type { DocId, Filter, Item, Sort } from '../../lib/tweakspace'

const SORTS: { key: Sort; label: string }[] = [
  { key: 'order', label: 'Slot order' },
  { key: 'name', label: 'Name' },
  { key: 'kind', label: 'Kind' },
]

/**
 * The documents on the left: the room's slots or the drafts on disk, searched
 * and sorted, each saying what it is called, how big it is and whether it
 * holds something the room does not.
 */
export function DocList(props: {
  items: Item[]
  active: DocId
  filter: Filter
  modified: number
  onSelect: (id: DocId) => void
  onFilter: (patch: Partial<Filter>) => void
}) {
  return (
    <aside class='doc-list'>
      <div class='doc-segments' role='tablist'>
        <button
          role='tab'
          class='chip-choice'
          classList={{ on: props.filter.segment === 'slots' }}
          aria-selected={props.filter.segment === 'slots'}
          onClick={() => props.onFilter({ segment: 'slots' })}
        >
          Slots
        </button>
        <button
          role='tab'
          class='chip-choice'
          classList={{ on: props.filter.segment === 'drafts' }}
          aria-selected={props.filter.segment === 'drafts'}
          onClick={() => props.onFilter({ segment: 'drafts' })}
        >
          Drafts
        </button>
      </div>

      <div class='doc-find'>
        <input
          placeholder='Find a slot or a name'
          aria-label='Find'
          value={props.filter.query}
          onInput={(event) =>
            props.onFilter({ query: event.currentTarget.value })
          }
        />
        <select
          aria-label='Sort'
          value={props.filter.sort}
          onChange={(event) =>
            props.onFilter({ sort: event.currentTarget.value as Sort })
          }
        >
          <For each={SORTS}>
            {(sort) => <option value={sort.key}>{sort.label}</option>}
          </For>
        </select>
      </div>

      <div class='doc-rows'>
        <For
          each={props.items}
          fallback={
            <p class='muted setup-empty'>
              {props.filter.segment === 'drafts'
                ? 'No drafts yet. Save one from the editor.'
                : 'Nothing matches.'}
            </p>
          }
        >
          {(item) => (
            <button
              class='doc'
              classList={{
                on: item.id === props.active,
                dirty: item.dirty,
                stale: item.stale,
                empty: item.empty && !item.dirty,
              }}
              title={item.name ?? item.title}
              onClick={() => props.onSelect(item.id)}
            >
              <span class='doc-line'>
                <span class='doc-kind'>{item.kind}</span>
                <span class='doc-title'>{item.title}</span>
                <span class='doc-size'>
                  {item.empty && !item.dirty
                    ? '—'
                    : `${item.size} ${item.unit === 'blob' ? 'B' : 'lua'}`}
                </span>
              </span>
              <span class='doc-line'>
                <span class='doc-name'>{item.name ?? ''}</span>
                <Show when={item.stale}>
                  <span class='doc-tag'>room moved</span>
                </Show>
                <Show when={item.dirty}>
                  <span class='doc-tag dirty'>edited</span>
                </Show>
              </span>
            </button>
          )}
        </For>
      </div>

      <Show when={props.modified > 0}>
        <div class='doc-foot'>
          {props.modified} {props.modified === 1 ? 'document' : 'documents'}{' '}
          with unsent edits
        </div>
      </Show>
    </aside>
  )
}
